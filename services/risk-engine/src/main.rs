//! Vigil Risk Engine Microservice Entrypoint
//!
//! Consumes indexed EVM events, maintains borrower positions in Postgres,
//! continuously evaluates health factors with exact Solidity math parity,
//! and emits liquidation signals via a transactional outbox.

use anyhow::{Context, Result};
use async_nats::jetstream;
use risk_engine::api::HttpServer;
use risk_engine::config::Config;
use risk_engine::consumer::EventConsumer;
use risk_engine::db::init_db;
use risk_engine::engine::health_gate::HealthGate;
use risk_engine::publisher::OutboxWorker;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,risk_engine=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Vigil Risk Engine microservice...");

    let config = Config::from_env();
    info!(
        database_url = %config.database_url,
        nats_url = %config.nats_url,
        http_port = config.http_port,
        max_lag_seconds = config.max_lag_seconds,
        debt_asset = %config.canonical_debt_asset,
        "Configuration loaded successfully"
    );

    // Initialize Postgres connection pool
    let pool = init_db(&config.database_url)
        .await
        .context("Failed to connect to Postgres")?;
    info!("Connected to Postgres database");

    // Initialize Fail-Closed Health Gate
    let health_gate = HealthGate::new(config.max_lag_seconds);

    // Spawn HTTP Health and Metrics Server
    let http_server = HttpServer::new(config.http_port, Arc::clone(&health_gate));
    tokio::spawn(async move {
        if let Err(e) = http_server.run().await {
            error!(error = %e, "HTTP health server crashed");
        }
    });

    // Initialize NATS JetStream
    let nats_client = async_nats::connect(&config.nats_url)
        .await
        .context("Failed to connect to NATS")?;
    let js_context = jetstream::new(nats_client);

    // Ensure VIGIL_EVENTS and VIGIL_SIGNALS streams exist
    let _ = js_context
        .get_or_create_stream(jetstream::stream::Config {
            name: "VIGIL_EVENTS".to_string(),
            subjects: vec!["vigil.events.>".to_string()],
            ..Default::default()
        })
        .await;

    let _ = js_context
        .get_or_create_stream(jetstream::stream::Config {
            name: "VIGIL_SIGNALS".to_string(),
            subjects: vec!["vigil.signals.>".to_string()],
            ..Default::default()
        })
        .await;

    // Spawn Transactional Outbox Worker
    let outbox_worker = OutboxWorker::new(
        pool.clone(),
        js_context.clone(),
        Arc::clone(&health_gate),
        Duration::from_millis(500),
    );
    tokio::spawn(async move {
        outbox_worker.run_loop().await;
    });

    // Spawn Event Consumer
    let consumer = EventConsumer::new(
        pool.clone(),
        js_context,
        config.clone(),
        Arc::clone(&health_gate),
    );
    tokio::spawn(async move {
        if let Err(e) = consumer.run_subscription_loop().await {
            error!(error = %e, "JetStream consumer encountered fatal error");
        }
    });

    info!("Risk Engine fully operational: awaiting chain mutations and price feeds");

    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received. Shutting down Risk Engine gracefully.");

    Ok(())
}
