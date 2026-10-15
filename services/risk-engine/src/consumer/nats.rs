//! NATS JetStream event consumer and transactional processing pipeline (RS-01, RS-04, RS-05).

use anyhow::{anyhow, Result};
use async_nats::jetstream::{self, Context as JetStreamContext, Message};
use common::events::{
    ChainEvent, IndexedEvent, RetractEvent, SUBJECT_CHAIN_EVENTS, SUBJECT_RETRACT_EVENTS,
};
use common::types::AssetConfig;
use futures::StreamExt;
use sqlx::{PgConnection, PgPool};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::db::events::{mark_retracted, record_event_if_new, AppliedDelta, EventRecordParams};
use crate::db::outbox::{
    cancel_pending_signals_for_block, enqueue_signal, supersede_pending_signals,
};
use crate::db::positions::{
    apply_borrow, apply_deposit, apply_liquidation, apply_repay, get_position,
    get_positions_by_collateral, revert_delta, update_health_factor,
};
use crate::db::prices::{get_asset_config, get_latest_oracle_price, upsert_oracle_price};
use crate::db::state::update_heartbeat;
use crate::engine::evaluator::{evaluate_position, PositionEvaluationInput};
use crate::engine::health_gate::HealthGate;

/// Consumer engine subscribing to chain events and reorg retractions.
pub struct EventConsumer {
    pool: PgPool,
    context: JetStreamContext,
    config: Config,
    health_gate: Arc<HealthGate>,
}

impl EventConsumer {
    pub fn new(
        pool: PgPool,
        context: JetStreamContext,
        config: Config,
        health_gate: Arc<HealthGate>,
    ) -> Self {
        Self {
            pool,
            context,
            config,
            health_gate,
        }
    }

    /// Connects to NATS JetStream and binds or creates the consumer.
    pub async fn connect_and_start(
        pool: PgPool,
        nats_url: &str,
        config: Config,
        health_gate: Arc<HealthGate>,
    ) -> Result<()> {
        let client = async_nats::connect(nats_url)
            .await
            .map_err(|e| anyhow!("Failed to connect to NATS at {nats_url}: {e}"))?;
        let context = jetstream::new(client);

        // Ensure the VIGIL_EVENTS stream exists
        let stream_config = jetstream::stream::Config {
            name: "VIGIL_EVENTS".to_string(),
            subjects: vec!["vigil.events.>".to_string()],
            ..Default::default()
        };
        context
            .get_or_create_stream(stream_config)
            .await
            .map_err(|e| anyhow!("Failed to ensure VIGIL_EVENTS stream: {e}"))?;

        let consumer = Self::new(pool, context, config, health_gate);
        consumer.run_subscription_loop().await
    }

    /// Background subscription loop consuming events and retractions.
    pub async fn run_subscription_loop(&self) -> Result<()> {
        // Create durable consumer for forward events
        let chain_consumer = self
            .context
            .create_consumer_on_stream(
                async_nats::jetstream::consumer::pull::Config {
                    durable_name: Some("risk-engine-chain-consumer".to_string()),
                    filter_subject: SUBJECT_CHAIN_EVENTS.to_string(),
                    ..Default::default()
                },
                "VIGIL_EVENTS",
            )
            .await
            .map_err(|e| anyhow!("Failed to create chain consumer: {e}"))?;

        // Create durable consumer for retraction events
        let retract_consumer = self
            .context
            .create_consumer_on_stream(
                async_nats::jetstream::consumer::pull::Config {
                    durable_name: Some("risk-engine-retract-consumer".to_string()),
                    filter_subject: SUBJECT_RETRACT_EVENTS.to_string(),
                    ..Default::default()
                },
                "VIGIL_EVENTS",
            )
            .await
            .map_err(|e| anyhow!("Failed to create retract consumer: {e}"))?;

        info!("Risk Engine JetStream consumers initialized: listening for events and retractions");

        let mut chain_messages = chain_consumer.messages().await?;
        let mut retract_messages = retract_consumer.messages().await?;

        loop {
            tokio::select! {
                Some(msg_res) = retract_messages.next() => {
                    match msg_res {
                        Ok(msg) => {
                            if let Err(e) = self.handle_retract_message(&msg).await {
                                error!(error = %e, "Failed to process RetractEvent");
                            } else {
                                let _ = msg.ack().await;
                            }
                        }
                        Err(e) => warn!(error = %e, "Error reading from retract stream"),
                    }
                }
                Some(msg_res) = chain_messages.next() => {
                    match msg_res {
                        Ok(msg) => {
                            if let Err(e) = self.handle_chain_message(&msg).await {
                                error!(error = %e, "Failed to process IndexedEvent");
                            } else {
                                let _ = msg.ack().await;
                            }
                        }
                        Err(e) => warn!(error = %e, "Error reading from chain stream"),
                    }
                }
            }
        }
    }

    async fn handle_chain_message(&self, msg: &Message) -> Result<()> {
        let event: IndexedEvent = serde_json::from_slice(&msg.payload)
            .map_err(|e| anyhow!("Failed to deserialize IndexedEvent: {e}"))?;

        let mut tx = self.pool.begin().await?;
        process_indexed_event_in_tx(&mut tx, &event, &self.config, &self.health_gate).await?;
        tx.commit().await?;

        Ok(())
    }

    async fn handle_retract_message(&self, msg: &Message) -> Result<()> {
        let event: RetractEvent = serde_json::from_slice(&msg.payload)
            .map_err(|e| anyhow!("Failed to deserialize RetractEvent: {e}"))?;

        let mut tx = self.pool.begin().await?;
        process_retract_event_in_tx(&mut tx, &event, &self.config, &self.health_gate).await?;
        tx.commit().await?;

        Ok(())
    }
}

// ============================================================================
// Core Transactional Processing Functions
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub struct PositionEvalContext<'a> {
    pub account: &'a str,
    pub collateral_asset: &'a str,
    pub block_number: u64,
    pub block_hash: &'a str,
    pub timestamp: u64,
}

/// Processes a single forward `IndexedEvent` in an isolated Postgres transaction.
/// 1. Idempotently records the event in `processed_events`.
/// 2. Applies the position / price mutation.
/// 3. Re-evaluates health factors.
/// 4. Writes outbox liquidation signals if liquidatable and healthy.
pub async fn process_indexed_event_in_tx(
    conn: &mut PgConnection,
    event: &IndexedEvent,
    config: &Config,
    health_gate: &HealthGate,
) -> Result<()> {
    let block_hash = &event.block_hash;
    let log_index = event.log_index;
    let block_number = event.block_number;
    let timestamp = event.timestamp;

    // Record heartbeat for fail-closed monitoring
    health_gate.record_heartbeat(timestamp);

    match &event.event {
        ChainEvent::Deposit {
            account,
            asset,
            amount,
            ..
        } => {
            let dummy_delta = AppliedDelta {
                prev_collateral: 0,
                new_collateral: *amount,
                prev_debt: 0,
                new_debt: 0,
            };

            // Deduplication check: if row already exists, skip
            let params = EventRecordParams {
                block_hash,
                log_index,
                block_number,
                event_type: "Deposit",
                account: Some(&account.0),
                asset: Some(&asset.0),
            };
            if !record_event_if_new(conn, params, &dummy_delta).await? {
                debug!(block_hash, log_index, "Duplicate Deposit event skipped");
                return Ok(());
            }

            // Apply collateral mutation
            let delta = apply_deposit(conn, &account.0, &asset.0, *amount, block_number).await?;
            // Update delta with true before/after values
            sqlx::query(
                "UPDATE processed_events SET applied_delta = $1 WHERE block_hash = $2 AND log_index = $3",
            )
            .bind(serde_json::to_value(&delta)?)
            .bind(block_hash)
            .bind(log_index as i64)
            .execute(&mut *conn)
            .await?;

            update_heartbeat(conn, block_number, block_hash).await?;

            // Re-evaluate borrower's position
            let eval_ctx = PositionEvalContext {
                account: &account.0,
                collateral_asset: &asset.0,
                block_number,
                block_hash,
                timestamp,
            };
            evaluate_and_update_position(conn, eval_ctx, config, health_gate).await?;
        }

        ChainEvent::Borrow {
            account, amount, ..
        } => {
            let collateral_asset = &config.canonical_debt_asset;
            let dummy_delta = AppliedDelta {
                prev_collateral: 0,
                new_collateral: 0,
                prev_debt: 0,
                new_debt: *amount,
            };

            let params = EventRecordParams {
                block_hash,
                log_index,
                block_number,
                event_type: "Borrow",
                account: Some(&account.0),
                asset: Some(collateral_asset),
            };
            if !record_event_if_new(conn, params, &dummy_delta).await? {
                debug!(block_hash, log_index, "Duplicate Borrow event skipped");
                return Ok(());
            }

            let delta =
                apply_borrow(conn, &account.0, collateral_asset, *amount, block_number).await?;
            sqlx::query(
                "UPDATE processed_events SET applied_delta = $1 WHERE block_hash = $2 AND log_index = $3",
            )
            .bind(serde_json::to_value(&delta)?)
            .bind(block_hash)
            .bind(log_index as i64)
            .execute(&mut *conn)
            .await?;

            update_heartbeat(conn, block_number, block_hash).await?;

            let eval_ctx = PositionEvalContext {
                account: &account.0,
                collateral_asset,
                block_number,
                block_hash,
                timestamp,
            };
            evaluate_and_update_position(conn, eval_ctx, config, health_gate).await?;
        }

        ChainEvent::Repay {
            account, amount, ..
        } => {
            let collateral_asset = &config.canonical_debt_asset;
            let dummy_delta = AppliedDelta {
                prev_collateral: 0,
                new_collateral: 0,
                prev_debt: *amount,
                new_debt: 0,
            };

            let params = EventRecordParams {
                block_hash,
                log_index,
                block_number,
                event_type: "Repay",
                account: Some(&account.0),
                asset: Some(collateral_asset),
            };
            if !record_event_if_new(conn, params, &dummy_delta).await? {
                debug!(block_hash, log_index, "Duplicate Repay event skipped");
                return Ok(());
            }

            let delta =
                apply_repay(conn, &account.0, collateral_asset, *amount, block_number).await?;
            sqlx::query(
                "UPDATE processed_events SET applied_delta = $1 WHERE block_hash = $2 AND log_index = $3",
            )
            .bind(serde_json::to_value(&delta)?)
            .bind(block_hash)
            .bind(log_index as i64)
            .execute(&mut *conn)
            .await?;

            update_heartbeat(conn, block_number, block_hash).await?;

            let eval_ctx = PositionEvalContext {
                account: &account.0,
                collateral_asset,
                block_number,
                block_hash,
                timestamp,
            };
            evaluate_and_update_position(conn, eval_ctx, config, health_gate).await?;
        }

        ChainEvent::LiquidationCall {
            collateral_asset,
            user,
            debt_to_cover,
            liquidated_collateral_amount,
            ..
        } => {
            let dummy_delta = AppliedDelta {
                prev_collateral: 0,
                new_collateral: 0,
                prev_debt: 0,
                new_debt: 0,
            };

            let params = EventRecordParams {
                block_hash,
                log_index,
                block_number,
                event_type: "LiquidationCall",
                account: Some(&user.0),
                asset: Some(&collateral_asset.0),
            };
            if !record_event_if_new(conn, params, &dummy_delta).await? {
                return Ok(());
            }

            let delta = apply_liquidation(
                conn,
                &user.0,
                &collateral_asset.0,
                *debt_to_cover,
                *liquidated_collateral_amount,
                block_number,
            )
            .await?;

            sqlx::query(
                "UPDATE processed_events SET applied_delta = $1 WHERE block_hash = $2 AND log_index = $3",
            )
            .bind(serde_json::to_value(&delta)?)
            .bind(block_hash)
            .bind(log_index as i64)
            .execute(&mut *conn)
            .await?;

            update_heartbeat(conn, block_number, block_hash).await?;

            let eval_ctx = PositionEvalContext {
                account: &user.0,
                collateral_asset: &collateral_asset.0,
                block_number,
                block_hash,
                timestamp,
            };
            evaluate_and_update_position(conn, eval_ctx, config, health_gate).await?;
        }

        ChainEvent::PriceUpdated(oracle_price) => {
            let dummy_delta = AppliedDelta {
                prev_collateral: 0,
                new_collateral: 0,
                prev_debt: 0,
                new_debt: 0,
            };

            let params = EventRecordParams {
                block_hash,
                log_index,
                block_number,
                event_type: "PriceUpdated",
                account: None,
                asset: Some(&oracle_price.asset.0),
            };
            if !record_event_if_new(conn, params, &dummy_delta).await? {
                return Ok(());
            }

            upsert_oracle_price(
                conn,
                &oracle_price.asset.0,
                oracle_price.price_wad,
                block_number,
                timestamp,
            )
            .await?;

            update_heartbeat(conn, block_number, block_hash).await?;

            // Re-evaluate all borrower positions backed by this asset
            let affected_positions =
                get_positions_by_collateral(conn, &oracle_price.asset.0).await?;
            for pos in affected_positions {
                let eval_ctx = PositionEvalContext {
                    account: &pos.account,
                    collateral_asset: &pos.collateral_asset,
                    block_number,
                    block_hash,
                    timestamp,
                };
                evaluate_and_update_position(conn, eval_ctx, config, health_gate).await?;
            }
        }
    }

    Ok(())
}

/// Processes a `RetractEvent` when an orphaned block is rolled back during a reorg (RS-05).
/// 1. Finds the original event in `processed_events`.
/// 2. Inverts the recorded delta in `positions`.
/// 3. Marks the event as `is_retracted = TRUE`.
/// 4. Cancels pending outbox signals created at that `block_hash`.
/// 5. Re-evaluates the position under the unwound state.
pub async fn process_retract_event_in_tx(
    conn: &mut PgConnection,
    retract: &RetractEvent,
    config: &Config,
    health_gate: &HealthGate,
) -> Result<()> {
    let block_hash = &retract.block_hash;
    let log_index = retract.log_index;
    let timestamp = retract.timestamp;

    // Idempotent retract check: if already retracted or non-existent, exit cleanly
    let retracted_entry = mark_retracted(conn, block_hash, log_index).await?;
    let (account_opt, asset_opt, delta) = match retracted_entry {
        Some(entry) => entry,
        None => {
            debug!(
                block_hash,
                log_index, "Retract event already processed or not found"
            );
            return Ok(());
        }
    };

    // 1. Invert the applied delta on the position
    if let (Some(account), Some(asset)) = (&account_opt, &asset_opt) {
        revert_delta(conn, account, asset, &delta).await?;
    }

    // 2. Cancel any pending liquidation signals that were computed at this orphaned block
    let cancelled_count = cancel_pending_signals_for_block(conn, block_hash).await?;
    if cancelled_count > 0 {
        info!(
            block_hash,
            cancelled_count, "Cancelled pending liquidation signals due to chain reorg"
        );
    }

    // 3. Re-evaluate the affected position under the reverted state
    if let (Some(account), Some(asset)) = (&account_opt, &asset_opt) {
        let eval_ctx = PositionEvalContext {
            account,
            collateral_asset: asset,
            block_number: retract.block_number,
            block_hash,
            timestamp,
        };
        evaluate_and_update_position(conn, eval_ctx, config, health_gate).await?;
    }

    Ok(())
}

/// Evaluates a borrower's position, updates health factor, and coordinates outbox signal emission.
async fn evaluate_and_update_position(
    conn: &mut PgConnection,
    ctx: PositionEvalContext<'_>,
    config: &Config,
    health_gate: &HealthGate,
) -> Result<()> {
    let pos_opt = get_position(conn, ctx.account, ctx.collateral_asset).await?;
    let pos = match pos_opt {
        Some(p) => p,
        None => return Ok(()),
    };

    // Fetch oracle price for collateral
    let price_wad = get_latest_oracle_price(conn, ctx.collateral_asset)
        .await?
        .unwrap_or(common::math::WAD_U128);

    // Fetch asset config
    let asset_cfg = get_asset_config(conn, ctx.collateral_asset)
        .await?
        .unwrap_or_else(|| AssetConfig {
            asset: common::types::Address::new(ctx.collateral_asset),
            symbol: "COLLATERAL".to_string(),
            decimals: 18,
            ltv_wad: config.default_ltv_wad,
            liq_threshold_wad: config.default_liq_threshold_wad,
            is_active: true,
        });

    let eval_input = PositionEvaluationInput {
        borrower: ctx.account,
        collateral_asset: ctx.collateral_asset,
        collateral_amount: pos.collateral_amount,
        collateral_decimals: asset_cfg.decimals,
        collateral_price_wad: price_wad,
        liq_threshold_wad: asset_cfg.liq_threshold_wad,
        debt_amount: pos.debt_amount,
        debt_asset: &config.canonical_debt_asset,
        block_number: ctx.block_number,
        block_hash: ctx.block_hash,
        timestamp: ctx.timestamp,
    };

    let eval_result = evaluate_position(&eval_input)?;

    // Persist latest health factor
    update_health_factor(
        conn,
        ctx.account,
        ctx.collateral_asset,
        eval_result.health_factor_wad,
    )
    .await?;

    if eval_result.is_liquidatable {
        // Only write to outbox if health gate allows signal emission (Fail-Closed)
        if let Ok(()) = health_gate.can_emit_signals(ctx.timestamp) {
            if let Some(signal) = eval_result.signal {
                enqueue_signal(conn, &signal).await?;
                debug!(
                    borrower = ctx.account,
                    hf_wad = eval_result.health_factor_wad,
                    "Enqueued liquidation signal in outbox"
                );
            }
        } else {
            warn!(
                borrower = ctx.account,
                "Position is liquidatable but signal emission suppressed by Fail-Closed health gate"
            );
        }
    } else {
        // If position is healthy, supersede any prior open signals for this position
        supersede_pending_signals(conn, ctx.account, ctx.collateral_asset).await?;
    }

    Ok(())
}
