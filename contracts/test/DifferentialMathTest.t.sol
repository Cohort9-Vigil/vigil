// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/libraries/FixedPointMath.sol";

contract DifferentialMathTest is Test {
    using FixedPointMath for uint256;

    uint256 constant WAD = 1e18;
    uint256 constant RAY = 1e27;

    function test_mul_div_down_basic() public pure {
        uint256 a = 10;
        uint256 b = 10;
        uint256 d = 3;
        assertEq(FixedPointMath.mulDivDown(a, b, d), 33);
    }

    function test_mul_div_up_basic() public pure {
        uint256 a = 10;
        uint256 b = 10;
        uint256 d = 3;
        assertEq(FixedPointMath.mulDivUp(a, b, d), 34);
    }

    function test_wad_mul_parity() public pure {
        uint256 one = WAD;
        uint256 two = 2 * WAD;
        assertEq(FixedPointMath.wadMulDown(one, two), two);
        assertEq(FixedPointMath.wadMulUp(one, two), two);
    }

    function test_health_factor_parity() public pure {
        // Collateral value = $150 WAD, liquidation threshold = 80% (0.8 WAD)
        uint256 collateral = 150 * WAD;
        uint256 liqThreshold = 800_000_000_000_000_000;
        uint256 adjustedCollateral = FixedPointMath.wadMulDown(collateral, liqThreshold);
        assertEq(adjustedCollateral, 120 * WAD);

        // Debt = $100 WAD -> HF = 1.2 WAD
        uint256 debt = 100 * WAD;
        uint256 hf = FixedPointMath.calculateHealthFactor(adjustedCollateral, debt);
        assertEq(hf, 1_200_000_000_000_000_000);

        // Debt = $130 WAD -> HF = 120 / 130 = 0.923076923076923076 WAD
        uint256 debtUnhealthy = 130 * WAD;
        uint256 hfUnhealthy = FixedPointMath.calculateHealthFactor(adjustedCollateral, debtUnhealthy);
        assertEq(hfUnhealthy, 923_076_923_076_923_076);
    }

    function test_compounded_interest_taylor() public pure {
        uint256 initialIndex = RAY;
        uint256 ratePerSec = 1_585_489_599; // ~5% APR in seconds
        uint256 deltaSecs = 86400; // 1 day

        uint256 newIndex = FixedPointMath.calculateCompoundedIndex(initialIndex, ratePerSec, deltaSecs);
        assertGt(newIndex, initialIndex);
    }

    function testFuzz_mulDiv_down_le_up(uint128 a, uint128 b, uint128 d) public pure {
        vm.assume(d > 0);
        uint256 down = FixedPointMath.mulDivDown(a, b, d);
        uint256 up = FixedPointMath.mulDivUp(a, b, d);
        assertLe(down, up);
        if ((uint256(a) * uint256(b)) % d == 0) {
            assertEq(down, up);
        } else {
            assertEq(down + 1, up);
        }
    }
}
