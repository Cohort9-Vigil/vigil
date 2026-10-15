//! Vigil Liquidator Microservice
//! Consumes liquidation signals from NATS, manages EVM nonces safely (gap-free),
//! performs pre-flight simulation, and submits liquidation transactions on-chain.

use anyhow::Result;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Vigil Liquidator microservice...");

    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://localhost:8545".into());
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());

    info!(rpc_url = %rpc_url, nats_url = %nats_url, "Liquidator configuration loaded");

    // Liquidator transaction execution loop placeholder
    info!("Liquidator active: awaiting liquidation signals and readying nonce manager...");

    tokio::signal::ctrl_c().await?;
    info!("Shutting down Liquidator gracefully.");

    Ok(())
}
