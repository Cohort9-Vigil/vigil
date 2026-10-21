//! Block listener, gap recovery, and hash-based log ingestion (RS-01, RS-03, RS-04).
//!
//! Enforces:
//! 1. Never trust WebSocket newHeads blindly: treat as hints and actively backfill gaps.
//! 2. Query logs strictly by block hash (eth_getLogs with blockHash), never block number.
//! 3. Publish to NATS before updating cursor state.

use alloy::providers::Provider;
use alloy::rpc::types::eth::{BlockNumberOrTag, Filter};
use anyhow::{anyhow, Result};
use common::events::{IndexedEvent, RetractEvent};
use tracing::{debug, info, warn};

use crate::abi::decode_log;
use crate::cursor::{CursorStore, MockCursorStore};
use crate::publisher::EventPublisher;
use crate::reorg::{BlockEvaluation, ReorgRingBuffer};
use crate::telemetry::Metrics;

/// Ingestion engine coordinating block listening, reorg protection, log decoding,
/// at-least-once cursor persistence, and telemetry metrics.
pub struct BlockIngestor<P, E, C = MockCursorStore> {
    pub provider: P,
    pub publisher: E,
    pub cursor_store: C,
    pub ring: ReorgRingBuffer,
    pub last_processed_block: Option<u64>,
    pub metrics: Metrics,
    pub target_addresses: Vec<alloy::primitives::Address>,
}

impl<E: EventPublisher> BlockIngestor<(), E, MockCursorStore> {
    pub fn connect_http(
        rpc_url: &str,
        publisher: E,
        ring_capacity: usize,
    ) -> Result<BlockIngestor<impl Provider + Clone + 'static, E, MockCursorStore>> {
        let url: url::Url = rpc_url.parse().map_err(|e| anyhow!("Invalid RPC URL: {e}"))?;
        let provider = alloy::providers::ProviderBuilder::new().connect_http(url);
        Ok(BlockIngestor::new(provider, publisher, ring_capacity))
    }
}

impl<P: Provider + Clone + 'static, E: EventPublisher> BlockIngestor<P, E, MockCursorStore> {
    pub fn new(provider: P, publisher: E, ring_capacity: usize) -> Self {
        Self {
            provider,
            publisher,
            cursor_store: MockCursorStore::new(),
            ring: ReorgRingBuffer::new(ring_capacity),
            last_processed_block: None,
            metrics: Metrics::new(),
            target_addresses: Vec::new(),
        }
    }
}

impl<P: Provider + Clone + 'static, E: EventPublisher, C: CursorStore> BlockIngestor<P, E, C> {
    /// Swaps in a custom cursor persistence store (e.g. PostgresCursorStore).
    pub fn with_cursor_store<C2: CursorStore>(self, cursor_store: C2) -> BlockIngestor<P, E, C2> {
        BlockIngestor {
            provider: self.provider,
            publisher: self.publisher,
            cursor_store,
            ring: self.ring,
            last_processed_block: self.last_processed_block,
            metrics: self.metrics,
            target_addresses: self.target_addresses,
        }
    }

    /// Attaches shared metrics.
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Configures an optional target contract address to filter logs (e.g. VigilPool).
    pub fn with_target_address(mut self, target_address: Option<alloy::primitives::Address>) -> Self {
        if let Some(addr) = target_address {
            if !self.target_addresses.contains(&addr) {
                self.target_addresses.push(addr);
            }
        }
        self
    }

    /// Configures multiple target contract addresses to filter logs (e.g. VigilPool and LiquidationSettlement).
    pub fn with_target_addresses(mut self, target_addresses: Vec<alloy::primitives::Address>) -> Self {
        self.target_addresses = target_addresses;
        self
    }

    /// Sets an initial starting cursor (e.g. loaded from Postgres on startup).
    pub fn with_initial_cursor(mut self, last_block: u64) -> Self {
        self.last_processed_block = Some(last_block);
        self.metrics.set_last_block(last_block);
        self
    }

    /// Recovers state from the cursor store on boot (RS-03):
    /// Loads the latest saved cursor and recent block window into the reorg ring buffer.
    pub async fn recover_from_cursor_store(&mut self, window_capacity: usize) -> Result<Option<u64>> {
        if let Some((last_num, last_hash)) = self.cursor_store.get_last_cursor().await? {
            info!(last_num, %last_hash, "Recovered indexer cursor from persistence store");
            self.last_processed_block = Some(last_num);
            self.metrics.set_last_block(last_num);

            let recent = self.cursor_store.load_recent_window(window_capacity).await?;
            for rec in recent {
                self.ring.push_block(rec.block_number, rec.block_hash, rec.parent_hash, Vec::new());
            }
            Ok(Some(last_num))
        } else {
            info!("No previous cursor found in persistence store; starting fresh");
            Ok(None)
        }
    }

    /// Ingests a new head block number as a hint (Rule 1).
    /// If there is a gap between `last_processed_block` and `hint_number`,
    /// sequentially fetches and processes every missing block.
    pub async fn ingest_hint(&mut self, hint_number: u64) -> Result<(Vec<RetractEvent>, Vec<IndexedEvent>)> {
        let mut all_retractions = Vec::new();
        let mut all_indexed = Vec::new();

        let start_block = match self.last_processed_block {
            Some(last) if hint_number > last => last + 1,
            Some(last) => hint_number.min(last),
            None => 1,
        };

        for block_num in start_block..=hint_number {
            let (retracts, indexed) = self.process_block(block_num).await?;
            all_retractions.extend(retracts);
            all_indexed.extend(indexed);
        }

        Ok((all_retractions, all_indexed))
    }

    /// Processes a single block number with parent-hash verification,
    /// reorg ancestor walk-back, and hash-filtered log ingestion.
    pub async fn process_block(
        &mut self,
        block_number: u64,
    ) -> Result<(Vec<RetractEvent>, Vec<IndexedEvent>)> {
        // 1. Fetch block header by number from RPC
        let block = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(block_number))
            .await
            .map_err(|e| anyhow!("RPC error fetching block {block_number}: {e}"))?
            .ok_or_else(|| anyhow!("Block {block_number} not found on RPC"))?;

        let block_hash = format!("{:?}", block.header.hash);
        let parent_hash = format!("{:?}", block.header.parent_hash);
        let block_timestamp = block.header.timestamp;

        let mut retractions_emitted = Vec::new();

        // 2. Evaluate block header against the 10-block sliding window
        match self.ring.evaluate_header(block_number, &block_hash, &parent_hash) {
            BlockEvaluation::DirectDescendant | BlockEvaluation::InitialBlock => {
                debug!(block_number, %block_hash, "Linear canonical block");
            }
            BlockEvaluation::AlreadyProcessed => {
                debug!(block_number, %block_hash, "Block already processed, skipping");
                return Ok((Vec::new(), Vec::new()));
            }
            BlockEvaluation::ReorgDetected {
                expected_parent_hash,
                actual_parent_hash,
                at_block,
            } => {
                warn!(
                    at_block,
                    expected_parent = %expected_parent_hash,
                    actual_parent = %actual_parent_hash,
                    "Chain reorganization detected! Initiating ancestor walk-back"
                );

                let provider_clone = self.provider.clone();
                let fetch_parent = move |_parent_str: String, height: u64| {
                    let p = provider_clone.clone();
                    async move {
                        let b = p
                            .get_block_by_number(BlockNumberOrTag::Number(height))
                            .await
                            .map_err(|e| anyhow!("Failed to fetch ancestor block {height}: {e}"))?
                            .ok_or_else(|| anyhow!("Ancestor block {height} not found"))?;
                        Ok((format!("{:?}", b.header.hash), format!("{:?}", b.header.parent_hash)))
                    }
                };

                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Walk back until a block hash in the new branch matches a stored hash
                let (ancestor_num, retractions) = self
                    .ring
                    .walk_back_to_ancestor(
                        parent_hash.clone(),
                        block_number - 1,
                        fetch_parent,
                        now_ts,
                    )
                    .await?;

                info!(
                    ancestor = ancestor_num,
                    retraction_count = retractions.len(),
                    "Ancestor reached. Publishing Retract events in reverse order"
                );

                // Publish Retract events in STRICT REVERSE ORDER before publishing any new logs (Rule 3)
                for retract in &retractions {
                    self.publisher.publish_retract_event(retract).await?;
                }

                // Update telemetry metrics for reorg
                self.metrics.inc_reorgs();
                self.metrics.inc_retractions(retractions.len() as u64);

                // Mark reorged blocks in cursor store
                self.cursor_store.mark_reorged(ancestor_num + 1, block_number - 1).await?;

                retractions_emitted = retractions;
                let mut intermediate_events = Vec::new();
                // Re-ingest the intermediate canonical blocks from (ancestor + 1) up to (block_number - 1)
                for intermediate in (ancestor_num + 1)..block_number {
                    let (ret, idx) = Box::pin(self.process_block(intermediate)).await?;
                    retractions_emitted.extend(ret);
                    intermediate_events.extend(idx);
                }

                // Append intermediate events to forward event output
                let (hash_retracts, mut current_events) = self.fetch_and_publish_logs(&block, block_number, &block_hash, &parent_hash, block_timestamp).await?;
                retractions_emitted.extend(hash_retracts);
                intermediate_events.append(&mut current_events);
                return Ok((retractions_emitted, intermediate_events));
            }
        }

        // 3. Fetch canonical block logs STRICTLY by block hash (Rule 2)
        let (retracts, emitted) = self.fetch_and_publish_logs(&block, block_number, &block_hash, &parent_hash, block_timestamp).await?;
        retractions_emitted.extend(retracts);
        Ok((retractions_emitted, emitted))
    }

    async fn fetch_and_publish_logs(
        &mut self,
        block: &alloy::rpc::types::eth::Block,
        block_number: u64,
        block_hash: &str,
        parent_hash: &str,
        block_timestamp: u64,
    ) -> Result<(Vec<RetractEvent>, Vec<IndexedEvent>)> {
        // Query logs strictly by block hash (Rule 2)
        let b256_hash = block.header.hash;
        let mut filter = Filter::new().at_block_hash(b256_hash);
        if !self.target_addresses.is_empty() {
            filter = filter.address(self.target_addresses.clone());
        }
        let logs = self
            .provider
            .get_logs(&filter)
            .await
            .map_err(|e| anyhow!("Failed to fetch logs for blockHash {block_hash}: {e}"))?;

        // Rule 4: Preserve log order within a block.
        // Risk Engine rules depend on (blockNumber, logIndex) order.
        let mut logs = logs;
        logs.sort_by_key(|l| l.log_index.unwrap_or(0));

        let mut emitted_events = Vec::new();
        for log in logs {
            let tx_hash = log
                .transaction_hash
                .map(|h| format!("{h:?}"))
                .unwrap_or_else(|| "0x0".into());
            let log_index = log.log_index.unwrap_or(0);

            if let Some(chain_event) = decode_log(&log, block_number, &tx_hash)? {
                let trace_id = format!(
                    "trace-b{block_number}-l{log_index}-{}",
                    uuid::Uuid::new_v4().simple()
                );
                let indexed_event = IndexedEvent::new(
                    trace_id,
                    block_number,
                    block_hash,
                    &tx_hash,
                    log_index,
                    chain_event,
                    block_timestamp,
                );
                emitted_events.push(indexed_event);
            }
        }

        if !emitted_events.is_empty() {
            info!(
                block_number,
                block_hash = %block_hash,
                event_count = emitted_events.len(),
                events = ?emitted_events.iter().map(|e| e.event.event_type()).collect::<Vec<_>>(),
                "Successfully indexed protocol events"
            );
        }

        // 1. Publish to NATS JetStream and await ack (Rule 3)
        for evt in &emitted_events {
            self.publisher.publish_indexed_event(evt).await?;
        }

        // 2. Commit events and cursor to Postgres ONLY AFTER NATS ack (Rule 3 & RS-03)
        self.cursor_store.save_events(&emitted_events).await?;
        self.cursor_store.commit_block(block_number, block_hash, parent_hash, block_timestamp).await?;

        // 3. Record in 10-block sliding window
        self.ring.push_block(
            block_number,
            block_hash.to_string(),
            parent_hash.to_string(),
            emitted_events.clone(),
        );

        // 4. Update in-memory cursor and telemetry metrics
        self.last_processed_block = Some(block_number);
        self.metrics.set_last_block(block_number);
        self.metrics.inc_events(emitted_events.len() as u64);
        for evt in &emitted_events {
            self.metrics.inc_event_by_type(evt.event.event_type());
        }

        Ok((Vec::new(), emitted_events))
    }
}
