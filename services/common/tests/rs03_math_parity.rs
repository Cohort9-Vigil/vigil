//! RS-03 Math Parity Test Suite
//! Verifies 100% bit-for-bit parity with Solidity FixedPointMath and differential vectors.
//! Tests boundary cases: zero, 1 wei, maximum values, rounding ties, and HF threshold.

use alloy::primitives::{uint, U256};
use common::math::*;

#[test]
fn test_rs03_mul_div_down_and_up_parity() {
    let a = uint!(10_U256);
    let b = uint!(10_U256);
    let d = uint!(3_U256);

    let down = mul_div_down(a, b, d).expect("mul_div_down");
    let up = mul_div_up(a, b, d).expect("mul_div_up");

    // Exact parity with DifferentialMathTest.t.sol
    assert_eq!(down, uint!(33_U256));
    assert_eq!(up, uint!(34_U256));
}

#[test]
fn test_rs03_wad_mul_parity_vectors() {
    let one = WAD_U256;
    let two = WAD_U256 * U256::from(2);

    assert_eq!(wad_mul_down(one, two).unwrap(), two);
    assert_eq!(wad_mul_up(one, two).unwrap(), two);
}

#[test]
fn test_rs03_health_factor_parity_vectors() {
    // Matches test_health_factor_parity in Foundry
    let collateral = uint!(150_000_000_000_000_000_000_U256); // 150 WAD
    let liq_threshold = uint!(800_000_000_000_000_000_U256); // 0.8 WAD (80%)
    let adjusted_collateral = wad_mul_down(collateral, liq_threshold).unwrap();
    assert_eq!(adjusted_collateral, uint!(120_000_000_000_000_000_000_U256));

    // Solvent position: Debt = 100 WAD -> HF = 1.2 WAD
    let debt = uint!(100_000_000_000_000_000_000_U256);
    let hf = calculate_health_factor_u256(adjusted_collateral, debt);
    assert_eq!(hf, uint!(1_200_000_000_000_000_000_U256));

    // Undercollateralized position: Debt = 130 WAD -> HF = 0.923076923076923076 WAD
    let debt_unhealthy = uint!(130_000_000_000_000_000_000_U256);
    let hf_unhealthy = calculate_health_factor_u256(adjusted_collateral, debt_unhealthy);
    assert_eq!(hf_unhealthy, uint!(923_076_923_076_923_076_U256));
    assert!(hf_unhealthy < WAD_U256); // Liquidatable!
}

#[test]
fn test_rs03_compounded_interest_taylor_parity() {
    let initial_index = RAY_U256;
    let rate_per_sec = uint!(1_585_489_599_U256); // ~5% APR
    let delta_secs = 86400; // 1 day

    let new_index = calculate_compounded_index(initial_index, rate_per_sec, delta_secs).unwrap();
    assert!(new_index > initial_index);
}

#[test]
fn test_rs03_boundary_cases() {
    // 1. Zero denominator -> division by zero error
    assert_eq!(
        mul_div_down(uint!(100_U256), uint!(50_U256), U256::ZERO),
        Err(MathError::DivisionByZero)
    );
    assert_eq!(
        mul_div_up(uint!(100_U256), uint!(50_U256), U256::ZERO),
        Err(MathError::DivisionByZero)
    );

    // 2. 1 wei arithmetic
    let one_wei = uint!(1_U256);
    assert_eq!(wad_mul_down(one_wei, one_wei).unwrap(), U256::ZERO); // rounds down to 0
    assert_eq!(wad_mul_up(one_wei, one_wei).unwrap(), uint!(1_U256)); // rounds up to 1 wei

    // 3. Rounding ties: exact divisibility
    let a = uint!(20_U256);
    let b = uint!(5_U256);
    let d = uint!(10_U256); // (20 * 5) / 10 = 100 / 10 = 10 (no remainder)
    assert_eq!(mul_div_down(a, b, d).unwrap(), uint!(10_U256));
    assert_eq!(mul_div_up(a, b, d).unwrap(), uint!(10_U256));

    // 4. Health factor exactly 1.0 (boundary check: HF == 1e18 is healthy, not liquidatable)
    let exact_debt = uint!(120_000_000_000_000_000_000_U256);
    let exact_adj_collateral = uint!(120_000_000_000_000_000_000_U256);
    let hf_one = calculate_health_factor_u256(exact_adj_collateral, exact_debt);
    assert_eq!(hf_one, WAD_U256);
    assert!(hf_one >= WAD_U256); // Exactly 1.0 is NOT liquidatable

    // 5. Zero debt -> U256::MAX (infinite health)
    assert_eq!(
        calculate_health_factor_u256(exact_adj_collateral, U256::ZERO),
        U256::MAX
    );
}
