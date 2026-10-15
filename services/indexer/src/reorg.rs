//! 10-Block Reorg Engine and Sliding Window State Machine (RS-02).
//!
//! Tracks canonical block hashes and emitted log references in a 10-block ring buffer.
//! When a parent-hash mismatch is detected, walks back to the common ancestor and
//! emits `RetractEvent`s in strict reverse chronological order before ingesting the new fork.

use anyhow::{anyhow, Result};
use common::events::{IndexedEvent, RetractEvent};
use std::collections::VecDeque;
use tracing::info;

pub const DEFAULT_RING_CAPACITY: usize = 10;

/// A stored block in the sliding window ring buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlock {
    pub block_number: u64,
    pub block_hash: String,
    pub parent_hash: String,
    pub emitted_events: Vec<IndexedEvent>,
}

/// The result of inspecting a new block candidate against the current ring buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum BlockEvaluation {
    /// The block is the direct descendant of the current canonical head (parent_hash matched).
    DirectDescendant,
    /// Ring buffer is empty, block can be accepted as initial head.
    InitialBlock,
    /// Already seen block with identical hash (idempotent duplicate).
    AlreadyProcessed,
    /// Chain reorganization detected! Requires ancestor walk-back.
    ReorgDetected {
        expected_parent_hash: String,
        actual_parent_hash: String,
        at_block: u64,
    },
}

/// Ring buffer storing up to 10 (block_number, block_hash) pairs plus emitted events.
#[derive(Debug, Clone)]
pub struct ReorgRingBuffer {
    capacity: usize,
    blocks: VecDeque<StoredBlock>,
}

impl Default for ReorgRingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_RING_CAPACITY)
    }
}

impl ReorgRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            blocks: VecDeque::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn latest_block(&self) -> Option<&StoredBlock> {
        self.blocks.back()
    }

    pub fn oldest_block(&self) -> Option<&StoredBlock> {
        self.blocks.front()
    }

    pub fn latest_block_number(&self) -> Option<u64> {
        self.blocks.back().map(|b| b.block_number)
    }

    pub fn get_block(&self, block_number: u64) -> Option<&StoredBlock> {
        self.blocks.iter().find(|b| b.block_number == block_number)
    }

    pub fn contains_hash(&self, hash: &str) -> Option<&StoredBlock> {
        self.blocks
            .iter()
            .find(|b| b.block_hash.eq_ignore_ascii_case(hash))
    }

    /// Evaluates a new block header against the current head of the ring buffer.
    pub fn evaluate_header(
        &self,
        block_number: u64,
        block_hash: &str,
        parent_hash: &str,
    ) -> BlockEvaluation {
        if self.blocks.is_empty() {
            return BlockEvaluation::InitialBlock;
        }

        let latest = self.latest_block().unwrap();

        // Check for duplicate / already processed head
        if latest.block_number == block_number && latest.block_hash.eq_ignore_ascii_case(block_hash)
        {
            return BlockEvaluation::AlreadyProcessed;
        }

        // Standard consecutive block
        if block_number == latest.block_number + 1 {
            if latest.block_hash.eq_ignore_ascii_case(parent_hash) {
                return BlockEvaluation::DirectDescendant;
            } else {
                return BlockEvaluation::ReorgDetected {
                    expected_parent_hash: latest.block_hash.clone(),
                    actual_parent_hash: parent_hash.to_string(),
                    at_block: block_number,
                };
            }
        }

        // If block number is less than or equal to latest, but different hash -> reorg
        if let Some(existing) = self.get_block(block_number) {
            if existing.block_hash.eq_ignore_ascii_case(block_hash) {
                return BlockEvaluation::AlreadyProcessed;
            } else {
                return BlockEvaluation::ReorgDetected {
                    expected_parent_hash: existing.parent_hash.clone(),
                    actual_parent_hash: parent_hash.to_string(),
                    at_block: block_number,
                };
            }
        }

        // Block jump (ahead by more than 1)
        BlockEvaluation::DirectDescendant
    }

    /// Inserts a newly verified canonical block and its emitted events into the ring buffer.
    /// Automatically evicts the oldest block when capacity (10) is exceeded.
    pub fn push_block(
        &mut self,
        block_number: u64,
        block_hash: String,
        parent_hash: String,
        emitted_events: Vec<IndexedEvent>,
    ) {
        if self.blocks.len() >= self.capacity {
            self.blocks.pop_front();
        }

        self.blocks.push_back(StoredBlock {
            block_number,
            block_hash,
            parent_hash,
            emitted_events,
        });
    }

    /// Reconciles a reorganization given a common ancestor block number.
    ///
    /// 1. Finds all orphaned blocks strictly greater than `ancestor_block_number`.
    /// 2. Generates `RetractEvent`s for all logs in STRICT REVERSE CHRONOLOGICAL ORDER
    ///    (highest block to lowest block, and highest log index to lowest log index).
    /// 3. Removes the orphaned blocks from the ring buffer.
    /// 4. Returns the reverse-ordered `RetractEvent`s to be published to NATS.
    pub fn rollback_to_ancestor(
        &mut self,
        ancestor_block_number: u64,
        retract_timestamp: u64,
    ) -> Result<Vec<RetractEvent>> {
        // Collect blocks that are strictly > ancestor_block_number
        let mut orphaned_blocks: Vec<StoredBlock> = Vec::new();

        while let Some(back) = self.blocks.back() {
            if back.block_number > ancestor_block_number {
                orphaned_blocks.push(self.blocks.pop_back().unwrap());
            } else {
                break;
            }
        }

        if orphaned_blocks.is_empty() {
            return Ok(Vec::new());
        }

        info!(
            ancestor = ancestor_block_number,
            orphaned_count = orphaned_blocks.len(),
            "Rolling back reorged blocks"
        );

        // Generate Retract events in strict reverse order:
        // orphaned_blocks is already in reverse order (highest block_number down to ancestor + 1)
        let mut retractions = Vec::new();

        for orphan in orphaned_blocks {
            // Reverse the emitted events within this block (highest log_index down to lowest)
            for evt in orphan.emitted_events.into_iter().rev() {
                let retract_trace_id = format!("{}-retract", evt.trace_id);
                let retract = RetractEvent::new(
                    retract_trace_id,
                    orphan.block_number,
                    orphan.block_hash.clone(),
                    evt.tx_hash.clone(),
                    evt.log_index,
                    Some(evt.event),
                    retract_timestamp,
                );
                retractions.push(retract);
            }
        }

        Ok(retractions)
    }

    /// Performs the walk-back algorithm using an asynchronous or synchronous parent fetcher.
    ///
    /// Given a reorged block at `new_block_number` with `new_parent_hash`,
    /// walks backwards by fetching parent blocks until a block matching a stored hash is found.
    pub async fn walk_back_to_ancestor<F, Fut>(
        &mut self,
        mut current_parent_hash: String,
        mut current_height: u64,
        fetch_parent_of: F,
        retract_timestamp: u64,
    ) -> Result<(u64, Vec<RetractEvent>)>
    where
        F: Fn(String, u64) -> Fut,
        Fut: std::future::Future<Output = Result<(String, String)>>, // returns (block_hash, parent_hash)
    {
        let oldest_num = self.oldest_block().map(|b| b.block_number).unwrap_or(0);

        // Loop backward until current_parent_hash matches a stored block in our ring
        while current_height >= oldest_num {
            // Check if our ring buffer has a stored block whose hash matches current_parent_hash
            if let Some(stored) = self.contains_hash(&current_parent_hash) {
                let ancestor_num = stored.block_number;
                info!(
                    ancestor_num = ancestor_num,
                    ancestor_hash = %stored.block_hash,
                    "Found common ancestor in ring buffer"
                );
                let retractions = self.rollback_to_ancestor(ancestor_num, retract_timestamp)?;
                return Ok((ancestor_num, retractions));
            }

            if current_height == 0 || current_height <= oldest_num {
                break;
            }

            // Fetch the parent block on the new branch
            current_height -= 1;
            let (_fetched_hash, fetched_parent) =
                fetch_parent_of(current_parent_hash.clone(), current_height).await?;
            current_parent_hash = fetched_parent;
        }

        Err(anyhow!(
            "Reorganization depth exceeded 10-block sliding window (oldest stored block: {})",
            oldest_num
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::events::ChainEvent;
    use common::types::Address;

    fn mock_event(block_num: u64, block_hash: &str, log_idx: u64) -> IndexedEvent {
        IndexedEvent::new(
            format!("trace-{block_num}-{log_idx}"),
            block_num,
            block_hash,
            format!("0xtx-{block_num}"),
            log_idx,
            ChainEvent::Deposit {
                account: Address::new("0xUser"),
                asset: Address::new("0xAsset"),
                amount: 100 * log_idx as u128,
                block_number: block_num,
                tx_hash: format!("0xtx-{block_num}"),
            },
            1700000000 + block_num,
        )
    }

    #[test]
    fn test_ring_buffer_fifo_eviction() {
        let mut ring = ReorgRingBuffer::new(3);

        ring.push_block(1, "0xhash1".into(), "0xhash0".into(), vec![]);
        ring.push_block(2, "0xhash2".into(), "0xhash1".into(), vec![]);
        ring.push_block(3, "0xhash3".into(), "0xhash2".into(), vec![]);

        assert_eq!(ring.len(), 3);
        assert_eq!(ring.oldest_block().unwrap().block_number, 1);
        assert_eq!(ring.latest_block().unwrap().block_number, 3);

        // Push 4th -> block 1 evicted
        ring.push_block(4, "0xhash4".into(), "0xhash3".into(), vec![]);
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.oldest_block().unwrap().block_number, 2);
        assert_eq!(ring.latest_block().unwrap().block_number, 4);
    }

    #[test]
    fn test_parent_hash_verification() {
        let mut ring = ReorgRingBuffer::new(10);
        ring.push_block(10, "0xhash10".into(), "0xhash9".into(), vec![]);

        // Direct descendant
        let eval1 = ring.evaluate_header(11, "0xhash11", "0xhash10");
        assert_eq!(eval1, BlockEvaluation::DirectDescendant);

        // Parent hash mismatch (reorg!)
        let eval2 = ring.evaluate_header(11, "0xfork11", "0xwrongparent");
        assert_eq!(
            eval2,
            BlockEvaluation::ReorgDetected {
                expected_parent_hash: "0xhash10".into(),
                actual_parent_hash: "0xwrongparent".into(),
                at_block: 11,
            }
        );
    }

    #[test]
    fn test_strict_reverse_retraction_order() {
        let mut ring = ReorgRingBuffer::new(10);

        // Store block 10 (common ancestor)
        ring.push_block(
            10,
            "0xhash10".into(),
            "0xhash9".into(),
            vec![mock_event(10, "0xhash10", 0)],
        );

        // Store block 11 (orphaned later) with 2 events: log 0, log 1
        ring.push_block(
            11,
            "0xhash11".into(),
            "0xhash10".into(),
            vec![mock_event(11, "0xhash11", 0), mock_event(11, "0xhash11", 1)],
        );

        // Store block 12 (orphaned later) with 2 events: log 0, log 1
        ring.push_block(
            12,
            "0xhash12".into(),
            "0xhash11".into(),
            vec![mock_event(12, "0xhash12", 0), mock_event(12, "0xhash12", 1)],
        );

        assert_eq!(ring.len(), 3);

        // Reorg back to block 10!
        let retractions = ring.rollback_to_ancestor(10, 1700000099).unwrap();

        // Must have exactly 4 retractions
        assert_eq!(retractions.len(), 4);

        // Strict reverse order assertion:
        // 1. Block 12, log 1
        // 2. Block 12, log 0
        // 3. Block 11, log 1
        // 4. Block 11, log 0
        assert_eq!(retractions[0].block_number, 12);
        assert_eq!(retractions[0].log_index, 1);

        assert_eq!(retractions[1].block_number, 12);
        assert_eq!(retractions[1].log_index, 0);

        assert_eq!(retractions[2].block_number, 11);
        assert_eq!(retractions[2].log_index, 1);

        assert_eq!(retractions[3].block_number, 11);
        assert_eq!(retractions[3].log_index, 0);

        // Ring buffer should now only have block 10
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.latest_block().unwrap().block_number, 10);
    }

    #[tokio::test]
    async fn test_walk_back_to_ancestor() {
        let mut ring = ReorgRingBuffer::new(10);
        ring.push_block(8, "0xancestor8".into(), "0x7".into(), vec![]);
        ring.push_block(
            9,
            "0xforkA_9".into(),
            "0xancestor8".into(),
            vec![mock_event(9, "0xforkA_9", 0)],
        );
        ring.push_block(
            10,
            "0xforkA_10".into(),
            "0xforkA_9".into(),
            vec![mock_event(10, "0xforkA_10", 0)],
        );

        // New fork B arrives at block 11, with parent "0xforkB_10"
        // Fork B:
        // Block 10 has parent "0xforkB_9"
        // Block 9 has parent "0xancestor8" (matches stored block 8!)
        let fetcher = |_parent_hash: String, height: u64| async move {
            if height == 10 {
                Ok(("0xforkB_10".into(), "0xforkB_9".into()))
            } else if height == 9 {
                Ok(("0xforkB_9".into(), "0xancestor8".into()))
            } else {
                panic!("Unexpected height {}", height);
            }
        };

        let (ancestor, retractions) = ring
            .walk_back_to_ancestor("0xforkB_10".into(), 10, fetcher, 1700000000)
            .await
            .expect("walk back success");

        assert_eq!(ancestor, 8);
        assert_eq!(retractions.len(), 2);
        assert_eq!(retractions[0].block_number, 10);
        assert_eq!(retractions[1].block_number, 9);
        assert_eq!(ring.latest_block().unwrap().block_number, 8);
    }
}
