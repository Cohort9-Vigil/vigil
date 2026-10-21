// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Vm} from "forge-std/Vm.sol";
import {VigilBase} from "./Base.t.sol";
import {VigilPool} from "../src/core/VigilPool.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";

/// @notice Follows the money: who pays, who receives and where the bonus and the reserve come from.
contract FundsFlowTest is VigilBase {
    // 10 WETH at $2000 backing $10,000 of debt
    function setUp() public override {
        super.setUp();
        openPosition(borrower, 10 ether, 10_000e6);
    }

    function _crash() internal {
        setWethPrice(1200);
    }

    /// @dev Lets interest build up and settles it into the indexes, with fresh feeds.
    function _accrueFor(uint256 duration) internal {
        vm.warp(block.timestamp + duration);
        setWethPrice(2000);
        refreshUsdcFeed();
        vm.prank(lender);
        pool.withdrawSupply(1);
    }

    function _lenderClaim() internal view returns (uint256) {
        VigilPool.PoolState memory state = pool.poolState();
        return state.totalScaledSupply * state.liquidityIndex / RAY;
    }

    // ---------------------------------------------------------------------
    // Liquidation bonus
    // ---------------------------------------------------------------------

    function test_bonusIsPaidInCollateralTakenFromTheBorrower() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        uint256 borrowerCollateralBefore = pool.getCollateralBalance(borrower, address(weth));
        uint256 poolWethBefore = weth.balanceOf(address(pool));

        liquidate(borrower, 5000e6, 0);

        // $5000 plus the 10% bonus is $5500, at $1200 a WETH
        uint256 value = 5500e6;
        uint256 expected = (value * 1e12 * 1e18) / 1200e18;
        uint256 received = weth.balanceOf(liquidator);
        assertEq(received, expected, "liquidator got repayment plus bonus");

        // Exactly what the liquidator got left the borrower's position and the pool.
        assertEq(borrowerCollateralBefore - pool.getCollateralBalance(borrower, address(weth)), received);
        assertEq(poolWethBefore - weth.balanceOf(address(pool)), received);
    }

    function test_bonusIsNotPaidByLendersTheReserveOrTheOwner() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        VigilPool.PoolState memory before = pool.poolState();
        uint256 lenderClaimBefore = _lenderClaim();

        liquidate(borrower, 5000e6, 0);

        VigilPool.PoolState memory afterState = pool.poolState();
        assertEq(afterState.reserve, before.reserve, "reserve untouched");
        assertEq(afterState.deficit, before.deficit, "no deficit created");
        assertEq(afterState.liquidityIndex, before.liquidityIndex, "supplier index untouched");
        assertEq(_lenderClaim(), lenderClaimBefore, "lender claim untouched");
        assertEq(weth.balanceOf(address(this)), 0, "owner got nothing");
        assertEq(weth.balanceOf(lender), 0, "lender got no collateral");
    }

    function test_liquidationMovesNoTokensToSettlementOrOwner() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        liquidate(borrower, 5000e6, 0);

        assertEq(usdc.balanceOf(address(settlement)), 0, "settlement holds no USDC");
        assertEq(weth.balanceOf(address(settlement)), 0, "settlement holds no WETH");
        assertEq(usdc.balanceOf(address(this)), 0, "owner got no USDC");
        assertEq(weth.balanceOf(address(this)), 0, "owner got no WETH");
        assertEq(usdc.balanceOf(address(oracle)), 0);
        assertEq(weth.balanceOf(address(oracle)), 0);
    }

    function test_repaymentGoesStraightToThePool() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));
        uint256 liquidatorUsdcBefore = usdc.balanceOf(liquidator);

        liquidate(borrower, 5000e6, 0);

        assertEq(usdc.balanceOf(address(pool)) - poolUsdcBefore, 5000e6, "pool received the repayment");
        assertEq(liquidatorUsdcBefore - usdc.balanceOf(liquidator), 5000e6, "liquidator paid it");
        assertEq(pool.getDebt(borrower), 5000e6, "debt fell by the same amount");
    }

    function test_noTokensAreCreatedOrDestroyedByALiquidation() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION);

        uint256 usdcTotalBefore = usdc.totalSupply();
        uint256 wethTotalBefore = weth.totalSupply();

        liquidate(borrower, 5000e6, 0);

        assertEq(usdc.totalSupply(), usdcTotalBefore);
        assertEq(weth.totalSupply(), wethTotalBefore);
        // Every WETH is with the pool or the liquidator.
        assertEq(weth.balanceOf(address(pool)) + weth.balanceOf(liquidator), wethTotalBefore);
    }

    function test_liquidationEventReportsTheBonusAndAmounts() public {
        _crash();
        startAuctionAndWait(borrower, AUCTION_DURATION / 2);

        // half way through the auction the bonus is 5%, so $5000 buys $5250 of WETH
        uint256 value = 5250e6;
        uint256 seized = (value * 1e12 * 1e18) / 1200e18;

        vm.expectEmit(true, true, true, true, address(settlement));
        emit LiquidationSettlement.LiquidationExecuted(
            borrower, liquidator, address(weth), 5000e6, seized, MAX_BONUS_BPS / 2
        );
        liquidate(borrower, 5000e6, 0);
    }

    function test_bonusIsZeroWhenTheAuctionJustStarted() public {
        _crash();
        settlement.startAuction(borrower);

        liquidate(borrower, 1200e6, 0);

        // $1200 at $1200 a WETH is exactly 1 WETH, nothing extra
        assertEq(weth.balanceOf(liquidator), 1 ether);
    }

    function testFuzz_collateralSeizedMatchesTheBonusAtThatBlock(uint8 wait) public {
        _crash();
        startAuctionAndWait(borrower, wait);

        uint256 bonusBps = settlement.currentBonusBps(borrower);
        uint256 repay = 5000e6;
        uint256 expected = (repay * (10_000 + bonusBps) / 10_000) * 1e12 * 1e18 / 1200e18;
        // the contract rounds the whole product down once, allow the difference from rounding twice
        liquidate(borrower, 5000e6, 0);

        assertApproxEqAbs(weth.balanceOf(liquidator), expected, 1e12);
        assertLe(weth.balanceOf(liquidator), (repay * 11 / 10) * 1e12 * 1e18 / 1200e18, "never above the max bonus");
        assertGe(weth.balanceOf(liquidator), repay * 1e12 * 1e18 / 1200e18, "never below the repayment value");
    }

    function test_bonusNeverExceedsTheCollateralTheBorrowerHas() public {
        // collateral worth far less than the debt, the bonus cannot conjure more than what is there
        wethFeed.setAnswer(1e6);
        startAuctionAndWait(borrower, AUCTION_DURATION);

        liquidate(borrower, 1e6, 0);

        assertEq(weth.balanceOf(liquidator), 10 ether, "liquidator takes all there is");
        assertEq(pool.getCollateralBalance(borrower, address(weth)), 0);
        assertEq(weth.balanceOf(address(pool)), 0, "pool holds none of it any more");
    }

    // ---------------------------------------------------------------------
    // Reserve
    // ---------------------------------------------------------------------

    function test_interestSplitsBetweenReserveAndLenders() public {
        uint256 claimBefore = _lenderClaim();
        _accrueFor(365 days);

        // Only one borrower, so their debt growth is all the interest of the pool.
        uint256 interest = pool.getDebt(borrower) - 10_000e6;
        assertGt(interest, 0);

        uint256 reserve = pool.poolState().reserve;
        uint256 lenderGain = _lenderClaim() - claimBefore;

        // reserveFactor is 10%
        assertApproxEqAbs(reserve, interest / 10, 3, "reserve gets 10%");
        // withdrawSupply(1) took 1 base unit out of the claim
        assertApproxEqAbs(lenderGain + 1, interest - reserve, 3, "lenders get the other 90%");
        assertApproxEqAbs(reserve + lenderGain + 1, interest, 4, "nothing lost or created");
    }

    function test_accruingInterestMovesNoTokens() public {
        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));
        uint256 lenderUsdcBefore = usdc.balanceOf(lender);

        _accrueFor(365 days);

        // Only the 1 unit the lender withdrew to trigger the accrual moved.
        assertEq(poolUsdcBefore - usdc.balanceOf(address(pool)), 1);
        assertEq(usdc.balanceOf(lender) - lenderUsdcBefore, 1);
        assertEq(usdc.balanceOf(address(this)), 0, "owner holds no reserve tokens");
        assertEq(usdc.balanceOf(address(settlement)), 0);
        assertGt(pool.poolState().reserve, 0);
    }

    function test_reserveStaysInsideThePoolBalance() public {
        _accrueFor(365 days);

        assertGe(usdc.balanceOf(address(pool)), pool.poolState().reserve, "reserve is backed by real tokens");
    }

    function test_reserveCannotBeBorrowedOut() public {
        _accrueFor(365 days);
        uint256 reserve = pool.poolState().reserve;
        assertGt(reserve, 0);

        address whale = makeAddr("whale");
        openPosition(whale, 1000 ether, 0);

        uint256 lendable = usdc.balanceOf(address(pool)) - reserve;

        vm.startPrank(whale);
        vm.expectRevert(VigilPool.InsufficientLiquidity.selector);
        pool.borrow(lendable + 1);
        pool.borrow(lendable);
        vm.stopPrank();

        // The reserve is still in the pool afterwards.
        assertEq(usdc.balanceOf(address(pool)), reserve);
    }

    function test_lenderCannotWithdrawTheReserve() public {
        _accrueFor(365 days);
        uint256 reserve = pool.poolState().reserve;

        address whale = makeAddr("whale");
        openPosition(whale, 1000 ether, 0);
        uint256 lendable = usdc.balanceOf(address(pool)) - reserve;
        vm.prank(whale);
        pool.borrow(lendable);

        // Pool cash is now exactly the reserve, so a lender cannot pull any of it.
        vm.prank(lender);
        vm.expectRevert(VigilPool.InsufficientLiquidity.selector);
        pool.withdrawSupply(1);
    }

    function test_reserveDoesNotGrowWithoutInterest() public {
        // Nothing borrowed against this pool state: repay everything and let time pass.
        usdc.mint(borrower, 1_000e6);
        vm.startPrank(borrower);
        usdc.approve(address(pool), type(uint256).max);
        pool.repay(type(uint256).max);
        vm.stopPrank();
        assertEq(pool.getDebt(borrower), 0);

        uint256 reserveBefore = pool.poolState().reserve;
        _accrueFor(365 days);

        assertEq(pool.poolState().reserve, reserveBefore, "no debt, no interest, no reserve");
    }

    function test_badDebtDrainsTheReserveThenRecordsDeficit() public {
        _accrueFor(365 days);
        uint256 reserveBefore = pool.poolState().reserve;
        assertGt(reserveBefore, 0);

        wethFeed.setAnswer(1e6);
        startAuctionAndWait(borrower, AUCTION_DURATION);

        uint256 debtBefore = pool.getDebt(borrower);
        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));

        vm.recordLogs();
        liquidate(borrower, 1e6, 0);
        (uint256 covered, uint256 deficit) = _readBadDebtEvent();

        // The liquidator repaid 1e6, the rest was written off.
        uint256 writtenOff = debtBefore - 1e6;
        assertEq(covered, reserveBefore, "the whole reserve was used first");
        assertApproxEqAbs(covered + deficit, writtenOff, 2, "reserve plus deficit cover the remaining debt");
        assertEq(pool.poolState().reserve, 0, "reserve is empty");
        assertEq(pool.poolState().deficit, deficit, "deficit recorded in pool state");

        // A write-off is bookkeeping, no tokens move except the liquidator's repayment.
        assertEq(usdc.balanceOf(address(pool)) - poolUsdcBefore, 1e6);
    }

    function test_reserveCoversASmallLossWithoutDeficit() public {
        _accrueFor(365 days);
        uint256 reserveBefore = pool.poolState().reserve;

        // Collateral is worth less than the debt plus bonus, so the liquidator takes all of it. Leave a remainder
        // of half the reserve unpaid, which the reserve can absorb in full.
        setWethPrice(1005);
        startAuctionAndWait(borrower, AUCTION_DURATION);
        uint256 debtBefore = pool.getDebt(borrower);
        uint256 remainder = reserveBefore / 2;

        vm.recordLogs();
        liquidate(borrower, debtBefore - remainder, 0);
        (uint256 covered, uint256 deficit) = _readBadDebtEvent();

        assertApproxEqAbs(covered, remainder, 2, "covered the unpaid remainder");
        assertLt(covered, reserveBefore, "only part of the reserve was needed");
        assertEq(deficit, 0, "no deficit");
        assertEq(pool.poolState().reserve, reserveBefore - covered, "reserve fell by exactly what was covered");
        assertEq(pool.getDebt(borrower), 0);
    }

    function test_settleBadDebtDoesNotChangeTokenBalances() public {
        wethFeed.setAnswer(1e6);
        startAuctionAndWait(borrower, AUCTION_DURATION);
        liquidate(borrower, 1e6, 0);

        // position is already cleared, check the write-off left the books consistent
        VigilPool.PoolState memory state = pool.poolState();
        assertEq(state.totalScaledDebt, 0, "no debt left in the pool");
        assertEq(state.reserve, 0);
        assertGt(state.deficit, 0);
    }

    function test_deficitIsNotPaidFromLenderBalances() public {
        wethFeed.setAnswer(1e6);
        startAuctionAndWait(borrower, AUCTION_DURATION);
        uint256 claimBefore = _lenderClaim();

        liquidate(borrower, 1e6, 0);

        // The loss is only recorded, the lender's claim is not silently cut.
        assertEq(_lenderClaim(), claimBefore);
        assertGt(pool.poolState().deficit, 0);
    }

    function _readBadDebtEvent() internal returns (uint256 covered, uint256 deficit) {
        Vm.Log[] memory logs = vm.getRecordedLogs();
        bytes32 topic = keccak256("BadDebtWrittenOff(address,uint256,uint256)");
        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].emitter == address(pool) && logs[i].topics[0] == topic) {
                return abi.decode(logs[i].data, (uint256, uint256));
            }
        }
        revert("BadDebtWrittenOff not emitted");
    }
}
