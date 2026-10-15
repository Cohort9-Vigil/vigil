// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "./Base.t.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";

contract LiquidationTest is VigilBase {
    // 10 WETH at $2000 backing $10,000 of debt, health factor 1.6
    function setUp() public override {
        super.setUp();
        openPosition(borrower, 10 ether, 10_000e6);
    }

    function _crash() internal {
        setWethPrice(1200);
    }

    // SC-18
    function test_cannotStartAuctionOnHealthyPosition() public {
        vm.expectRevert(LiquidationSettlement.PositionHealthy.selector);
        settlement.startAuction(borrower);
    }

    function test_anyoneCanStartAuctionOnceUnhealthy() public {
        _crash();
        vm.prank(makeAddr("stranger"));
        settlement.startAuction(borrower);

        assertEq(settlement.auctionStartBlock(borrower), block.number);
    }

    function test_cannotStartTwice() public {
        _crash();
        settlement.startAuction(borrower);
        vm.expectRevert(LiquidationSettlement.AuctionAlreadyActive.selector);
        settlement.startAuction(borrower);
    }

    function test_bonusGrowsLinearlyThenCaps() public {
        _crash();
        settlement.startAuction(borrower);
        assertEq(settlement.currentBonusBps(borrower), 0);

        vm.roll(block.number + AUCTION_DURATION / 2);
        assertEq(settlement.currentBonusBps(borrower), MAX_BONUS_BPS / 2);

        vm.roll(block.number + AUCTION_DURATION);
        assertEq(settlement.currentBonusBps(borrower), MAX_BONUS_BPS);
    }

    function testFuzz_bonusNeverDecreases(uint8 firstWait, uint8 secondWait) public {
        _crash();
        settlement.startAuction(borrower);

        vm.roll(block.number + firstWait);
        uint256 first = settlement.currentBonusBps(borrower);
        vm.roll(block.number + secondWait);
        assertGe(settlement.currentBonusBps(borrower), first);
    }

    function test_auctionResetsWhenPositionRecovers() public {
        _crash();
        settlement.startAuction(borrower);

        vm.expectRevert(LiquidationSettlement.PositionHealthy.selector);
        settlement.resetAuction(borrower);

        setWethPrice(2000);
        settlement.resetAuction(borrower);
        assertEq(settlement.auctionStartBlock(borrower), 0);
        assertEq(settlement.currentBonusBps(borrower), 0);

        _crash();
        settlement.startAuction(borrower);
        assertEq(settlement.currentBonusBps(borrower), 0);
    }

    function test_resetNeedsAnAuction() public {
        vm.expectRevert(LiquidationSettlement.NoActiveAuction.selector);
        settlement.resetAuction(borrower);
    }

    function test_liquidationNeedsAnAuction() public {
        _crash();
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);
        vm.expectRevert(LiquidationSettlement.NoActiveAuction.selector);
        settlement.executeLiquidation(intent, signature);
    }

    function test_liquidationPaysTheBonus() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        uint256 usdcBefore = usdc.balanceOf(liquidator);
        liquidate(borrower, 5000e6, 0);

        // $5000 repaid plus the full 10% bonus, at $1200 a WETH
        uint256 repaid = 5000e6;
        uint256 expectedCollateral = (repaid * 11 / 10) * 1e12 * 1e18 / 1200e18;
        assertEq(weth.balanceOf(liquidator), expectedCollateral);
        assertEq(usdcBefore - usdc.balanceOf(liquidator), 5000e6);
        assertEq(pool.getDebt(borrower), 5000e6);
    }

    function test_noBonusRightAfterTheAuctionStarts() public {
        _crash();
        settlement.startAuction(borrower);
        liquidate(borrower, 1200e6, 0);

        assertEq(weth.balanceOf(liquidator), 1 ether);
    }

    // SC-19
    function test_repayIsCappedAtHalfTheDebt() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        liquidate(borrower, 100_000e6, 0);
        assertEq(pool.getDebt(borrower), 5000e6);
    }

    function test_dustExceptionClosesTheWholeDebt() public {
        address smallBorrower = makeAddr("smallBorrower");
        openPosition(smallBorrower, 0.01 ether, 15e6);
        _crash();
        startAuctionAndWait(smallBorrower, 10);

        // half would leave $7.50, under the $10 dust threshold
        liquidate(smallBorrower, 100e6, 0);
        assertEq(pool.getDebt(smallBorrower), 0);
        assertEq(settlement.auctionStartBlock(smallBorrower), 0);
    }

    function test_maxRepayInTheIntentIsRespected() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        liquidate(borrower, 2000e6, 0);
        assertEq(pool.getDebt(borrower), 8000e6);
    }

    // SC-20
    function test_replayedIntentReverts() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 7);
        bytes memory signature = signIntent(intent);
        settlement.executeLiquidation(intent, signature);

        vm.expectRevert(LiquidationSettlement.NonceAlreadyUsed.selector);
        settlement.executeLiquidation(intent, signature);
    }

    function test_expiredIntentReverts() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory signature = signIntent(intent);
        vm.warp(intent.deadline + 1);
        setWethPrice(1200);
        usdcFeed.setAnswer(1e8);

        vm.expectRevert(LiquidationSettlement.IntentExpired.selector);
        settlement.executeLiquidation(intent, signature);
    }

    function test_intentSignedForAnotherChainFails() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory signature = signIntent(intent);

        // a different chain id changes the domain, so the signature recovers to some other address
        vm.chainId(block.chainid + 1);
        vm.expectRevert();
        settlement.executeLiquidation(intent, signature);
    }

    function test_tamperedIntentFails() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory signature = signIntent(intent);
        intent.maxRepay = 5000e6;

        vm.expectRevert();
        settlement.executeLiquidation(intent, signature);
    }

    function test_garbageSignatureFails() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        vm.expectRevert(LiquidationSettlement.InvalidSignature.selector);
        settlement.executeLiquidation(intent, hex"1234");
    }

    function test_noncesCanBeUsedInAnyOrder() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        liquidate(borrower, 500e6, 300);
        liquidate(borrower, 500e6, 2);
        liquidate(borrower, 500e6, 256);
        liquidate(borrower, 500e6, 0);

        assertTrue(settlement.nonceBitmap(liquidator, 1) & (1 << 44) != 0);
        assertEq(settlement.nonceBitmap(liquidator, 0), 5);
    }

    function test_minCollateralOutIsEnforced() public {
        _crash();
        startAuctionAndWait(borrower, 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        intent.minCollateralOut = 100 ether;
        bytes memory signature = signIntent(intent);

        vm.expectRevert(LiquidationSettlement.InsufficientCollateralOut.selector);
        settlement.executeLiquidation(intent, signature);
    }

    // SC-21
    function test_liquidationDoesNotLowerHealthFactor() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        uint256 before = pool.getHealthFactor(borrower);
        liquidate(borrower, 5000e6, 0);
        assertGe(pool.getHealthFactor(borrower), before);
    }

    // an underwater position cannot be made healthier, so the close factor and the health check step aside
    function test_underwaterPositionCanBeClearedPastTheCloseFactor() public {
        setWethPrice(1000);
        startAuctionAndWait(borrower, AUCTION_DURATION / 2);

        liquidate(borrower, 6000e6, 0);

        // 60% of the debt in one go, a solvent position would be capped at 50%
        assertEq(pool.getDebt(borrower), 4000e6);
        assertTrue(pool.hasCollateral(borrower));
        assertGt(settlement.auctionStartBlock(borrower), 0);
    }

    function test_underwaterPositionIsFullyClearedWhenTheLiquidatorTakesAllCollateral() public {
        setWethPrice(1000);
        startAuctionAndWait(borrower, AUCTION_DURATION);

        liquidate(borrower, 100_000e6, 0);

        assertEq(pool.getDebt(borrower), 0);
        assertEq(weth.balanceOf(liquidator), 10 ether);
        assertEq(settlement.auctionStartBlock(borrower), 0);
    }

    function test_solventPositionStillRespectsTheCloseFactor() public {
        setWethPrice(1230);
        startAuctionAndWait(borrower, AUCTION_DURATION);

        // $12,300 of collateral covers the $10,000 debt and the bonus, so this is not underwater
        liquidate(borrower, 100_000e6, 0);
        assertEq(pool.getDebt(borrower), 5000e6);
    }

    // SC-22
    function test_collateralRunningOutWritesOffAgainstTheReserve() public {
        // let some interest build up so the reserve has something in it
        vm.warp(block.timestamp + 365 days);
        usdcFeed.setAnswer(1e8);
        setWethPrice(2000);
        vm.prank(lender);
        pool.withdrawSupply(1e6);
        uint256 reserveBefore = pool.poolState().reserve;
        assertGt(reserveBefore, 0);

        // collateral is now worth almost nothing
        wethFeed.setAnswer(1e6);
        startAuctionAndWait(borrower, AUCTION_DURATION);
        // the collateral is worth about $0.10, so that is all a liquidator pays for it
        liquidate(borrower, 1e6, 0);

        assertEq(pool.getDebt(borrower), 0);
        assertFalse(pool.hasCollateral(borrower));
        assertEq(settlement.auctionStartBlock(borrower), 0);
        assertLt(pool.poolState().reserve, reserveBefore);
    }

    function test_shortfallBeyondTheReserveIsRecordedAsDeficit() public {
        wethFeed.setAnswer(1e6);
        startAuctionAndWait(borrower, AUCTION_DURATION);
        assertEq(pool.poolState().reserve, 0);

        liquidate(borrower, 1e6, 0);

        assertEq(pool.getDebt(borrower), 0);
        assertGt(pool.poolState().deficit, 0);
    }

    function test_settleBadDebtNeedsEmptyCollateral() public {
        vm.expectRevert(LiquidationSettlement.CollateralRemaining.selector);
        settlement.settleBadDebt(borrower);
    }

    function test_settleBadDebtNeedsDebt() public {
        address nobody = makeAddr("nobody");
        vm.expectRevert(LiquidationSettlement.NoDebt.selector);
        settlement.settleBadDebt(nobody);
    }

    function test_cannotLiquidateAHealthyPositionWithAStaleAuction() public {
        _crash();
        settlement.startAuction(borrower);
        setWethPrice(2000);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory signature = signIntent(intent);
        vm.expectRevert(LiquidationSettlement.PositionHealthy.selector);
        settlement.executeLiquidation(intent, signature);
    }
}
