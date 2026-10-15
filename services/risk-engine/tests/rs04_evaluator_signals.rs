//! RS-04 Integration Test: Valuation, Multi-Decimal Tokens, and Signal Generation
//!
//! Verifies:
//! 1. 6-decimal (USDC), 8-decimal (WBTC), and 18-decimal (WETH) normalization to WAD.
//! 2. 25% market crash simulation: position transitions from healthy (HF > 1) to liquidatable (HF < 1).
//! 3. 50% close factor cap: debt_to_cover is capped at 50% of outstanding debt.
//! 4. Deterministic deduplication key generation.
//! 5. HF = 1.0 boundary: position remains non-liquidatable.

use common::math::WAD;
use risk_engine::engine::evaluator::{evaluate_position, PositionEvaluationInput};

#[test]
fn test_rs04_multi_decimal_token_valuation() {
    let debt_asset = "0x0000000000000000000000000000000000000002";

    // 1. 6-decimal token: 1,000 USDC collateral (1,000 * 1e6 = 1_000_000_000)
    // Price = $1.00 WAD, LTV = 80% (0.8 WAD)
    // Adjusted collateral = $800 WAD. Debt = $500 WAD. HF = 800 / 500 = 1.6 WAD
    let usdc_input = PositionEvaluationInput {
        borrower: "0xuser1",
        collateral_asset: "0xusdc",
        collateral_amount: 1_000_000_000, // 1000 USDC
        collateral_decimals: 6,
        collateral_price_wad: WAD,
        liq_threshold_wad: 800_000_000_000_000_000,
        debt_amount: 500 * WAD,
        debt_asset,
        block_number: 100,
        block_hash: "0xhash100",
        timestamp: 1700000000,
    };

    let res = evaluate_position(&usdc_input).expect("evaluate usdc");
    assert_eq!(res.health_factor_wad, 1_600_000_000_000_000_000);
    assert!(!res.is_liquidatable);
    assert!(res.signal.is_none());

    // 2. 8-decimal token: 1.5 WBTC collateral (1.5 * 1e8 = 150_000_000)
    // Price = $60,000 WAD. Total value = $90,000 WAD.
    // Liq threshold = 80% -> Adj collateral = $72,000 WAD.
    // Debt = $50,000 WAD. HF = 72,000 / 50,000 = 1.44 WAD.
    let wbtc_input = PositionEvaluationInput {
        borrower: "0xuser2",
        collateral_asset: "0xwbtc",
        collateral_amount: 150_000_000, // 1.5 WBTC
        collateral_decimals: 8,
        collateral_price_wad: 60_000 * WAD,
        liq_threshold_wad: 800_000_000_000_000_000,
        debt_amount: 50_000 * WAD,
        debt_asset,
        block_number: 101,
        block_hash: "0xhash101",
        timestamp: 1700000005,
    };

    let res_wbtc = evaluate_position(&wbtc_input).expect("evaluate wbtc");
    assert_eq!(res_wbtc.health_factor_wad, 1_440_000_000_000_000_000);
    assert!(!res_wbtc.is_liquidatable);
}

#[test]
fn test_rs04_market_crash_25_percent_crosses_liquidation_threshold() {
    let debt_asset = "0x0000000000000000000000000000000000000002";
    let borrower = "0xborrower99";
    let collateral_asset = "0xcollateral99";

    // Position setup: 100 WETH collateral, initial price $2,000.
    // Collateral value = $200,000. Liq threshold = 80% -> Adj collateral = $160,000.
    // Debt = $140,000 WAD.
    // Initial HF = 160,000 / 140,000 = 1.142857142857142857 WAD (Healthy!)
    let initial_price = 2_000 * WAD;
    let initial_input = PositionEvaluationInput {
        borrower,
        collateral_asset,
        collateral_amount: 100 * WAD,
        collateral_decimals: 18,
        collateral_price_wad: initial_price,
        liq_threshold_wad: 800_000_000_000_000_000,
        debt_amount: 140_000 * WAD,
        debt_asset,
        block_number: 200,
        block_hash: "0xhash200",
        timestamp: 1700000000,
    };

    let res_initial = evaluate_position(&initial_input).expect("initial");
    assert!(!res_initial.is_liquidatable);
    assert_eq!(res_initial.health_factor_wad, 1_142_857_142_857_142_857);
    assert!(res_initial.signal.is_none());

    // Market crash: price drops 25% to $1,500.
    // Collateral value = $150,000. Adj collateral = 150,000 * 0.8 = $120,000.
    // Debt = $140,000.
    // New HF = 120,000 / 140,000 = 0.857142857142857142 WAD (< 1.0, Liquidatable!)
    let crashed_price = 1_500 * WAD;
    let crashed_input = PositionEvaluationInput {
        borrower,
        collateral_asset,
        collateral_amount: 100 * WAD,
        collateral_decimals: 18,
        collateral_price_wad: crashed_price,
        liq_threshold_wad: 800_000_000_000_000_000,
        debt_amount: 140_000 * WAD,
        debt_asset,
        block_number: 215,
        block_hash: "0xhash215",
        timestamp: 1700000015,
    };

    let res_crashed = evaluate_position(&crashed_input).expect("crashed");
    assert!(res_crashed.is_liquidatable);
    assert_eq!(res_crashed.health_factor_wad, 857_142_857_142_857_142);

    let signal = res_crashed
        .signal
        .expect("liquidation signal must be generated");
    // 50% close factor rule: debt_to_cover = 140,000 / 2 = 70,000 WAD
    assert_eq!(signal.debt_to_cover, 70_000 * WAD);
    assert_eq!(signal.detected_at_block, 215);
    assert_eq!(signal.block_hash, "0xhash215");

    // Deterministic dedupe key
    assert_eq!(
        signal.dedupe_key,
        format!(
            "{}:{}:{}:{}",
            borrower, collateral_asset, debt_asset, "0xhash215"
        )
    );
}

#[test]
fn test_rs04_health_factor_one_boundary_is_not_liquidatable() {
    let input = PositionEvaluationInput {
        borrower: "0xuserExact",
        collateral_asset: "0xassetExact",
        collateral_amount: 100 * WAD,
        collateral_decimals: 18,
        collateral_price_wad: WAD,
        liq_threshold_wad: WAD, // 100% threshold
        debt_amount: 100 * WAD, // Exactly equal debt
        debt_asset: "0xdebt",
        block_number: 300,
        block_hash: "0xhash300",
        timestamp: 1700000000,
    };

    let res = evaluate_position(&input).expect("boundary");
    assert_eq!(res.health_factor_wad, WAD); // Exactly 1.0 WAD
    assert!(!res.is_liquidatable); // Healthy!
    assert!(res.signal.is_none());
}
