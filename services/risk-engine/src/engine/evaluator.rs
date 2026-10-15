//! Position valuation, Health Factor evaluation, and Liquidation Signal generation.
//! Enforces zero floating point arithmetic and bit-for-bit parity with Solidity.

use alloy::primitives::U256;
use common::events::LiquidationSignal;
use common::math::{
    calculate_health_factor_u256, calculate_max_repay_allowed, normalize_to_wad, wad_mul_down,
    WAD_U256,
};
use common::types::Address;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionEvaluationInput<'a> {
    pub borrower: &'a str,
    pub collateral_asset: &'a str,
    pub collateral_amount: u128,
    pub collateral_decimals: u8,
    pub collateral_price_wad: u128,
    pub liq_threshold_wad: u128,
    pub debt_amount: u128,
    pub debt_asset: &'a str,
    pub block_number: u64,
    pub block_hash: &'a str,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationResult {
    pub health_factor_wad: u128,
    pub total_collateral_value_wad: u128,
    pub adjusted_collateral_value_wad: u128,
    pub total_debt_wad: u128,
    pub is_liquidatable: bool,
    pub signal: Option<LiquidationSignal>,
}

/// Evaluates a single position for solvency and generates a liquidation signal if unhealthy.
pub fn evaluate_position(input: &PositionEvaluationInput) -> anyhow::Result<EvaluationResult> {
    let collateral_raw = U256::from(input.collateral_amount);
    let collateral_wad = normalize_to_wad(collateral_raw, input.collateral_decimals)?;
    let price_wad = U256::from(input.collateral_price_wad);

    // Total Collateral Value in USD WAD (rounded down)
    let total_collateral_val_wad = wad_mul_down(collateral_wad, price_wad)?;

    // Adjusted Collateral Value = Collateral Value * Liquidation Threshold (rounded down)
    let liq_threshold = U256::from(input.liq_threshold_wad);
    let adjusted_collateral_val_wad = wad_mul_down(total_collateral_val_wad, liq_threshold)?;

    // Debt Value in USD WAD
    let total_debt_wad = U256::from(input.debt_amount);

    // Health Factor = (Adjusted Collateral * 1e18) / Total Debt (rounded down)
    let hf_u256 = calculate_health_factor_u256(adjusted_collateral_val_wad, total_debt_wad);

    // Liquidatable if HF < 1.0 (1e18 WAD) AND debt > 0
    let is_liquidatable = hf_u256 < WAD_U256 && total_debt_wad > U256::ZERO;

    let signal = if is_liquidatable {
        let debt_to_cover = calculate_max_repay_allowed(total_debt_wad)?;
        let dedupe_key = format!(
            "{}:{}:{}:{}",
            input.borrower.to_lowercase(),
            input.collateral_asset.to_lowercase(),
            input.debt_asset.to_lowercase(),
            input.block_hash
        );

        Some(LiquidationSignal {
            trace_id: Uuid::new_v4().to_string(),
            dedupe_key,
            borrower: Address::new(input.borrower),
            collateral_asset: Address::new(input.collateral_asset),
            debt_asset: Address::new(input.debt_asset),
            health_factor_wad: u128::try_from(hf_u256).unwrap_or(u128::MAX),
            total_debt_wad: input.debt_amount,
            total_collateral_value_wad: u128::try_from(total_collateral_val_wad)
                .unwrap_or(u128::MAX),
            debt_to_cover: u128::try_from(debt_to_cover).unwrap_or(input.debt_amount / 2),
            price_snapshot_wad: input.collateral_price_wad,
            detected_at_block: input.block_number,
            block_hash: input.block_hash.to_string(),
            timestamp: input.timestamp,
        })
    } else {
        None
    };

    Ok(EvaluationResult {
        health_factor_wad: u128::try_from(hf_u256).unwrap_or(u128::MAX),
        total_collateral_value_wad: u128::try_from(total_collateral_val_wad).unwrap_or(0),
        adjusted_collateral_value_wad: u128::try_from(adjusted_collateral_val_wad).unwrap_or(0),
        total_debt_wad: input.debt_amount,
        is_liquidatable,
        signal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::math::WAD_U128 as WAD;

    #[test]
    fn test_evaluator_healthy_position() {
        let input = PositionEvaluationInput {
            borrower: "0x1111111111111111111111111111111111111111",
            collateral_asset: "0x2222222222222222222222222222222222222222",
            collateral_amount: 150 * WAD, // $150
            collateral_decimals: 18,
            collateral_price_wad: WAD,                  // $1.00
            liq_threshold_wad: 800_000_000_000_000_000, // 80%
            debt_amount: 100 * WAD,                     // $100
            debt_asset: "0x3333333333333333333333333333333333333333",
            block_number: 10,
            block_hash: "0xhash10",
            timestamp: 1700000000,
        };

        let result = evaluate_position(&input).expect("evaluate");
        assert_eq!(result.health_factor_wad, 1_200_000_000_000_000_000);
        assert!(!result.is_liquidatable);
        assert!(result.signal.is_none());
    }

    #[test]
    fn test_evaluator_unhealthy_position_generates_signal() {
        let input = PositionEvaluationInput {
            borrower: "0x1111111111111111111111111111111111111111",
            collateral_asset: "0x2222222222222222222222222222222222222222",
            collateral_amount: 100 * WAD, // $100
            collateral_decimals: 18,
            collateral_price_wad: WAD,                  // $1.00
            liq_threshold_wad: 800_000_000_000_000_000, // 80% (adj collateral = $80)
            debt_amount: 90 * WAD,                      // $90 (HF = 80/90 = 0.888... WAD)
            debt_asset: "0x3333333333333333333333333333333333333333",
            block_number: 11,
            block_hash: "0xhash11",
            timestamp: 1700000005,
        };

        let result = evaluate_position(&input).expect("evaluate");
        assert!(result.is_liquidatable);
        assert!(result.health_factor_wad < WAD);
        let signal = result.signal.expect("must produce signal");
        assert_eq!(signal.debt_to_cover, 45 * WAD); // 50% of 90 WAD
        assert_eq!(
            signal.dedupe_key,
            "0x1111111111111111111111111111111111111111:0x2222222222222222222222222222222222222222:0x3333333333333333333333333333333333333333:0xhash11"
        );
    }
}
