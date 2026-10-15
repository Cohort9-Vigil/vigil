//! Fixed-point math utilities for Vigil protocol.
//! Ensures 100% parity with Solidity WAD (1e18) and RAY (1e27) calculations.
//! All financial math uses `alloy::primitives::U256` with zero floating point operations.

use alloy::primitives::{uint, U256, U512};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MathError {
    #[error("Division by zero")]
    DivisionByZero,
    #[error("Arithmetic overflow in multiplication or conversion")]
    Overflow,
    #[error("Invalid token decimals: {0}")]
    InvalidDecimals(u8),
}

/// 18 decimals scale constant (1e18)
pub const WAD_U256: U256 = uint!(1_000_000_000_000_000_000_U256);
pub const HALF_WAD_U256: U256 = uint!(500_000_000_000_000_000_U256);

/// 27 decimals scale constant (1e27)
pub const RAY_U256: U256 = uint!(1_000_000_000_000_000_000_000_000_000_U256);
pub const HALF_RAY_U256: U256 = uint!(500_000_000_000_000_000_000_000_000_U256);

/// 50% close factor in WAD (0.5 * 1e18)
pub const CLOSE_FACTOR_50_PERCENT: U256 = uint!(500_000_000_000_000_000_U256);

// Backward-compatible u128 constants
pub const WAD: u128 = 1_000_000_000_000_000_000;
pub const HALF_WAD: u128 = WAD / 2;
pub const RAY: u128 = 1_000_000_000_000_000_000_000_000_000;
pub const HALF_RAY: u128 = RAY / 2;

/// Safe conversion from U512 to U256 checking high 256 bits
#[inline]
pub fn u512_to_u256(val: U512) -> Result<U256, MathError> {
    let limbs = val.as_limbs();
    if limbs[4] != 0 || limbs[5] != 0 || limbs[6] != 0 || limbs[7] != 0 {
        return Err(MathError::Overflow);
    }
    Ok(U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]]))
}

// ============================================================================
// Core U256 Fixed-Point Functions with Exact Rounding Parity
// ============================================================================

/// Multiplies `a` and `b` then divides by `denominator`, rounding DOWN.
/// Uses 512-bit intermediate product to prevent premature overflow,
/// mirroring OpenZeppelin / PRBMath `mulDiv(a, b, denominator)`.
pub fn mul_div_down(a: U256, b: U256, denominator: U256) -> Result<U256, MathError> {
    if denominator == U256::ZERO {
        return Err(MathError::DivisionByZero);
    }
    let prod = a.widening_mul(b);
    let denom = U512::from(denominator);
    let quotient = prod / denom;
    u512_to_u256(quotient)
}

/// Multiplies `a` and `b` then divides by `denominator`, rounding UP.
/// Used for conservative debt calculations.
pub fn mul_div_up(a: U256, b: U256, denominator: U256) -> Result<U256, MathError> {
    if denominator == U256::ZERO {
        return Err(MathError::DivisionByZero);
    }
    let prod = a.widening_mul(b);
    if prod == U512::ZERO {
        return Ok(U256::ZERO);
    }
    let denom = U512::from(denominator);
    let quotient = (prod - U512::from(1)) / denom + U512::from(1);
    u512_to_u256(quotient)
}

/// WAD multiplication rounding down: (a * b) / 1e18
pub fn wad_mul_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, b, WAD_U256)
}

/// WAD multiplication rounding up: (a * b) / 1e18, rounded UP
pub fn wad_mul_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, b, WAD_U256)
}

/// WAD division rounding down: (a * 1e18) / b
pub fn wad_div_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, WAD_U256, b)
}

/// WAD division rounding up: (a * 1e18) / b, rounded UP
pub fn wad_div_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, WAD_U256, b)
}

/// RAY multiplication rounding down: (a * b) / 1e27
pub fn ray_mul_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, b, RAY_U256)
}

/// RAY multiplication rounding up: (a * b) / 1e27, rounded UP
pub fn ray_mul_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, b, RAY_U256)
}

/// RAY division rounding down: (a * 1e27) / b
pub fn ray_div_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, RAY_U256, b)
}

/// RAY division rounding up: (a * 1e27) / b, rounded UP
pub fn ray_div_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, RAY_U256, b)
}

// ============================================================================
// Token Decimals Normalization
// ============================================================================

/// Normalizes token amounts with arbitrary decimals (e.g. 6 for USDC, 8 for WBTC)
/// to standard 18-decimal WAD representation.
pub fn normalize_to_wad(amount: U256, decimals: u8) -> Result<U256, MathError> {
    match decimals {
        18 => Ok(amount),
        d if d < 18 => {
            let diff = 18 - d;
            let factor = U256::from(10).pow(U256::from(diff));
            amount.checked_mul(factor).ok_or(MathError::Overflow)
        }
        d if d <= 36 => {
            let diff = d - 18;
            let factor = U256::from(10).pow(U256::from(diff));
            Ok(amount / factor)
        }
        invalid => Err(MathError::InvalidDecimals(invalid)),
    }
}

/// Converts a 18-decimal WAD amount back into the token's native decimal representation.
pub fn denormalize_from_wad(amount_wad: U256, decimals: u8) -> Result<U256, MathError> {
    match decimals {
        18 => Ok(amount_wad),
        d if d < 18 => {
            let diff = 18 - d;
            let factor = U256::from(10).pow(U256::from(diff));
            Ok(amount_wad / factor)
        }
        d if d <= 36 => {
            let diff = d - 18;
            let factor = U256::from(10).pow(U256::from(diff));
            amount_wad.checked_mul(factor).ok_or(MathError::Overflow)
        }
        invalid => Err(MathError::InvalidDecimals(invalid)),
    }
}

// ============================================================================
// Taylor-Approximated Compounding Interest Parity
// ============================================================================

/// Calculates the new compounded interest index using a 2nd-order Taylor expansion:
/// compounding_factor = 1 + (rate_per_second * dt) + ((rate_per_second * dt)^2 / 2)
/// new_index = current_index * compounding_factor (in RAY)
pub fn calculate_compounded_index(
    current_index_ray: U256,
    rate_per_second_ray: U256,
    delta_seconds: u64,
) -> Result<U256, MathError> {
    if delta_seconds == 0 || rate_per_second_ray == U256::ZERO {
        return Ok(current_index_ray);
    }

    let dt = U256::from(delta_seconds);
    // term1 = rate_per_second * dt (in RAY)
    let rate_dt = rate_per_second_ray
        .checked_mul(dt)
        .ok_or(MathError::Overflow)?;

    // term2 = (rate_dt^2) / (2 * RAY)
    let rate_dt_sq = rate_dt.widening_mul(rate_dt);
    let two_ray = U512::from(
        RAY_U256
            .checked_mul(U256::from(2))
            .ok_or(MathError::Overflow)?,
    );
    let term2_u512 = rate_dt_sq / two_ray;
    let term2 = u512_to_u256(term2_u512)?;

    // compounding_factor = RAY + rate_dt + term2
    let factor = RAY_U256
        .checked_add(rate_dt)
        .ok_or(MathError::Overflow)?
        .checked_add(term2)
        .ok_or(MathError::Overflow)?;

    // new_index = ray_mul_up(current_index, factor) for conservative debt accrual
    ray_mul_up(current_index_ray, factor)
}

// ============================================================================
// Health Factor and Liquidation Logic
// ============================================================================

/// Calculates the borrower's health factor in WAD (1.0 = 1e18).
/// Returns U256::MAX when total_debt_wad is 0 (position is fully solvent).
/// Rounds DOWN conservatively so that the risk engine never over-estimates solvency.
pub fn calculate_health_factor_u256(
    adjusted_collateral_value_wad: U256,
    total_debt_wad: U256,
) -> U256 {
    if total_debt_wad == U256::ZERO {
        return U256::MAX;
    }
    // HF = (adjusted_collateral * 1e18) / total_debt
    wad_div_down(adjusted_collateral_value_wad, total_debt_wad).unwrap_or(U256::ZERO)
}

/// Calculates the maximum repay amount allowed under the 50% close factor rule.
pub fn calculate_max_repay_allowed(total_debt: U256) -> Result<U256, MathError> {
    wad_mul_down(total_debt, CLOSE_FACTOR_50_PERCENT)
}

// ============================================================================
// Backward-Compatible u128 Functions
// ============================================================================

pub fn wad_mul(a: u128, b: u128) -> u128 {
    (a * b) / WAD
}

pub fn wad_mul_up_u128(a: u128, b: u128) -> u128 {
    let product = a * b;
    if product == 0 {
        0
    } else {
        product.div_ceil(WAD)
    }
}

pub fn wad_div(a: u128, b: u128) -> u128 {
    assert!(b > 0, "Division by zero");
    (a * WAD) / b
}

pub fn wad_div_up_u128(a: u128, b: u128) -> u128 {
    assert!(b > 0, "Division by zero");
    let numerator = a * WAD;
    if numerator == 0 {
        0
    } else {
        numerator.div_ceil(b)
    }
}

pub fn calculate_health_factor(
    collateral_value_wad: u128,
    ltv_wad: u128,
    debt_value_wad: u128,
) -> u128 {
    if debt_value_wad == 0 {
        return u128::MAX;
    }
    let adjusted_collateral = wad_mul(collateral_value_wad, ltv_wad);
    wad_div(adjusted_collateral, debt_value_wad)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wad_mul_parity() {
        let one = WAD_U256;
        let two = WAD_U256 * U256::from(2);
        assert_eq!(wad_mul_down(one, two).unwrap(), two);
        assert_eq!(wad_mul_up(one, two).unwrap(), two);
    }

    #[test]
    fn test_mul_div_down_up_rounding_difference() {
        let a = uint!(10_U256);
        let b = uint!(10_U256);
        let d = uint!(3_U256); // 100 / 3 = 33.333...

        let down = mul_div_down(a, b, d).unwrap();
        let up = mul_div_up(a, b, d).unwrap();

        assert_eq!(down, uint!(33_U256));
        assert_eq!(up, uint!(34_U256));
    }

    #[test]
    fn test_token_decimals_normalization() {
        // 100 USDC (6 decimals) = 100_000_000 units
        let usdc_raw = uint!(100_000_000_U256);
        let usdc_wad = normalize_to_wad(usdc_raw, 6).unwrap();
        // 100 * 1e18
        assert_eq!(usdc_wad, uint!(100_000_000_000_000_000_000_U256));

        let usdc_back = denormalize_from_wad(usdc_wad, 6).unwrap();
        assert_eq!(usdc_back, usdc_raw);

        // 1.5 WBTC (8 decimals) = 150_000_000 units
        let wbtc_raw = uint!(150_000_000_U256);
        let wbtc_wad = normalize_to_wad(wbtc_raw, 8).unwrap();
        // 1.5 * 1e18 = 1_500_000_000_000_000_000
        assert_eq!(wbtc_wad, uint!(1_500_000_000_000_000_000_U256));

        let wbtc_back = denormalize_from_wad(wbtc_wad, 8).unwrap();
        assert_eq!(wbtc_back, wbtc_raw);
    }

    #[test]
    fn test_health_factor_u256_conservative() {
        // Collateral value = $150 WAD, liquidation threshold = 80% (0.8 WAD)
        let collateral = uint!(150_000_000_000_000_000_000_U256);
        let liq_threshold = uint!(800_000_000_000_000_000_U256);
        let adjusted_collateral = wad_mul_down(collateral, liq_threshold).unwrap();
        assert_eq!(adjusted_collateral, uint!(120_000_000_000_000_000_000_U256));

        // Debt = $100 WAD -> HF = 1.2 WAD
        let debt = uint!(100_000_000_000_000_000_000_U256);
        let hf = calculate_health_factor_u256(adjusted_collateral, debt);
        assert_eq!(hf, uint!(1_200_000_000_000_000_000_U256));

        // Debt = $130 WAD -> adjusted collateral = 120 -> HF = 120/130 = 0.923076923076923076 WAD
        let debt_unhealthy = uint!(130_000_000_000_000_000_000_U256);
        let hf_unhealthy = calculate_health_factor_u256(adjusted_collateral, debt_unhealthy);
        assert!(hf_unhealthy < WAD_U256); // < 1.0, liquidatable!
        assert_eq!(hf_unhealthy, uint!(923_076_923_076_923_076_U256));

        // Zero debt -> infinite health
        assert_eq!(
            calculate_health_factor_u256(adjusted_collateral, U256::ZERO),
            U256::MAX
        );
    }

    #[test]
    fn test_close_factor_50_percent() {
        let debt = uint!(100_000_000_000_000_000_000_U256);
        let max_repay = calculate_max_repay_allowed(debt).unwrap();
        assert_eq!(max_repay, uint!(50_000_000_000_000_000_000_U256));
    }

    #[test]
    fn test_compounded_interest_taylor() {
        let initial_index = RAY_U256; // 1.0 RAY
                                      // Rate: ~5% APR in seconds (5e16 / 31536000 ~= 1.585e9 RAY/sec)
        let rate_per_sec = uint!(1_585_489_599_U256);
        let delta_secs = 86400; // 1 day

        let new_index =
            calculate_compounded_index(initial_index, rate_per_sec, delta_secs).unwrap();
        assert!(new_index > initial_index);
        // Ensure strictly monotonic
        let next_index = calculate_compounded_index(new_index, rate_per_sec, delta_secs).unwrap();
        assert!(next_index > new_index);
    }
}
