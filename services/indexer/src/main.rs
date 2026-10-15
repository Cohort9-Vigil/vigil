//! Vigil Indexer Microservice
//! Subscribes to EVM logs from Anvil/RPC, indexes borrower position updates,
//! handles chain reorgs, persists crash-safe cursors in PostgreSQL,
//! exposes Prometheus metrics & health checks, and streams events to NATS JetStream.

use std::time::Duration;
use alloy::providers::Provider;
use anyhow::Result;
use vigil_indexer::cursor::PostgresCursorStore;
use vigil_indexer::listener::BlockIngestor;
use vigil_indexer::publisher::NatsJetStreamPublisher;
use vigil_indexer::telemetry::{start_health_and_metrics_server, Metrics};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,indexer=debug".into()))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Vigil Indexer microservice (Japheth - Infra, Bus & Persistence)...");

    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8545".into());
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/vigil".into());
    let metrics_port: u16 = std::env::var("METRICS_PORT")
        .or_else(|_| std::env::var("HEALTH_PORT"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    let poll_interval_ms: u64 = std::env::var("POLL_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    info!(
        rpc_url = %rpc_url,
        nats_url = %nats_url,
        database_url = %database_url,
        metrics_port = metrics_port,
        poll_interval_ms = poll_interval_ms,
        "Configuration loaded"
    );

    // 1. Initialize Prometheus metrics and spawn HTTP health/metrics server (RS-04)
    let metrics = Metrics::new();
    let _health_server_handle = start_health_and_metrics_server(metrics_port, metrics.clone()).await?;

    // 2. Initialize PostgreSQL Cursor Store (RS-03)
    let cursor_store = match PostgresCursorStore::connect(&database_url).await {
        Ok(store) => {
            info!("Connected to PostgreSQL and verified cursor schema");
            store
        }
        Err(e) => {
            error!(error = %e, "Failed to connect to PostgreSQL; exiting");
            return Err(e);
        }
    };

    // 3. Initialize NATS JetStream Publisher (RS-01, RS-04)
    let publisher = match NatsJetStreamPublisher::connect(&nats_url).await {
        Ok(publ) => publ,
        Err(e) => {
            error!(error = %e, "Failed to connect to NATS JetStream; exiting");
            return Err(e);
        }
    };

    // 4. Initialize BlockIngestor with 10-block sliding window, Postgres persistence, and metrics
    let mut ingestor = BlockIngestor::connect_http(&rpc_url, publisher, 10)?
        .with_cursor_store(cursor_store)
        .with_metrics(metrics);

    // 5. Run startup recovery routine (RS-03): recover saved cursor and recent block window
    match ingestor.recover_from_cursor_store(10).await {
        Ok(Some(last_block)) => {
            info!(last_block, "Resuming block ingestion from cursor + 1");
        }
        Ok(None) => {
            info!("No previous cursor saved. Starting block ingestion from genesis/current head");
        }
        Err(e) => {
            warn!(error = %e, "Warning during startup cursor recovery; continuing with fresh state");
        }
    }

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
