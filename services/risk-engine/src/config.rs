//! Configuration for Vigil Risk Engine.

use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub nats_url: String,
    pub http_port: u16,
    pub max_lag_seconds: u64,
    pub canonical_debt_asset: String,
    pub default_ltv_wad: u128,
    pub default_liq_threshold_wad: u128,
}

impl Config {
    pub fn from_env() -> Self {
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/vigil".into());
        let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
        let http_port = env::var("HTTP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        let max_lag_seconds = env::var("MAX_LAG_SECONDS")
            .ok()
            .and_then(|l| l.parse().ok())
            .unwrap_or(60);
        let canonical_debt_asset = env::var("DEBT_ASSET")
            .unwrap_or_else(|_| "0x0000000000000000000000000000000000000002".into())
            .to_lowercase();

        // Default 80% LTV and 85% Liquidation Threshold in WAD (1e18)
        let default_ltv_wad = 800_000_000_000_000_000;
        let default_liq_threshold_wad = 850_000_000_000_000_000;

        Self {
            database_url,
            nats_url,
            http_port,
            max_lag_seconds,
            canonical_debt_asset,
            default_ltv_wad,
            default_liq_threshold_wad,
        }
    }
}
