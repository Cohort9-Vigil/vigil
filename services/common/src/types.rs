//! Common types and domain models for Vigil protocol.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address(pub String);

impl Address {
    pub fn new(addr: impl Into<String>) -> Self {
        Self(addr.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub account: Address,
    pub collateral_asset: Address,
    pub collateral_amount: u128,
    pub debt_amount: u128,
    pub last_updated_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OraclePrice {
    pub asset: Address,
    pub price_wad: u128,
    pub block_number: u64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationIntent {
    pub trace_id: String,
    pub borrower: Address,
    pub collateral_asset: Address,
    pub debt_to_cover: u128,
    pub min_collateral_expected: u128,
    pub block_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetConfig {
    pub asset: Address,
    pub symbol: String,
    pub decimals: u8,
    pub ltv_wad: u128,
    pub liq_threshold_wad: u128,
    pub is_active: bool,
}
