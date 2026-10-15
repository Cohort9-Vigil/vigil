// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "./Base.t.sol";
import {VigilPool} from "../src/core/VigilPool.sol";
import {FeeOnTransferToken} from "../src/mocks/FeeOnTransferToken.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {NoReturnToken} from "../src/mocks/NoReturnToken.sol";
import {ReentrantToken} from "../src/mocks/ReentrantToken.sol";
import {SafeTransfer} from "../src/libraries/SafeTransfer.sol";

contract HostileTokensTest is VigilBase {
    // SC-11
    function test_tokenWithoutReturnValuesCanBeDepositedAndWithdrawn() public {
        NoReturnToken usdt = new NoReturnToken("Tether", "USDT", 6);
        listToken(address(usdt), 1);
        usdt.mint(borrower, 1000e6);

        vm.startPrank(borrower);
        usdt.approve(address(pool), 1000e6);
        pool.depositCollateral(address(usdt), 1000e6);
        assertEq(pool.getCollateralBalance(borrower, address(usdt)), 1000e6);

        pool.withdrawCollateral(address(usdt), 400e6);
        vm.stopPrank();

        assertEq(usdt.balanceOf(borrower), 400e6);
    }

    function test_failedTransferReverts() public {
        NoReturnToken usdt = new NoReturnToken("Tether", "USDT", 6);
        listToken(address(usdt), 1);

        vm.startPrank(borrower);
        usdt.approve(address(pool), 1000e6);
        // the borrower has no balance, the token reverts, and SafeTransfer reports it
        vm.expectRevert();
        pool.depositCollateral(address(usdt), 1000e6);
        vm.stopPrank();
    }

    function test_safeTransferRejectsAddressWithoutCode() public {
        vm.expectRevert(SafeTransfer.TransferFailed.selector);
        this.callSafeTransfer(address(0xDEAD), address(this), 1);
    }

    function callSafeTransfer(address token, address to, uint256 amount) external {
        SafeTransfer.safeTransfer(token, to, amount);
    }

    // SC-12
    function test_feeOnTransferCreditsTheAmountThatArrived() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        vm.stopPrank();

        assertEq(taxed.balanceOf(address(pool)), 98 ether);
        assertEq(pool.getCollateralBalance(borrower, address(taxed)), 98 ether);
    }

    function test_feeOnTransferBorrowLimitUsesTheCreditedAmount() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);

        // 98 tokens * $100 * 80% = $7840
        vm.expectRevert(VigilPool.HealthFactorTooLow.selector);
        pool.borrow(7841e6);
        pool.borrow(7840e6);
        vm.stopPrank();
    }

    function test_feeOnTransferWithdrawalPaysTheFeeAgain() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        pool.withdrawCollateral(address(taxed), 98 ether);
        vm.stopPrank();

        assertEq(pool.getCollateralBalance(borrower, address(taxed)), 0);
        assertEq(taxed.balanceOf(borrower), 96.04 ether);
    }

    // SC-13
    function test_sameDollarValueGivesTheSameHealthFactorFor6And8Decimals() public {
        MockERC20 sixDecimals = new MockERC20("Six", "SIX", 6);
        MockERC20 eightDecimals = new MockERC20("Eight", "EIGHT", 8);
        listToken(address(sixDecimals), 100);
        listToken(address(eightDecimals), 100);
        sixDecimals.mint(borrower, 100e6);

        vm.startPrank(borrower);
        sixDecimals.approve(address(pool), 100e6);
        pool.depositCollateral(address(sixDecimals), 100e6);
        pool.borrow(5000e6);
        vm.stopPrank();

        address second = makeAddr("second");
        eightDecimals.mint(second, 100e8);
        vm.startPrank(second);
        eightDecimals.approve(address(pool), 100e8);
        pool.depositCollateral(address(eightDecimals), 100e8);
        pool.borrow(5000e6);
        vm.stopPrank();

        // $10,000 of collateral at 80% against $5000 of debt
        assertEq(pool.getHealthFactor(borrower), 1.6e18);
        assertEq(pool.getHealthFactor(second), 1.6e18);
    }

    function testFuzz_borrowLimitFollowsTokenDecimals(uint8 decimalsSeed, uint256 amountSeed) public {
        uint8 tokenDecimals = uint8(bound(decimalsSeed, 6, 18));
        MockERC20 token = new MockERC20("Fuzz", "FUZZ", tokenDecimals);
        listToken(address(token), 100);

        uint256 unit = 10 ** tokenDecimals;
        uint256 amount = bound(amountSeed, 10 * unit, 1000 * unit);
        token.mint(borrower, amount);

        // collateral value in USDC units, then the 80% limit, both rounded down
        uint256 limit = ((amount * 100e6) / unit) * 8000 / 10_000;

        vm.startPrank(borrower);
        token.approve(address(pool), amount);
        pool.depositCollateral(address(token), amount);
        vm.expectRevert(VigilPool.HealthFactorTooLow.selector);
        pool.borrow(limit + 1e6);
        pool.borrow(limit);
        vm.stopPrank();

        assertGe(pool.getHealthFactor(borrower), 1e18);
    }

    // SC-23
    function test_reentrantTokenCannotCallBackIntoThePool() public {
        ReentrantToken evil = new ReentrantToken("Evil", "EVIL", 18);
        listToken(address(evil), 10);
        evil.mint(borrower, 10 ether);
        evil.setCallback(address(pool), abi.encodeCall(pool.supply, (1)), true);

        // the lock reverts the callback, the token passes that up, and SafeTransfer reports a failed transfer
        vm.startPrank(borrower);
        evil.approve(address(pool), 10 ether);
        vm.expectRevert(SafeTransfer.TransferFailed.selector);
        pool.depositCollateral(address(evil), 10 ether);
        vm.stopPrank();
    }

    function test_reentryAttemptIsRecordedWhenTheTokenSwallowsIt() public {
        ReentrantToken evil = new ReentrantToken("Evil", "EVIL", 18);
        listToken(address(evil), 10);
        evil.mint(borrower, 10 ether);
        evil.setCallback(address(pool), abi.encodeCall(pool.borrow, (1e6)), false);

        vm.startPrank(borrower);
        evil.approve(address(pool), 10 ether);
        pool.depositCollateral(address(evil), 10 ether);
        vm.stopPrank();

        assertFalse(evil.lastCallbackSucceeded());
        assertEq(bytes4(evil.lastCallbackReturnData()), VigilPool.ReentrantCall.selector);
        assertEq(pool.getDebt(borrower), 0);
    }

    function test_reentrantWithdrawCannotDrainTheSamePosition() public {
        ReentrantToken evil = new ReentrantToken("Evil", "EVIL", 18);
        listToken(address(evil), 10);
        evil.mint(borrower, 10 ether);

        vm.startPrank(borrower);
        evil.approve(address(pool), 10 ether);
        pool.depositCollateral(address(evil), 10 ether);
        vm.stopPrank();

        evil.setCallback(address(pool), abi.encodeCall(pool.withdrawCollateral, (address(evil), 10 ether)), true);

        vm.prank(borrower);
        vm.expectRevert(SafeTransfer.TransferFailed.selector);
        pool.withdrawCollateral(address(evil), 10 ether);

        assertEq(pool.getCollateralBalance(borrower, address(evil)), 10 ether);
    }

    // SC-24
    function test_viewsRevertDuringACallback() public {
        ReentrantToken evil = new ReentrantToken("Evil", "EVIL", 18);
        listToken(address(evil), 10);
        evil.mint(borrower, 20 ether);

        vm.startPrank(borrower);
        evil.approve(address(pool), 20 ether);
        pool.depositCollateral(address(evil), 10 ether);
        vm.stopPrank();

        bytes[3] memory reads = [
            abi.encodeCall(pool.getHealthFactor, (borrower)),
            abi.encodeCall(pool.getPrice, (address(evil))),
            abi.encodeCall(pool.getDebt, (borrower))
        ];

        for (uint256 i = 0; i < reads.length; i++) {
            evil.setCallback(address(pool), reads[i], false);
            vm.prank(borrower);
            pool.depositCollateral(address(evil), 1 ether);

            assertFalse(evil.lastCallbackSucceeded());
            assertEq(bytes4(evil.lastCallbackReturnData()), VigilPool.ReentrantCall.selector);
        }
    }

    function test_viewsWorkOutsideACallback() public {
        openPosition(borrower, 10 ether, 1000e6);
        assertGt(pool.getHealthFactor(borrower), 1e18);
        assertEq(pool.getPrice(address(weth)), 2000e18);
        assertEq(pool.getDebt(borrower), 1000e6);
    }
}
