// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {FixedPointMath} from "./FixedPointMath.sol";

/// @notice Kinked borrow rate: gentle up to the optimal utilization, steep after it.
library InterestRateModel {
    using FixedPointMath for uint256;

    uint256 internal constant SECONDS_PER_YEAR = 365 days;

    /// @dev All values are annual rates or utilizations in RAY.
    struct RateParams {
        uint256 baseRate;
        uint256 slope1;
        uint256 slope2;
        uint256 optimalUtilization;
    }

    error InvalidOptimalUtilization();

    function validate(RateParams memory params) internal pure {
        if (params.optimalUtilization == 0 || params.optimalUtilization >= FixedPointMath.RAY) {
            revert InvalidOptimalUtilization();
        }
    }

    /// @param utilization Borrowed over supplied, in RAY, at most 1e27.
    function borrowRateAnnual(uint256 utilization, RateParams memory params) internal pure returns (uint256) {
        if (utilization <= params.optimalUtilization) {
            return params.baseRate + params.slope1.mulDivDown(utilization, params.optimalUtilization);
        }

        uint256 excess = utilization - params.optimalUtilization;
        uint256 excessRange = FixedPointMath.RAY - params.optimalUtilization;
        return params.baseRate + params.slope1 + params.slope2.mulDivDown(excess, excessRange);
    }

    function borrowRatePerSecond(uint256 utilization, RateParams memory params) internal pure returns (uint256) {
        return borrowRateAnnual(utilization, params) / SECONDS_PER_YEAR;
    }
}
