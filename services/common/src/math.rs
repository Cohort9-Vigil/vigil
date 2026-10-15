//! Fixed-point math utilities for Vigil protocol.
//! Ensures 100% parity with Solidity WAD (1e18) and RAY (1e27) calculations.
//! All financial math uses `alloy::primitives::U256` with zero floating point operations.

use alloy::primitives::{uint, U256, U512};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MathError {
    #[error("division by zero")]
    DivisionByZero,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("invalid token decimals: {0}")]
    InvalidDecimals(u8),
}

/// 18 decimals scale constant (1e18)
pub const WAD: U256 = uint!(1_000_000_000_000_000_000_U256);
pub const WAD_U256: U256 = WAD;
pub const HALF_WAD: U256 = uint!(500_000_000_000_000_000_U256);
pub const HALF_WAD_U256: U256 = HALF_WAD;

/// 27 decimals scale constant (1e27)
pub const RAY: U256 = uint!(1_000_000_000_000_000_000_000_000_000_U256);
pub const RAY_U256: U256 = RAY;
pub const HALF_RAY: U256 = uint!(500_000_000_000_000_000_000_000_000_U256);
pub const HALF_RAY_U256: U256 = HALF_RAY;

/// Basis points scale (10,000)
pub const BPS: U256 = uint!(10_000_U256);
pub const SECONDS_PER_YEAR: U256 = uint!(31_536_000_U256);

/// 50% close factor in WAD (0.5 * 1e18)
pub const CLOSE_FACTOR_50_PERCENT: U256 = HALF_WAD;

// Backward-compatible u128 constants
pub const WAD_U128: u128 = 1_000_000_000_000_000_000;
pub const HALF_WAD_U128: u128 = WAD_U128 / 2;
pub const RAY_U128: u128 = 1_000_000_000_000_000_000_000_000_000;
pub const HALF_RAY_U128: u128 = RAY_U128 / 2;

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
/// Uses 512-bit intermediate product to prevent premature overflow.
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

pub fn wad_mul_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, b, WAD)
}

pub fn wad_mul_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, b, WAD)
}

pub fn wad_div_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, WAD, b)
}

pub fn wad_div_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, WAD, b)
}

pub fn ray_mul_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, b, RAY)
}

pub fn ray_mul_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, b, RAY)
}

pub fn ray_div_down(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_down(a, RAY, b)
}

pub fn ray_div_up(a: U256, b: U256) -> Result<U256, MathError> {
    mul_div_up(a, RAY, b)
}

fn pow10(n: u32) -> U256 {
    U256::from(10u64).pow(U256::from(n))
}

/// Token amount with `decimals` into WAD, rounding down.
pub fn to_wad_down(amount: U256, decimals: u8) -> U256 {
    match decimals.cmp(&18) {
        std::cmp::Ordering::Equal => amount,
        std::cmp::Ordering::Less => amount * pow10(18 - decimals as u32),
        std::cmp::Ordering::Greater => amount / pow10(decimals as u32 - 18),
    }
}

/// Token amount with `decimals` into WAD, rounding up.
pub fn to_wad_up(amount: U256, decimals: u8) -> U256 {
    if decimals >= 18 {
        let f = pow10(decimals as u32 - 18);
        (amount + f - U256::from(1)) / f
    } else {
        amount * pow10(18 - decimals as u32)
    }
}

/// WAD into token units with `decimals`, rounding down.
pub fn from_wad_down(amount: U256, decimals: u8) -> U256 {
    match decimals.cmp(&18) {
        std::cmp::Ordering::Equal => amount,
        std::cmp::Ordering::Less => amount / pow10(18 - decimals as u32),
        std::cmp::Ordering::Greater => amount * pow10(decimals as u32 - 18),
    }
}

/// Normalizes any raw ERC-20 token amount to 18-decimal WAD representation.
pub fn normalize_to_wad(amount: U256, decimals: u8) -> Result<U256, MathError> {
    if decimals > 77 {
        return Err(MathError::InvalidDecimals(decimals));
    }
    Ok(to_wad_down(amount, decimals))
}

/// Denormalizes an 18-decimal WAD amount back to token native units.
pub fn denormalize_from_wad(amount_wad: U256, decimals: u8) -> Result<U256, MathError> {
    if decimals > 77 {
        return Err(MathError::InvalidDecimals(decimals));
    }
    Ok(from_wad_down(amount_wad, decimals))
}

/// Computes borrower health factor: `adjusted_collateral / total_debt` (WAD).
pub fn calculate_health_factor_u256(adjusted_collateral_wad: U256, total_debt_wad: U256) -> U256 {
    if total_debt_wad == U256::ZERO {
        return U256::MAX;
    }
    wad_div_down(adjusted_collateral_wad, total_debt_wad).unwrap_or(U256::ZERO)
}

/// 2nd-order Taylor expansion for per-second interest compounding.
pub fn calculate_compounded_index(
    current_index_ray: U256,
    rate_per_second_ray: U256,
    delta_seconds: u64,
) -> Result<U256, MathError> {
    if delta_seconds == 0 || rate_per_second_ray == U256::ZERO {
        return Ok(current_index_ray);
    }
    let dt = U256::from(delta_seconds);
    let rate_dt = rate_per_second_ray
        .checked_mul(dt)
        .ok_or(MathError::Overflow)?;

    let two_ray = RAY
        .checked_mul(U256::from(2))
        .ok_or(MathError::Overflow)?;
    let term2 = mul_div_down(rate_dt, rate_dt, two_ray)?;

    let factor = RAY
        .checked_add(rate_dt)
        .ok_or(MathError::Overflow)?
        .checked_add(term2)
        .ok_or(MathError::Overflow)?;

    ray_mul_up(current_index_ray, factor)
}

/// Computes maximum allowable repayment enforcing 50% close factor rule.
pub fn calculate_max_repay_allowed(total_debt_wad: U256) -> Result<U256, MathError> {
    wad_mul_down(total_debt_wad, CLOSE_FACTOR_50_PERCENT)
}

// ---- InterestMath & Rate Models ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateParams {
    pub base_rate: U256,
    pub slope1: U256,
    pub slope2: U256,
    pub optimal_util: U256,
}

/// Utilisation = debt / supply, capped at 100%. Zero supply gives zero.
pub fn utilization(total_debt: U256, total_supply: U256) -> Result<U256, MathError> {
    if total_supply.is_zero() {
        return Ok(U256::ZERO);
    }
    let u = ray_div_down(total_debt, total_supply)?;
    Ok(if u > RAY { RAY } else { u })
}

/// Annual borrow rate (RAY) at a utilisation level (RAY).
pub fn annual_borrow_rate(p: &RateParams, util: U256) -> Result<U256, MathError> {
    if util <= p.optimal_util {
        Ok(p.base_rate + mul_div_up(p.slope1, util, p.optimal_util)?)
    } else {
        let excess = util - p.optimal_util;
        Ok(p.base_rate + p.slope1 + mul_div_up(p.slope2, excess, RAY - p.optimal_util)?)
    }
}

/// Per-second rate (RAY), rounded up.
pub fn rate_per_second(annual: U256) -> Result<U256, MathError> {
    mul_div_up(annual, U256::from(1), SECONDS_PER_YEAR)
}

/// 3-term binomial Taylor approximation of (1 + r)^t, RAY-scaled, rounded up.
pub fn compound_factor(r_per_sec: U256, t: U256) -> Result<U256, MathError> {
    if t.is_zero() || r_per_sec.is_zero() {
        return Ok(RAY);
    }
    let term1 = r_per_sec.checked_mul(t).ok_or(MathError::Overflow)?;
    let r2 = ray_mul_up(r_per_sec, r_per_sec)?;
    let r3 = ray_mul_up(r2, r_per_sec)?;
    let one = U256::from(1);
    let two = U256::from(2);
    let three = U256::from(3);
    let k2 = if t >= two {
        t * (t - one) / two
    } else {
        U256::ZERO
    };
    let k3 = if t >= three {
        t * (t - one) * (t - two) / U256::from(6)
    } else {
        U256::ZERO
    };
    let sum = RAY
        .checked_add(term1)
        .and_then(|v| v.checked_add(r2.checked_mul(k2)?))
        .and_then(|v| v.checked_add(r3.checked_mul(k3)?))
        .ok_or(MathError::Overflow)?;
    Ok(sum)
}

// Backward-compatible u128 functions
pub fn wad_mul(a: u128, b: u128) -> u128 {
    (a * b) / WAD_U128
}

pub fn wad_div(a: u128, b: u128) -> u128 {
    assert!(b > 0, "Division by zero");
    (a * WAD_U128) / b
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

    fn u(x: u128) -> U256 {
        U256::from(x)
    }

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
        let usdc_raw = uint!(100_000_000_U256);
        let usdc_wad = normalize_to_wad(usdc_raw, 6).unwrap();
        assert_eq!(usdc_wad, uint!(100_000_000_000_000_000_000_U256));

        let usdc_back = denormalize_from_wad(usdc_wad, 6).unwrap();
        assert_eq!(usdc_back, usdc_raw);

        let wbtc_raw = uint!(150_000_000_U256);
        let wbtc_wad = normalize_to_wad(wbtc_raw, 8).unwrap();
        assert_eq!(wbtc_wad, uint!(1_500_000_000_000_000_000_U256));

        let wbtc_back = denormalize_from_wad(wbtc_wad, 8).unwrap();
        assert_eq!(wbtc_back, wbtc_raw);
    }

    #[test]
    fn test_health_factor_u256_conservative() {
        let collateral = uint!(150_000_000_000_000_000_000_U256);
        let liq_threshold = uint!(800_000_000_000_000_000_U256);
        let adjusted_collateral = wad_mul_down(collateral, liq_threshold).unwrap();
        assert_eq!(adjusted_collateral, uint!(120_000_000_000_000_000_000_U256));

        let debt = uint!(100_000_000_000_000_000_000_U256);
        let hf = calculate_health_factor_u256(adjusted_collateral, debt);
        assert_eq!(hf, uint!(1_200_000_000_000_000_000_U256));

        let debt_unhealthy = uint!(130_000_000_000_000_000_000_U256);
        let hf_unhealthy = calculate_health_factor_u256(adjusted_collateral, debt_unhealthy);
        assert!(hf_unhealthy < WAD_U256);
        assert_eq!(hf_unhealthy, uint!(923_076_923_076_923_076_U256));

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
        let initial_index = RAY_U256;
        let rate_per_sec = uint!(1_585_489_599_U256);
        let delta_secs = 86400;

        let new_index =
            calculate_compounded_index(initial_index, rate_per_sec, delta_secs).unwrap();
        assert!(new_index > initial_index);
        let next_index = calculate_compounded_index(new_index, rate_per_sec, delta_secs).unwrap();
        assert!(next_index > new_index);
    }

    #[test]
    fn constants_match_solidity() {
        assert_eq!(WAD, U256::from(10).pow(U256::from(18)));
        assert_eq!(RAY, U256::from(10).pow(U256::from(27)));
    }

    #[test]
    fn rounding_directions() {
        assert_eq!(wad_mul_down(u(3), u(3e17 as u128)).unwrap(), U256::ZERO);
        assert_eq!(wad_mul_up(u(3), u(3e17 as u128)).unwrap(), U256::from(1));
        assert_eq!(wad_div_down(u(1), u(3) * WAD).unwrap(), U256::ZERO);
        assert_eq!(wad_div_up(u(1), u(3) * WAD).unwrap(), U256::from(1));
        assert_eq!(ray_mul_up(U256::ZERO, RAY).unwrap(), U256::ZERO);
    }

    #[test]
    fn division_by_zero_is_an_error() {
        assert_eq!(
            wad_div_down(WAD, U256::ZERO),
            Err(MathError::DivisionByZero)
        );
    }

    #[test]
    fn decimal_normalisation() {
        assert_eq!(to_wad_down(u(1_000_000), 6), WAD);
        assert_eq!(to_wad_down(u(1_234_567), 6), u(1_234_567_000_000_000_000));
        assert_eq!(to_wad_down(u(5), 18), u(5));
        assert_eq!(from_wad_down(u(1_500_000_000_000_000_001), 6), u(1_500_000));
        assert_eq!(to_wad_up(u(1), 24), u(1));
        assert_eq!(to_wad_down(u(1), 24), U256::ZERO);
        assert_eq!(to_wad_up(u(1), 8), u(10_000_000_000));
    }

    #[test]
    fn kinked_rate_matches_contract_vectors() {
        let p = RateParams {
            base_rate: u(2e25 as u128),
            slope1: u(4e25 as u128),
            slope2: u(75e25 as u128),
            optimal_util: u(8e26 as u128),
        };
        assert_eq!(annual_borrow_rate(&p, U256::ZERO).unwrap(), p.base_rate);
        assert_eq!(
            annual_borrow_rate(&p, p.optimal_util).unwrap(),
            p.base_rate + p.slope1
        );
        assert_eq!(
            annual_borrow_rate(&p, RAY).unwrap(),
            p.base_rate + p.slope1 + p.slope2
        );
    }

    #[test]
    fn compound_one_year_at_ten_percent_is_close_to_e_pow_0_1() {
        let r = rate_per_second(u(1e26 as u128)).unwrap();
        let f = compound_factor(r, u(31_536_000)).unwrap();
        let expected = u(1_105_170_918_075_648_000_000_000_000);
        let diff = if f > expected {
            f - expected
        } else {
            expected - f
        };
        assert!(
            diff * U256::from(100_000) < expected,
            "factor {f} too far from e^0.1"
        );
    }
}
