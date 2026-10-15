// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "./Base.t.sol";
import {VigilPool} from "../src/core/VigilPool.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";
import {OracleAdapter} from "../src/oracle/OracleAdapter.sol";
import {MockAMM} from "../src/oracle/MockAMM.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {IPriceFeed} from "../src/interfaces/IPriceFeed.sol";
import {ITwapSource} from "../src/interfaces/ITwapSource.sol";

contract OracleTest is VigilBase {
    function _frozen() internal view returns (bool) {
        return oracle.state() == IOracleAdapter.State.Frozen;
    }

    function test_normalPrice() public view {
        assertEq(oracle.getPrice(address(weth)), 2000e18);
        assertEq(oracle.getPrice(address(usdc)), 1e18);
        assertFalse(_frozen());
    }

    // SC-14: one test per failure mode
    function test_staleFeedFreezes() public {
        vm.warp(block.timestamp + 1 days + 1);
        assertTrue(_frozen());

        vm.expectRevert(abi.encodeWithSelector(OracleAdapter.InvalidPrice.selector, address(weth)));
        oracle.getPrice(address(weth));
    }

    function test_zeroAnswerFreezes() public {
        wethFeed.setAnswer(0);
        assertTrue(_frozen());
        vm.expectRevert(abi.encodeWithSelector(OracleAdapter.InvalidPrice.selector, address(weth)));
        oracle.getPrice(address(weth));
    }

    function test_negativeAnswerFreezes() public {
        wethFeed.setAnswer(-5);
        assertTrue(_frozen());
    }

    function test_answerBelowMinFreezes() public {
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.minAnswer = 1000e8;
        oracle.setSource(address(weth), source);

        wethFeed.setAnswer(999e8);
        assertTrue(_frozen());
        wethFeed.setAnswer(1000e8);
        assertFalse(_frozen());
    }

    function test_answerAboveMaxFreezes() public {
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.maxAnswer = 5000e8;
        oracle.setSource(address(weth), source);

        wethFeed.setAnswer(5001e8);
        assertTrue(_frozen());
    }

    function test_oneBadAssetFreezesTheWholeOracle() public {
        usdcFeed.setAnswer(0);
        assertTrue(_frozen());
        // the other asset still reads fine on its own
        assertEq(oracle.getPrice(address(weth)), 2000e18);
    }

    function test_unknownAssetHasNoPrice() public {
        vm.expectRevert(abi.encodeWithSelector(OracleAdapter.InvalidPrice.selector, address(0xBEEF)));
        oracle.getPrice(address(0xBEEF));
    }

    function test_onlyOwnerSetsSources() public {
        OracleAdapter.Source memory source = plainSource(wethFeed);
        vm.prank(borrower);
        vm.expectRevert();
        oracle.setSource(address(weth), source);
    }

    function test_sourceNeedsAFeedAndSaneBounds() public {
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.feed = IPriceFeed(address(0));
        vm.expectRevert(OracleAdapter.InvalidSource.selector);
        oracle.setSource(address(weth), source);

        source = plainSource(wethFeed);
        source.minAnswer = 10;
        source.maxAnswer = 5;
        vm.expectRevert(OracleAdapter.InvalidSource.selector);
        oracle.setSource(address(weth), source);
    }

    // SC-15: a TWAP over a window matches the expected average
    function test_twapMatchesExpectedAverage() public {
        MockAMM amm = new MockAMM(1e18, 2000e18);

        vm.warp(block.timestamp + 1 hours);
        amm.setReserves(1e18, 3000e18);
        vm.warp(block.timestamp + 1 hours);

        // 1 hour at $2000 then 1 hour at $3000
        assertEq(amm.consult(2 hours), 2500e18);
        assertEq(amm.consult(1 hours), 3000e18);
    }

    function test_twapNeedsHistory() public {
        MockAMM amm = new MockAMM(1e18, 2000e18);
        vm.expectRevert(MockAMM.InsufficientHistory.selector);
        amm.consult(1 hours);
    }

    function test_swapInTheSameBlockDoesNotMoveTheTwap() public {
        MockAMM amm = new MockAMM(1e18, 2000e18);
        vm.warp(block.timestamp + 1 hours);
        uint256 before = amm.consult(1 hours);

        amm.swapQuoteForBase(1_000_000e18);
        assertGt(amm.spotPrice(), 100_000e18);
        assertEq(amm.consult(1 hours), before);
    }

    // SC-16: a TWAP that disagrees with the feed freezes the oracle
    function test_twapDeviationFreezes() public {
        MockAMM amm = new MockAMM(1e18, 2000e18);
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.twapSource = ITwapSource(address(amm));
        source.twapWindow = 1 hours;
        source.maxDeviationBps = 500;
        oracle.setSource(address(weth), source);

        vm.warp(block.timestamp + 1 hours);
        wethFeed.setAnswer(2000e8);
        usdcFeed.setAnswer(1e8);
        assertFalse(_frozen());

        // the AMM drifts 20% away from the feed
        amm.setReserves(1e18, 2400e18);
        vm.warp(block.timestamp + 1 hours);
        wethFeed.setAnswer(2000e8);
        usdcFeed.setAnswer(1e8);
        assertTrue(_frozen());

        // the feed catches up
        wethFeed.setAnswer(2400e8);
        assertFalse(_frozen());
    }

    function test_twapWithoutHistoryFreezes() public {
        MockAMM amm = new MockAMM(1e18, 2000e18);
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.twapSource = ITwapSource(address(amm));
        source.twapWindow = 1 hours;
        source.maxDeviationBps = 500;
        oracle.setSource(address(weth), source);

        assertTrue(_frozen());
    }

    // SC-16: while frozen only repay and add collateral are allowed
    function test_frozenOracleBlocksRiskyActions() public {
        openPosition(borrower, 10 ether, 1000e6);
        wethFeed.setUpdatedAt(0);
        assertTrue(_frozen());

        vm.startPrank(borrower);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.borrow(1e6);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.withdrawCollateral(address(weth), 1 ether);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.setCollateralEnabled(address(weth), false);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.supply(1e6);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.withdrawSupply(1e6);
        vm.stopPrank();
    }

    function test_frozenOracleStillAllowsRepayAndAddCollateral() public {
        openPosition(borrower, 10 ether, 1000e6);
        wethFeed.setUpdatedAt(0);

        usdc.mint(borrower, 500e6);
        weth.mint(borrower, 1 ether);
        vm.startPrank(borrower);
        usdc.approve(address(pool), type(uint256).max);
        weth.approve(address(pool), type(uint256).max);
        pool.repay(500e6);
        pool.depositCollateral(address(weth), 1 ether);
        vm.stopPrank();

        assertEq(pool.getCollateralBalance(borrower, address(weth)), 11 ether);
    }

    function test_frozenOracleBlocksLiquidation() public {
        openPosition(borrower, 10 ether, 15_000e6);
        setWethPrice(1500);
        settlement.startAuction(borrower);
        vm.roll(block.number + 10);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);

        wethFeed.setUpdatedAt(0);
        vm.expectRevert(LiquidationSettlement.OracleFrozen.selector);
        settlement.executeLiquidation(intent, signature);
    }
}
