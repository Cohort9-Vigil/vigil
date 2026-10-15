//! Transactional Outbox worker for liquidation signal publishing (RS-04, RS-05, RS-06).

use anyhow::{anyhow, Result};
use async_nats::jetstream::Context as JetStreamContext;
use async_nats::HeaderMap;
use chrono::Utc;
use common::events::SUBJECT_LIQUIDATION_SIGNALS;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::db::outbox::{fetch_pending_signals, mark_signal_sent};
use crate::engine::health_gate::HealthGate;

pub struct OutboxWorker {
    pool: PgPool,
    context: JetStreamContext,
    health_gate: Arc<HealthGate>,
    poll_interval: Duration,
}

impl OutboxWorker {
    pub fn new(
        pool: PgPool,
        context: JetStreamContext,
        health_gate: Arc<HealthGate>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            pool,
            context,
            health_gate,
            poll_interval,
        }
    }

    /// Runs the outbox publisher loop until shutdown.
    pub async fn run_loop(&self) {
        info!("Starting Risk Engine outbox worker loop...");
        let mut interval = tokio::time::interval(self.poll_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.publish_pending_batch().await {
                error!(error = %e, "Error processing outbox signals batch");
            }
        }
    }

    /// Fetches and publishes a single batch of pending liquidation signals.
    pub async fn publish_pending_batch(&self) -> Result<usize> {
        let now_secs = Utc::now().timestamp().max(0) as u64;

        // Fail-Closed check: do NOT emit signals if degraded or frozen
        if let Err(reason) = self.health_gate.can_emit_signals(now_secs) {
            warn!(
                reason = %reason,
                "Outbox worker paused: cannot emit liquidation signals under fail-closed conditions"
            );
            return Ok(0);
        }

        let pending_signals = fetch_pending_signals(&self.pool, 50).await?;
        if pending_signals.is_empty() {
            return Ok(0);
        }

        let mut published_count = 0;
        for record in pending_signals {
            let payload = serde_json::to_vec(&record.signal)
                .map_err(|e| anyhow!("Failed to serialize LiquidationSignal: {e}"))?;

            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", record.dedupe_key.as_str());
            headers.insert("trace_id", record.signal.trace_id.as_str());

            // Publish to NATS JetStream
            match self
                .context
                .publish_with_headers(SUBJECT_LIQUIDATION_SIGNALS, headers, payload.into())
                .await
            {
                Ok(ack_future) => match ack_future.await {
                    Ok(ack) => {
                        debug!(
                            seq = ack.sequence,
                            dedupe_key = %record.dedupe_key,
                            borrower = %record.position_owner,
                            "Published LiquidationSignal to NATS JetStream"
                        );
                        mark_signal_sent(&self.pool, &record.id).await?;
                        published_count += 1;
                    }
                    Err(e) => {
                        error!(error = %e, dedupe_key = %record.dedupe_key, "NATS publish ack failed");
                    }
                },
                Err(e) => {
                    error!(error = %e, dedupe_key = %record.dedupe_key, "NATS publish failed");
                }
            }
        }

        Ok(published_count)
    }
}
