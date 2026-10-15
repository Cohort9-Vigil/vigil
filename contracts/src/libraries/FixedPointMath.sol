// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice WAD (1e18) and RAY (1e27) fixed-point math. Every function names its rounding direction.
/// @dev The Rust common crate must follow the same operation order to match bit for bit.
library FixedPointMath {
    error DivisionByZero();
    error Overflow();

    uint256 internal constant WAD = 1e18;
    uint256 internal constant HALF_WAD = 0.5e18;
    uint256 internal constant RAY = 1e27;
    uint256 internal constant HALF_RAY = 0.5e27;
    uint256 internal constant CLOSE_FACTOR_50_PERCENT = 0.5e18;

    function mulDivDown(uint256 a, uint256 b, uint256 denominator) internal pure returns (uint256) {
        if (denominator == 0) revert DivisionByZero();
        return (a * b) / denominator;
    }

    function mulDivUp(uint256 a, uint256 b, uint256 denominator) internal pure returns (uint256) {
        if (denominator == 0) revert DivisionByZero();
        uint256 product = a * b;
        if (product == 0) return 0;
        return (product - 1) / denominator + 1;
    }

    function wadMulDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, y, WAD);
    }

    function wadMulUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, y, WAD);
    }

    function wadDivDown(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulDivDown(a, WAD, b);
    }

    function wadDivUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, WAD, y);
    }

    function rayMulDown(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulDivDown(a, b, RAY);
    }

    function rayMulUp(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulDivUp(a, b, RAY);
    }

    function rayDivDown(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulDivDown(a, RAY, b);
    }

    function rayDivUp(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulDivUp(a, RAY, b);
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
        uint256 term2 = mulDivDown(rateDt, rateDt, 2 * RAY);
        uint256 factor = RAY + rateDt + term2;

        return rayMulUp(currentIndexRay, factor);
    }

    /// @notice (1 + r)^t as 1 + rt + t(t-1)/2 * r^2 + t(t-1)(t-2)/6 * r^3, rounded down.
    /// @param ratePerSecond Interest rate per second in RAY.
    function compoundedGrowth(uint256 ratePerSecond, uint256 elapsed) internal pure returns (uint256) {
        if (elapsed == 0) return RAY;

        uint256 secondTerm = elapsed < 2 ? 0 : (elapsed * (elapsed - 1)) / 2;
        uint256 thirdTerm = elapsed < 3 ? 0 : (elapsed * (elapsed - 1) * (elapsed - 2)) / 6;

        uint256 ratePower2 = rayMulDown(ratePerSecond, ratePerSecond);
        uint256 ratePower3 = rayMulDown(ratePower2, ratePerSecond);

        return RAY + ratePerSecond * elapsed + ratePower2 * secondTerm + ratePower3 * thirdTerm;
    }
}
