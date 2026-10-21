// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "./Base.t.sol";
import {VigilPool} from "../src/core/VigilPool.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";
import {OracleAdapter} from "../src/oracle/OracleAdapter.sol";
import {MockAMM} from "../src/oracle/MockAMM.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {ITwapSource} from "../src/interfaces/ITwapSource.sol";

/// @notice Follows the money around the oracle: it and the AMM hold nothing, and a frozen oracle stops funds moving.
contract OracleFlowTest is VigilBase {
    MockAMM internal amm;
    address internal victim = makeAddr("victim");

    function setUp() public override {
        super.setUp();

        amm = new MockAMM(1000e18, 2_000_000e18);
        OracleAdapter.Source memory source = plainSource(wethFeed);
        source.twapSource = ITwapSource(address(amm));
        source.twapWindow = 1 hours;
        source.maxDeviationBps = 500;
        oracle.setSource(address(weth), source);

        vm.warp(block.timestamp + 1 hours);
        _refreshFeeds(2000);

        // $20,000 of WETH against $12,000 of debt
        openPosition(victim, 10 ether, 12_000e6);
    }

    function _refreshFeeds(uint256 wethUsd) internal {
        setWethPrice(wethUsd);
        refreshUsdcFeed();
    }

    function _frozen() internal view returns (bool) {
        return oracle.state() == IOracleAdapter.State.Frozen;
    }

    /// @dev Every balance that matters, hashed, so a test can say "no funds moved at all".
    function _balances() internal view returns (bytes32) {
        return keccak256(
            abi.encode(
                usdc.balanceOf(address(pool)),
                weth.balanceOf(address(pool)),
                usdc.balanceOf(victim),
                weth.balanceOf(victim),
                usdc.balanceOf(lender),
                usdc.balanceOf(liquidator),
                weth.balanceOf(liquidator),
                usdc.balanceOf(address(settlement)),
                weth.balanceOf(address(settlement)),
                pool.getCollateralBalance(victim, address(weth)),
                pool.getDebt(victim)
            )
        );
    }

    /// @dev Moves the AMM and the feed to the same new price and lets the TWAP catch up, the oracle stays normal.
    function _moveMarketTo(uint256 wethUsd) internal {
        amm.setReserves(1000e18, 1000e18 * wethUsd);
        vm.warp(block.timestamp + 1 hours);
        _refreshFeeds(wethUsd);
        assertFalse(_frozen());
    }

    /// @dev Pushes the AMM 20% away from the feed so the TWAP cross-check freezes the oracle.
    function _freezeByTwapDeviation() internal {
        amm.setReserves(1000e18, 2_400_000e18);
        vm.warp(block.timestamp + 1 hours);
        _refreshFeeds(2000);
        assertTrue(_frozen());
    }

    // ---------------------------------------------------------------------
    // Nothing is held, nothing is moved
    // ---------------------------------------------------------------------

    function test_oracleAndAmmHoldNoTokens() public view {
        assertEq(usdc.balanceOf(address(oracle)), 0);
        assertEq(weth.balanceOf(address(oracle)), 0);
        assertEq(usdc.balanceOf(address(amm)), 0);
        assertEq(weth.balanceOf(address(amm)), 0);
        assertEq(address(oracle).balance, 0);
        assertEq(address(amm).balance, 0);
    }

    function test_priceReadsMoveNoFunds() public view {
        // views cannot change state, this documents that a read is also all the oracle ever does
        assertEq(oracle.getPrice(address(weth)), 2000e18);
        assertEq(oracle.getPrice(address(usdc)), 1e18);
        assertEq(usdc.balanceOf(address(oracle)), 0);
    }

    function test_ammSwapsAreVirtualAndMoveNoTokens() public {
        bytes32 before = _balances();
        uint256 usdcSupply = usdc.totalSupply();
        uint256 wethSupply = weth.totalSupply();

        amm.swapBaseForQuote(900e18);
        amm.swapQuoteForBase(100_000e18);

        // the reserves moved, the ERC20 balances did not
        assertEq(_balances(), before);
        assertEq(usdc.totalSupply(), usdcSupply);
        assertEq(weth.totalSupply(), wethSupply);
        assertEq(usdc.balanceOf(address(amm)), 0);
        assertEq(weth.balanceOf(address(amm)), 0);
    }

    function test_theAttackerEndsWithNoTokensFromTheDump() public {
        address attackerEoa = makeAddr("attackerEoa");
        vm.startPrank(attackerEoa);
        amm.swapBaseForQuote(900e18);
        vm.stopPrank();

        assertEq(usdc.balanceOf(attackerEoa), 0);
        assertEq(weth.balanceOf(attackerEoa), 0);
    }

    function testFuzz_sameBlockDumpNeverChangesPriceOrOracleState(uint96 dumpSeed) public {
        uint256 dump = bound(dumpSeed, 1e18, 100_000e18);
        uint256 priceBefore = oracle.getPrice(address(weth));
        bytes32 before = _balances();

        amm.swapBaseForQuote(dump);

        assertFalse(_frozen());
        assertEq(oracle.getPrice(address(weth)), priceBefore);
        assertEq(_balances(), before);
    }

    // ---------------------------------------------------------------------
    // A frozen oracle stops funds leaving
    // ---------------------------------------------------------------------

    function test_frozenOracleStopsEveryOutflowAndNoBalanceChanges() public {
        _freezeByTwapDeviation();
        bytes32 before = _balances();

        vm.startPrank(victim);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.borrow(1e6);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.withdrawCollateral(address(weth), 1 ether);
        vm.stopPrank();

        vm.prank(lender);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.withdrawSupply(1e6);

        assertEq(_balances(), before, "no funds moved");
    }

    function test_frozenOracleStopsLiquidationAndNoFundsMove() public {
        _moveMarketTo(1200);
        settlement.startAuction(victim);
        vm.roll(block.number + AUCTION_DURATION);

        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(victim, 5000e6, 0);
        bytes memory signature = signIntent(intent);

        wethFeed.setUpdatedAt(0);
        assertTrue(_frozen());
        bytes32 before = _balances();

        vm.expectRevert(LiquidationSettlement.OracleFrozen.selector);
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before, "liquidator paid nothing and received nothing");
        assertEq(weth.balanceOf(liquidator), 0);
    }

    function test_frozenOracleBlocksNewAuctions() public {
        _moveMarketTo(1200);
        wethFeed.setUpdatedAt(0);

        vm.expectRevert(LiquidationSettlement.OracleFrozen.selector);
        settlement.startAuction(victim);
        assertEq(settlement.auctionStartBlock(victim), 0);
    }

    function test_frozenOracleStillLetsRepaymentFundsIn() public {
        _freezeByTwapDeviation();

        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));
        uint256 debtBefore = pool.getDebt(victim);

        uint256 victimBefore = usdc.balanceOf(victim);
        vm.startPrank(victim);
        usdc.approve(address(pool), type(uint256).max);
        pool.repay(500e6);
        vm.stopPrank();

        assertEq(usdc.balanceOf(address(pool)) - poolUsdcBefore, 500e6, "pool received the repayment");
        assertEq(victimBefore - usdc.balanceOf(victim), 500e6, "victim paid it");
        assertEq(debtBefore - pool.getDebt(victim), 500e6, "debt fell by the same amount");
    }

    function test_frozenOracleStillLetsCollateralIn() public {
        _freezeByTwapDeviation();

        uint256 poolWethBefore = weth.balanceOf(address(pool));
        weth.mint(victim, 2 ether);
        vm.startPrank(victim);
        weth.approve(address(pool), type(uint256).max);
        pool.depositCollateral(address(weth), 2 ether);
        vm.stopPrank();

        assertEq(weth.balanceOf(address(pool)) - poolWethBefore, 2 ether);
        assertEq(pool.getCollateralBalance(victim, address(weth)), 12 ether);
    }

    function test_interestKeepsGrowingWhileFrozen() public {
        uint256 debtBefore = pool.getDebt(victim);

        // feeds go stale, so the oracle freezes on its own
        vm.warp(block.timestamp + 30 days);
        assertTrue(_frozen());

        uint256 debtFrozen = pool.getDebt(victim);
        assertGt(debtFrozen, debtBefore, "a frozen oracle does not stop the meter");

        // a repayment settles the interest into the reserve even though the oracle is frozen
        usdc.mint(victim, 1e6);
        vm.startPrank(victim);
        usdc.approve(address(pool), type(uint256).max);
        pool.repay(1);
        vm.stopPrank();
        assertGt(pool.poolState().reserve, 0);
    }

    function test_frozenOracleDoesNotLoseOrCreateTokensWhenItRecovers() public {
        _freezeByTwapDeviation();
        uint256 usdcSupply = usdc.totalSupply();
        uint256 wethSupply = weth.totalSupply();

        // the feed catches up with the AMM, the oracle recovers
        _refreshFeeds(2400);
        assertFalse(_frozen());

        assertEq(usdc.totalSupply(), usdcSupply);
        assertEq(weth.totalSupply(), wethSupply);
    }

    function test_borrowedFundsFlowAgainOnceTheOracleRecovers() public {
        _freezeByTwapDeviation();
        vm.prank(victim);
        vm.expectRevert(VigilPool.OracleFrozen.selector);
        pool.borrow(1000e6);

        _refreshFeeds(2400);
        assertFalse(_frozen());

        uint256 poolBefore = usdc.balanceOf(address(pool));
        uint256 victimBefore = usdc.balanceOf(victim);
        vm.prank(victim);
        pool.borrow(1000e6);

        assertEq(usdc.balanceOf(victim) - victimBefore, 1000e6, "victim received the loan");
        assertEq(poolBefore - usdc.balanceOf(address(pool)), 1000e6, "it came from the pool");
    }

    // ---------------------------------------------------------------------
    // Which price decides how much moves
    // ---------------------------------------------------------------------

    function test_liquidationPayoutFollowsTheFeedNotTheAmmSpot() public {
        _moveMarketTo(1200);
        startAuctionAndWait(victim, AUCTION_DURATION);

        uint256 debtBefore = pool.getDebt(victim);

        // dump the spot in the same block, the TWAP and so the oracle ignore it
        amm.swapBaseForQuote(900e18);
        assertFalse(_frozen());

        liquidate(victim, 6000e6, 0);

        // $6000 plus the full 10% bonus at the feed price of $1200
        uint256 value = 6600e6;
        assertEq(weth.balanceOf(liquidator), (value * 1e12 * 1e18) / 1200e18);
        // interest built up during the warps, so the debt is the original minus the repayment, plus that interest
        assertEq(pool.getDebt(victim), debtBefore - 6000e6);
    }

    function test_liquidationFundsMatchEvenWhenTheTwapSourceIsAttached() public {
        _moveMarketTo(1200);
        startAuctionAndWait(victim, AUCTION_DURATION);

        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));
        uint256 liquidatorUsdcBefore = usdc.balanceOf(liquidator);
        uint256 victimCollateralBefore = pool.getCollateralBalance(victim, address(weth));

        liquidate(victim, 6000e6, 0);

        assertEq(usdc.balanceOf(address(pool)) - poolUsdcBefore, 6000e6);
        assertEq(liquidatorUsdcBefore - usdc.balanceOf(liquidator), 6000e6);
        assertEq(victimCollateralBefore - pool.getCollateralBalance(victim, address(weth)), weth.balanceOf(liquidator));
        assertEq(usdc.balanceOf(address(amm)), 0);
        assertEq(weth.balanceOf(address(amm)), 0);
    }

    function test_twapAverageIsWhatTheOracleComparesAgainst() public {
        // 1 hour at $2000 then 1 hour at $2300, the 1 hour window only sees $2300
        amm.setReserves(1000e18, 2_300_000e18);
        vm.warp(block.timestamp + 1 hours);
        assertEq(amm.consult(1 hours), 2300e18);
        assertEq(amm.consult(2 hours), 2150e18);

        // feed at $2000 is 13% away from the $2300 TWAP, more than the 5% limit
        _refreshFeeds(2000);
        assertTrue(_frozen());

        // feed at $2200 is inside 5% of $2300
        _refreshFeeds(2200);
        assertFalse(_frozen());
        assertEq(oracle.getPrice(address(weth)), 2200e18);
    }
}
