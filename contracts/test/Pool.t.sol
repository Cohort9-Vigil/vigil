// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";

import {VigilBase} from "./Base.t.sol";
import {VigilPool} from "../src/core/VigilPool.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {InterestRateModel} from "../src/libraries/InterestRateModel.sol";

contract PoolTest is VigilBase {
    // SC-01
    function test_implementationCannotBeInitialized() public {
        VigilPool implementation = new VigilPool();
        InterestRateModel.RateParams memory rate = InterestRateModel.RateParams(0.02e27, 0.04e27, 0.75e27, 0.8e27);

        vm.expectRevert(Initializable.InvalidInitialization.selector);
        implementation.initialize(address(this), address(usdc), IOracleAdapter(address(oracle)), 1000, rate);
    }

    function test_proxyCannotBeInitializedTwice() public {
        InterestRateModel.RateParams memory rate = InterestRateModel.RateParams(0.02e27, 0.04e27, 0.75e27, 0.8e27);

        vm.expectRevert(Initializable.InvalidInitialization.selector);
        pool.initialize(address(this), address(usdc), IOracleAdapter(address(oracle)), 1000, rate);
    }

    // SC-04
    function test_pauseBlocksBorrowing() public {
        openPosition(borrower, 10 ether, 0);
        pool.pause();

        vm.prank(borrower);
        vm.expectRevert(VigilPool.PoolPaused.selector);
        pool.borrow(1000e6);

        pool.unpause();
        vm.prank(borrower);
        pool.borrow(1000e6);
    }

    function test_unpauseNeedsNormalOracle() public {
        pool.pause();
        wethFeed.setUpdatedAt(0);

        vm.expectRevert(VigilPool.OracleNotNormal.selector);
        pool.unpause();

        wethFeed.setAnswer(2000e8);
        pool.unpause();
        assertFalse(pool.poolState().paused);
    }

    function test_onlyOwnerCanPause() public {
        vm.prank(borrower);
        vm.expectRevert();
        pool.pause();
    }

    function test_repayStillWorksWhilePaused() public {
        openPosition(borrower, 10 ether, 1000e6);
        pool.pause();

        usdc.mint(borrower, 1000e6);
        vm.startPrank(borrower);
        usdc.approve(address(pool), type(uint256).max);
        pool.repay(1000e6);
        vm.stopPrank();

        assertEq(pool.getDebt(borrower), 0);
    }

    function test_supplyAndWithdraw() public {
        (, uint256 scaledSupply,) = pool.positionOf(lender);
        assertEq(scaledSupply, 1_000_000e6 * 1e27 / 1e27);

        vm.prank(lender);
        pool.withdrawSupply(400_000e6);
        assertEq(usdc.balanceOf(lender), 400_000e6);
    }

    function test_withdrawMoreThanSuppliedReverts() public {
        vm.prank(lender);
        vm.expectRevert(VigilPool.InsufficientBalance.selector);
        pool.withdrawSupply(1_000_001e6);
    }

    function test_cannotWithdrawLiquidityThatIsLentOut() public {
        openPosition(borrower, 100 ether, 150_000e6);

        vm.prank(lender);
        vm.expectRevert(VigilPool.InsufficientLiquidity.selector);
        pool.withdrawSupply(900_000e6);
    }

    // SC-05
    function test_interestRaisesIndexesAndDebt() public {
        openPosition(borrower, 100 ether, 100_000e6);
        uint256 debtBefore = pool.getDebt(borrower);
        uint256 supplyIndexBefore = pool.poolState().liquidityIndex;

        vm.warp(block.timestamp + 365 days);
        setWethPrice(2000);
        usdcFeed.setAnswer(1e8);

        assertGt(pool.getDebt(borrower), debtBefore);

        vm.prank(lender);
        pool.withdrawSupply(1e6);
        assertGt(pool.poolState().borrowIndex, RAY);
        assertGt(pool.poolState().liquidityIndex, supplyIndexBefore);
    }

    function test_reserveGetsItsShareOfInterest() public {
        openPosition(borrower, 100 ether, 100_000e6);
        vm.warp(block.timestamp + 365 days);
        setWethPrice(2000);
        usdcFeed.setAnswer(1e8);

        vm.prank(lender);
        pool.withdrawSupply(1e6);

        assertGt(pool.poolState().reserve, 0);
    }

    function test_previewMatchesAccrual() public {
        openPosition(borrower, 100 ether, 100_000e6);
        vm.warp(block.timestamp + 30 days);
        setWethPrice(2000);
        usdcFeed.setAnswer(1e8);

        uint256 previewed = pool.getDebt(borrower);
        usdc.mint(borrower, 1e6);
        vm.startPrank(borrower);
        usdc.approve(address(pool), type(uint256).max);
        pool.repay(1);
        vm.stopPrank();

        assertApproxEqAbs(pool.getDebt(borrower), previewed, 1);
    }

    // SC-07: debt rounds up
    function test_debtNeverBelowBorrowed() public {
        openPosition(borrower, 100 ether, 0);
        vm.warp(block.timestamp + 100 days);
        setWethPrice(2000);
        usdcFeed.setAnswer(1e8);

        openPosition(makeAddr("other"), 100 ether, 50_000e6);
        vm.warp(block.timestamp + 100 days);
        setWethPrice(2000);
        usdcFeed.setAnswer(1e8);

        vm.prank(borrower);
        pool.borrow(7);
        assertGe(pool.getDebt(borrower), 7);
    }

    // SC-10
    function test_borrowAboveHealthLimitReverts() public {
        openPosition(borrower, 10 ether, 0);

        // 10 WETH * $2000 * 80% = $16,000 of borrowing power
        vm.prank(borrower);
        vm.expectRevert(VigilPool.HealthFactorTooLow.selector);
        pool.borrow(16_001e6);

        vm.prank(borrower);
        pool.borrow(16_000e6);
        assertEq(pool.getHealthFactor(borrower), 1e18);
    }

    function testFuzz_borrowAboveLimitAlwaysReverts(uint256 amount) public {
        openPosition(borrower, 10 ether, 0);
        amount = bound(amount, 16_001e6, 900_000e6);

        vm.prank(borrower);
        vm.expectRevert(VigilPool.HealthFactorTooLow.selector);
        pool.borrow(amount);
    }

    function testFuzz_borrowWithinLimitKeepsHealthFactorAboveOne(uint256 amount) public {
        openPosition(borrower, 10 ether, 0);
        amount = bound(amount, 1e6, 16_000e6);

        vm.prank(borrower);
        pool.borrow(amount);
        assertGe(pool.getHealthFactor(borrower), 1e18);
    }

    function test_withdrawCollateralThatBreaksHealthReverts() public {
        openPosition(borrower, 10 ether, 10_000e6);

        vm.prank(borrower);
        vm.expectRevert(VigilPool.HealthFactorTooLow.selector);
        pool.withdrawCollateral(address(weth), 5 ether);

        vm.prank(borrower);
        pool.withdrawCollateral(address(weth), 1 ether);
        assertEq(weth.balanceOf(borrower), 1 ether);
    }

    function test_disableCollateralThatBreaksHealthReverts() public {
        openPosition(borrower, 10 ether, 1000e6);

        vm.prank(borrower);
        vm.expectRevert(VigilPool.HealthFactorTooLow.selector);
        pool.setCollateralEnabled(address(weth), false);
    }

    function test_disableCollateralWithoutDebtWorks() public {
        openPosition(borrower, 10 ether, 0);

        vm.prank(borrower);
        pool.setCollateralEnabled(address(weth), false);
        (,, uint256 bitmap) = pool.positionOf(borrower);
        assertEq(bitmap, 0);
    }

    function test_repayCapsAtDebt() public {
        openPosition(borrower, 10 ether, 1000e6);
        usdc.mint(borrower, 5000e6);

        vm.startPrank(borrower);
        usdc.approve(address(pool), type(uint256).max);
        pool.repay(5000e6);
        vm.stopPrank();

        assertEq(pool.getDebt(borrower), 0);
        assertLt(usdc.balanceOf(borrower), 5000e6 + 1000e6);
    }

    function test_healthFactorWithoutDebtIsMax() public {
        openPosition(borrower, 10 ether, 0);
        assertEq(pool.getHealthFactor(borrower), type(uint256).max);
    }

    function test_onlySettlementCanApplyLiquidation() public {
        vm.expectRevert(VigilPool.NotSettlement.selector);
        pool.applyLiquidation(borrower, address(weth), address(this), 1, 1);
    }

    function test_collateralListingLimits() public {
        listToken(address(new MockERC20("A", "A", 18)), 10);
        listToken(address(new MockERC20("B", "B", 18)), 10);

        MockERC20 extra = new MockERC20("C", "C", 18);
        vm.expectRevert(VigilPool.TooManyCollateralAssets.selector);
        pool.listCollateral(address(extra), 8000);

        vm.expectRevert(VigilPool.AssetAlreadyListed.selector);
        pool.listCollateral(address(weth), 8000);
    }

    function test_unlistedCollateralRejected() public {
        MockERC20 stranger = new MockERC20("X", "X", 18);
        stranger.mint(borrower, 1 ether);

        vm.startPrank(borrower);
        stranger.approve(address(pool), 1 ether);
        vm.expectRevert(VigilPool.AssetNotListed.selector);
        pool.depositCollateral(address(stranger), 1 ether);
        vm.stopPrank();
    }

    // SC-09: the health factor loop costs more with more enabled assets, not with more listed assets
    function test_healthFactorGasFollowsEnabledAssets() public {
        MockERC20 second = new MockERC20("Second", "TWO", 18);
        MockERC20 third = new MockERC20("Third", "THREE", 18);
        listToken(address(second), 100);
        listToken(address(third), 100);

        address oneEnabled = makeAddr("oneEnabled");
        address threeEnabled = makeAddr("threeEnabled");
        openPosition(oneEnabled, 10 ether, 1000e6);
        openPosition(threeEnabled, 10 ether, 1000e6);

        second.mint(threeEnabled, 100 ether);
        third.mint(threeEnabled, 100 ether);
        vm.startPrank(threeEnabled);
        second.approve(address(pool), type(uint256).max);
        third.approve(address(pool), type(uint256).max);
        pool.depositCollateral(address(second), 100 ether);
        pool.depositCollateral(address(third), 100 ether);
        vm.stopPrank();

        uint256 gasBefore = gasleft();
        pool.getHealthFactor(oneEnabled);
        uint256 oneEnabledGas = gasBefore - gasleft();

        gasBefore = gasleft();
        pool.getHealthFactor(threeEnabled);
        uint256 threeEnabledGas = gasBefore - gasleft();

        assertGt(threeEnabledGas, oneEnabledGas);
    }

    function test_viewsAgreeWithEachOther() public {
        openPosition(borrower, 10 ether, 5000e6);
        (uint256 healthFactor, uint256 debt, uint256 collateralBalance) =
            pool.getLiquidationData(borrower, address(weth));

        assertEq(healthFactor, pool.getHealthFactor(borrower));
        assertEq(debt, pool.getDebt(borrower));
        assertEq(collateralBalance, pool.getCollateralBalance(borrower, address(weth)));
        assertTrue(pool.hasCollateral(borrower));
    }
}
