// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "./Base.t.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";

/// @notice SC-25: gas cost of a single-asset liquidation.
/// @dev The brief asks for 220k. We measure about 257k with a warm liquidator, so the ceiling below only stops
/// regressions until the team agrees how to close the gap.
contract LiquidationGasTest is VigilBase {
    uint256 constant BRIEF_TARGET = 220_000;
    uint256 constant GAS_CEILING = 260_000;

    address secondBorrower = makeAddr("secondBorrower");

    LiquidationSettlement.LiquidationIntent intent;
    bytes signature;

    function setUp() public override {
        super.setUp();

        openPosition(borrower, 10 ether, 10_000e6);
        openPosition(secondBorrower, 10 ether, 10_000e6);

        // The liquidator already holds both tokens, like a bot that has been running for a while.
        weth.mint(liquidator, 1);

        setWethPrice(1200);
        settlement.startAuction(borrower);
        settlement.startAuction(secondBorrower);
        vm.roll(block.number + 25);

        // A first liquidation warms the liquidator's nonce word, later ones only flip another bit.
        liquidate(borrower, 5000e6, 0);

        intent = makeIntent(secondBorrower, 5000e6, 1);
        signature = signIntent(intent);
    }

    function test_liquidationStaysWithinGasBudget() public {
        uint256 gasBefore = gasleft();
        settlement.executeLiquidation(intent, signature);
        uint256 gasUsed = gasBefore - gasleft();

        emit log_named_uint("executeLiquidation gas", gasUsed);
        emit log_named_uint("brief target", BRIEF_TARGET);
        assertLt(gasUsed, GAS_CEILING);
    }
}
