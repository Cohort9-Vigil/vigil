// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "../Base.t.sol";
import {LiquidationSettlement} from "../../src/liquidation/LiquidationSettlement.sol";
import {OracleAdapter} from "../../src/oracle/OracleAdapter.sol";
import {MockAMM} from "../../src/oracle/MockAMM.sol";
import {IOracleAdapter} from "../../src/interfaces/IOracleAdapter.sol";
import {ITwapSource} from "../../src/interfaces/ITwapSource.sol";

/// @notice Borrows a huge amount for one transaction, pushes the AMM price down and tries to liquidate a healthy position.
contract FlashAttacker {
    MockAMM immutable amm;
    LiquidationSettlement immutable settlement;

    constructor(MockAMM amm_, LiquidationSettlement settlement_) {
        amm = amm_;
        settlement = settlement_;
    }

    function attack(address victim, uint256 flashAmount) external {
        // the flash-borrowed base tokens are dumped into the pool to crash the spot price
        amm.swapBaseForQuote(flashAmount);
        settlement.startAuction(victim);
    }
}

/// @notice SC-17
contract TwapAttackTest is VigilBase {
    MockAMM amm;
    FlashAttacker attacker;

    address victim = makeAddr("victim");

    function setUp() public override {
        super.setUp();

        amm = new MockAMM(1000e18, 2_000_000e18);
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.twapSource = ITwapSource(address(amm));
        source.twapWindow = 1 hours;
        source.maxDeviationBps = 500;
        oracle.setSource(address(weth), source);

        vm.warp(block.timestamp + 1 hours);
        _refreshFeeds();

        // a healthy position: $20,000 of WETH against $12,000 of debt, health factor 1.33
        openPosition(victim, 10 ether, 12_000e6);
        attacker = new FlashAttacker(amm, settlement);
    }

    function _refreshFeeds() internal {
        wethFeed.setAnswer(2000e8);
        usdcFeed.setAnswer(1e8);
    }

    function test_oracleIsNormalBeforeTheAttack() public view {
        assertEq(uint256(oracle.state()), uint256(IOracleAdapter.State.Normal));
        assertGt(pool.getHealthFactor(victim), 1e18);
    }

    function test_flashLoanPriceDumpCannotTriggerALiquidation() public {
        uint256 healthBefore = pool.getHealthFactor(victim);

        vm.expectRevert(LiquidationSettlement.PositionHealthy.selector);
        attacker.attack(victim, 900e18);

        assertEq(settlement.auctionStartBlock(victim), 0);
        assertEq(pool.getHealthFactor(victim), healthBefore);
    }

    function test_theDumpWasReal() public {
        // the attack only fails because of the oracle, the AMM price really does move
        uint256 spotBefore = amm.spotPrice();
        amm.swapBaseForQuote(900e18);
        assertLt(amm.spotPrice(), spotBefore / 3);
    }

    function test_theTwapDoesNotMoveInsideTheAttackTransaction() public {
        uint256 twapBefore = amm.consult(1 hours);
        amm.swapBaseForQuote(900e18);
        assertEq(amm.consult(1 hours), twapBefore);
        assertEq(uint256(oracle.state()), uint256(IOracleAdapter.State.Normal));
    }

    function test_attackerEndsWithNothing() public {
        vm.expectRevert(LiquidationSettlement.PositionHealthy.selector);
        attacker.attack(victim, 900e18);

        assertEq(weth.balanceOf(address(attacker)), 0);
        assertEq(usdc.balanceOf(address(attacker)), 0);
        assertEq(pool.getCollateralBalance(victim, address(weth)), 10 ether);
        assertEq(pool.getDebt(victim), 12_000e6);
    }

    function test_holdingTheManipulationFreezesTheOracleInsteadOfLiquidating() public {
        amm.swapBaseForQuote(900e18);

        // keep the price down long enough for the TWAP to follow it
        vm.warp(block.timestamp + 2 hours);
        _refreshFeeds();

        assertEq(uint256(oracle.state()), uint256(IOracleAdapter.State.Frozen));
        vm.expectRevert(LiquidationSettlement.OracleFrozen.selector);
        settlement.startAuction(victim);
    }

    function test_borrowingIsBlockedWhileTheOracleIsFrozen() public {
        amm.swapBaseForQuote(900e18);
        vm.warp(block.timestamp + 2 hours);
        _refreshFeeds();

        vm.prank(victim);
        vm.expectRevert();
        pool.borrow(1e6);
    }
}
