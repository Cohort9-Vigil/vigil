// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test} from "forge-std/Test.sol";

import {FixedPointMath} from "../src/libraries/FixedPointMath.sol";
import {InterestRateModel} from "../src/libraries/InterestRateModel.sol";
import {CollateralBitmap} from "../src/libraries/CollateralBitmap.sol";

contract FixedPointMathTest is Test {
    uint256 constant RAY = 1e27;

    // SC-07: rounding direction is explicit
    function test_roundingDirections() public pure {
        assertEq(FixedPointMath.rayMulDown(1, RAY / 2), 0);
        assertEq(FixedPointMath.rayMulUp(1, RAY / 2), 1);
        assertEq(FixedPointMath.rayDivDown(1, 3 * RAY), 0);
        assertEq(FixedPointMath.rayDivUp(1, 3 * RAY), 1);
        assertEq(FixedPointMath.rayMulUp(0, RAY), 0);
    }

    function testFuzz_roundUpIsNeverBelowRoundDown(uint128 a, uint128 b) public pure {
        vm.assume(b > 0);
        uint256 down = FixedPointMath.mulDivDown(a, b, RAY);
        uint256 up = FixedPointMath.mulDivUp(a, b, RAY);
        assertGe(up, down);
        assertLe(up - down, 1);
    }

    function test_compoundedGrowthWithNoTimeIsOne() public pure {
        assertEq(FixedPointMath.compoundedGrowth(1e18, 0), RAY);
    }

    // SC-06: one second is exact, longer periods follow the three-term series
    function test_compoundedGrowthTerms() public pure {
        uint256 rate = 1e18;
        assertEq(FixedPointMath.compoundedGrowth(rate, 1), RAY + rate);

        uint256 ratePower2 = FixedPointMath.rayMulDown(rate, rate);
        assertEq(FixedPointMath.compoundedGrowth(rate, 2), RAY + 2 * rate + ratePower2);

        uint256 ratePower3 = FixedPointMath.rayMulDown(ratePower2, rate);
        assertEq(FixedPointMath.compoundedGrowth(rate, 3), RAY + 3 * rate + 3 * ratePower2 + ratePower3);
    }

    function test_compoundedGrowthCloseToExactOverAYear() public pure {
        // 10% a year, per second
        uint256 ratePerSecond = (RAY / 10) / 365 days;
        uint256 growth = FixedPointMath.compoundedGrowth(ratePerSecond, 365 days);
        // e^0.1 = 1.10517..., dropping the fourth term costs under 0.001%
        assertApproxEqRel(growth, 1.10517091807e27, 1e13);
    }

    function testFuzz_compoundedGrowthNeverShrinks(uint64 rate, uint32 elapsed) public pure {
        assertGe(FixedPointMath.compoundedGrowth(rate, elapsed), RAY);
    }

    // SC-08: kinked rate model at 0%, optimal and 100% utilization
    function test_rateModelKink() public pure {
        InterestRateModel.RateParams memory params = InterestRateModel.RateParams(0.02e27, 0.04e27, 0.75e27, 0.8e27);

        assertEq(InterestRateModel.borrowRateAnnual(0, params), 0.02e27);
        assertEq(InterestRateModel.borrowRateAnnual(0.8e27, params), 0.06e27);
        assertEq(InterestRateModel.borrowRateAnnual(RAY, params), 0.81e27);
        assertEq(InterestRateModel.borrowRateAnnual(0.4e27, params), 0.04e27);
    }

    function testFuzz_rateNeverDecreasesWithUtilization(uint256 low, uint256 high) public pure {
        low = bound(low, 0, RAY);
        high = bound(high, low, RAY);
        InterestRateModel.RateParams memory params = InterestRateModel.RateParams(0.02e27, 0.04e27, 0.75e27, 0.8e27);

        assertLe(InterestRateModel.borrowRateAnnual(low, params), InterestRateModel.borrowRateAnnual(high, params));
    }

    function test_bitmapWalkVisitsOnlySetBits() public pure {
        uint256 bitmap = CollateralBitmap.set(CollateralBitmap.set(0, 0), 2);

        assertEq(CollateralBitmap.lowestSetBit(bitmap), 0);
        bitmap = CollateralBitmap.clearLowest(bitmap);
        assertEq(CollateralBitmap.lowestSetBit(bitmap), 2);
        assertEq(CollateralBitmap.clearLowest(bitmap), 0);
        assertEq(CollateralBitmap.clear(7, 1), 5);
    }
}
