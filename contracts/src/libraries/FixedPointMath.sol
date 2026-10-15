// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title FixedPointMath
/// @notice Arithmetic library for WAD (1e18) and RAY (1e27) fixed-point math with exact rounding parity to Vigil Risk Engine.
library FixedPointMath {
    error DivisionByZero();
    error Overflow();

    uint256 internal constant WAD = 1e18;
    uint256 internal constant HALF_WAD = 0.5e18;
    uint256 internal constant RAY = 1e27;
    uint256 internal constant HALF_RAY = 0.5e27;
    uint256 internal constant CLOSE_FACTOR_50_PERCENT = 0.5e18;

    /// @notice Multiplies x and y, then divides by denominator, rounding down.
    function mulDivDown(uint256 x, uint256 y, uint256 denominator) internal pure returns (uint256 z) {
        if (denominator == 0) revert DivisionByZero();
        assembly {
            // Equivalent to (x * y) / denominator using 512-bit intermediate product
            let mm := mulmod(x, y, not(0))
            let p0 := mul(x, y)
            let p1 := sub(sub(mm, p0), lt(mm, p0))

            if iszero(p1) {
                z := div(p0, denominator)
            }
            if iszero(iszero(p1)) {
                // For general 512/256 division, revert if overflow
                if iszero(gt(denominator, p1)) {
                    // Revert with Overflow()
                    mstore(0x00, 0x35278d12)
                    revert(0x1c, 0x04)
                }
                // Compute remainder of 512-bit division
                let r := mulmod(x, y, denominator)
                let p0_adj := sub(p0, r)
                let p1_adj := sub(p1, lt(p0, r))

                // Invert denominator mod 2^256
                let inv := sub(mul(3, denominator), 2)
                inv := mul(inv, sub(2, mul(denominator, inv)))
                inv := mul(inv, sub(2, mul(denominator, inv)))
                inv := mul(inv, sub(2, mul(denominator, inv)))
                inv := mul(inv, sub(2, mul(denominator, inv)))
                inv := mul(inv, sub(2, mul(denominator, inv)))

                z := mul(p0_adj, inv)
            }
        }
    }

    /// @notice Multiplies x and y, then divides by denominator, rounding up.
    function mulDivUp(uint256 x, uint256 y, uint256 denominator) internal pure returns (uint256 z) {
        if (denominator == 0) revert DivisionByZero();
        if (x == 0 || y == 0) return 0;
        z = mulDivDown(x, y, denominator);
        if (mulmod(x, y, denominator) != 0) {
            if (z == type(uint256).max) revert Overflow();
            unchecked {
                z += 1;
            }
        }
    }

    function wadMulDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, y, WAD);
    }

    function wadMulUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, y, WAD);
    }

    function wadDivDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, WAD, y);
    }

    function wadDivUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, WAD, y);
    }

    function rayMulDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, y, RAY);
    }

    function rayMulUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, y, RAY);
    }

    function rayDivDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, RAY, y);
    }

    function rayDivUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, RAY, y);
    }

    /// @notice Computes borrower health factor with conservative round-down.
    function calculateHealthFactor(uint256 adjustedCollateralValueWad, uint256 totalDebtWad)
        internal
        pure
        returns (uint256)
    {
        if (totalDebtWad == 0) return type(uint256).max;
        return wadDivDown(adjustedCollateralValueWad, totalDebtWad);
    }

    /// @notice 2nd-order Taylor expansion for per-second interest compounding.
    function calculateCompoundedIndex(uint256 currentIndexRay, uint256 ratePerSecondRay, uint256 deltaSeconds)
        internal
        pure
        returns (uint256)
    {
        if (deltaSeconds == 0 || ratePerSecondRay == 0) return currentIndexRay;

        uint256 rateDt = ratePerSecondRay * deltaSeconds;
        // term2 = (rateDt^2) / (2 * RAY)
        uint256 term2 = mulDivDown(rateDt, rateDt, 2 * RAY);
        uint256 factor = RAY + rateDt + term2;

        return rayMulUp(currentIndexRay, factor);
    }
}
