//! Vigil Indexer Microservice
//! Subscribes to EVM logs from Anvil/RPC, indexes borrower position updates,
//! handles chain reorgs, and streams events to NATS JetStream.

use alloy::providers::Provider;
use anyhow::Result;
use indexer::listener::BlockIngestor;
use indexer::publisher::NatsJetStreamPublisher;
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,indexer=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Vigil Indexer microservice...");

    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8545".into());
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/vigil".into());
    let poll_interval_ms: u64 = std::env::var("POLL_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    info!(
        rpc_url = %rpc_url,
        nats_url = %nats_url,
        database_url = %database_url,
        poll_interval_ms = poll_interval_ms,
        "Configuration loaded"
    );

    // Initialize NATS JetStream Publisher
    let publisher = match NatsJetStreamPublisher::connect(&nats_url).await {
        Ok(publ) => publ,
        Err(e) => {
            error!(error = %e, "Failed to connect to NATS JetStream; exiting");
            return Err(e);
        }
    };

    // Initialize BlockIngestor with 10-block sliding window
    let mut ingestor = BlockIngestor::connect_http(&rpc_url, publisher, 10)?;
    info!("BlockIngestor initialized with 10-block reorg sliding window");

    let mut poll_interval = tokio::time::interval(Duration::from_millis(poll_interval_ms));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutdown signal received. Shutting down Indexer gracefully.");
                break;
            }
            _ = poll_interval.tick() => {
                match ingestor.provider.get_block_number().await {
                    Ok(latest_head) => {
                        if let Err(e) = ingestor.ingest_hint(latest_head).await {
                            warn!(error = %e, head = latest_head, "Error during block ingestion");
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to poll block number from RPC");
                    }
                }
            }
        }
    }

    Ok(())
}
