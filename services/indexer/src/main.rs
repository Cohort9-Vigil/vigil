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

    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8545".into()).trim().to_string();
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into()).trim().to_string();
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/vigil".into()).trim().to_string();
    let metrics_port: u16 = std::env::var("PORT")
        .or_else(|_| std::env::var("METRICS_PORT"))
        .or_else(|_| std::env::var("HEALTH_PORT"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    let poll_interval_ms: u64 = std::env::var("POLL_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    // Load target contract addresses and deploy block from env or deployments JSON
    let deployments_path = std::env::var("DEPLOYMENTS_FILE").ok();
    let (file_pool, file_settlement, file_deploy_block) = if let Some(path) = deployments_path {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let p = json.get("pool").and_then(|v| v.as_str()).map(|s| s.to_string());
                let s = json.get("settlement").and_then(|v| v.as_str()).map(|s| s.to_string());
                let b = json.get("deployBlock").and_then(|v| v.as_u64());
                (p, s, b)
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    let pool_address_raw = std::env::var("POOL_ADDRESS")
        .ok()
        .or(file_pool)
        .unwrap_or_else(|| "0xecF5622D592293c960bFec2A948dF9e8B1323358".into())
        .trim()
        .to_string();

    let settlement_address_raw = std::env::var("SETTLEMENT_ADDRESS")
        .ok()
        .or(file_settlement)
        .unwrap_or_else(|| "0x488eB39f40D0E808dC0e5533697A18b3c0f3787B".into())
        .trim()
        .to_string();

    let mut target_addresses: Vec<alloy::primitives::Address> = Vec::new();
    match pool_address_raw.parse() {
        Ok(addr) => target_addresses.push(addr),
        Err(e) => warn!(error = %e, raw = %pool_address_raw, "Invalid POOL_ADDRESS"),
    }
    match settlement_address_raw.parse() {
        Ok(addr) => target_addresses.push(addr),
        Err(e) => warn!(error = %e, raw = %settlement_address_raw, "Invalid SETTLEMENT_ADDRESS"),
    }

    let start_block_env: Option<u64> = std::env::var("START_BLOCK")
        .ok()
        .and_then(|v| v.parse().ok())
        .or(file_deploy_block);

    info!(
        rpc_url = %rpc_url,
        nats_url = %nats_url,
        database_url = %database_url,
        pool_address = %pool_address_raw,
        settlement_address = %settlement_address_raw,
        target_addresses = ?target_addresses,
        start_block = ?start_block_env,
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
        .with_metrics(metrics)
        .with_target_addresses(target_addresses);

    // 5. Run startup recovery routine (RS-03): recover saved cursor and recent block window
    match ingestor.recover_from_cursor_store(10).await {
        Ok(Some(last_block)) => {
            if let Some(sb) = start_block_env {
                if sb > last_block {
                    info!(last_block, start_block = sb, "Fast-forwarding cursor to configured START_BLOCK");
                    ingestor = ingestor.with_initial_cursor(sb.saturating_sub(1));
                } else {
                    info!(last_block, "Resuming block ingestion from cursor + 1");
                }
            } else {
                info!(last_block, "Resuming block ingestion from cursor + 1");
            }
        }
        Ok(None) => {
            if let Some(sb) = start_block_env {
                info!(start_block = sb, "No previous cursor saved. Starting block ingestion from configured START_BLOCK");
                ingestor = ingestor.with_initial_cursor(sb.saturating_sub(1));
            } else {
                match ingestor.provider.get_block_number().await {
                    Ok(current_head) => {
                        let resume_from = current_head.saturating_sub(20);
                        info!(current_head, resume_from, "No previous cursor saved. Resuming from recent block window");
                        ingestor = ingestor.with_initial_cursor(resume_from);
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to get current head block; starting fresh from 1");
                    }
                }
            }
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
