// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test,console} from "forge-std/Test.sol";
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

        console.log("Balance of borrower before is",taxed.balanceOf(borrower));
        console.log("Balance of Pool before is", taxed.balanceOf(address(pool)));

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        vm.stopPrank();

        console.log("Balnance of Borrower After ", taxed.balanceOf(borrower));
        console.log("Balance of Pool After ____________________",taxed.balanceOf(address(pool)));
        
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

    // Follows the 2 of every 100 on a deposit: it must be burned, not kept by anyone.
    function test_feeOnTransferDepositBurnsTheFeeAndNobodyKeepsIt() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);
        uint256 supplyBefore = taxed.totalSupply();

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        vm.stopPrank();

        // Sender paid 100, pool received 98, the missing 2 left circulation.
        assertEq(taxed.balanceOf(borrower), 0, "borrower sent the full amount");
        assertEq(taxed.balanceOf(address(pool)), 98 ether, "pool got amount minus fee");
        assertEq(supplyBefore - taxed.totalSupply(), 2 ether, "fee was burned");

        // The fee did not land with any protocol actor.
        assertEq(taxed.balanceOf(address(0)), 0, "zero address holds nothing, burn lowers supply");
        assertEq(taxed.balanceOf(address(this)), 0, "owner got no fee");
        assertEq(taxed.balanceOf(address(settlement)), 0, "settlement got no fee");
        assertEq(taxed.balanceOf(address(oracle)), 0, "oracle got no fee");
        assertEq(taxed.balanceOf(lender), 0, "lender got no fee");
        assertEq(taxed.balanceOf(liquidator), 0, "liquidator got no fee");

        // Supplies and borrowers are accounted by the pool, so the sum of holders equals total supply.
        assertEq(
            taxed.balanceOf(borrower) + taxed.balanceOf(address(pool)),
            taxed.totalSupply(),
            "all remaining tokens are with borrower and pool"
        );
    }

    // The collateral token fee must never leak into the debt-asset reserve or the pool's books.
    function test_feeOnTransferFeeIsNotAddedToReserveOrDebtAccounting() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);

        VigilPool.PoolState memory before = pool.poolState();
        uint256 usdcInPoolBefore = usdc.balanceOf(address(pool));

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        vm.stopPrank();

        VigilPool.PoolState memory afterState = pool.poolState();
        assertEq(afterState.reserve, before.reserve, "reserve unchanged");
        assertEq(afterState.deficit, before.deficit, "deficit unchanged");
        assertEq(afterState.totalScaledSupply, before.totalScaledSupply, "supply shares unchanged");
        assertEq(afterState.totalScaledDebt, before.totalScaledDebt, "debt shares unchanged");
        assertEq(usdc.balanceOf(address(pool)), usdcInPoolBefore, "debt asset balance unchanged");
    }

    // Credited collateral must equal what the pool really holds, no phantom 2 left in the books.
    function test_feeOnTransferPoolBalanceMatchesCreditedCollateral() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        vm.stopPrank();

        assertEq(
            pool.getCollateralBalance(borrower, address(taxed)),
            taxed.balanceOf(address(pool)),
            "credited equals held"
        );
    }

    // Withdrawal pays the fee a second time: 98 leaves the pool, 96.04 arrives, 1.96 is burned.
    function test_feeOnTransferWithdrawalBurnsTheSecondFee() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        listToken(address(taxed), 100);
        taxed.mint(borrower, 100 ether);

        vm.startPrank(borrower);
        taxed.approve(address(pool), 100 ether);
        pool.depositCollateral(address(taxed), 100 ether);
        uint256 supplyAfterDeposit = taxed.totalSupply();
        pool.withdrawCollateral(address(taxed), 98 ether);
        vm.stopPrank();

        assertEq(taxed.balanceOf(address(pool)), 0, "pool fully emptied");
        assertEq(taxed.balanceOf(borrower), 96.04 ether, "borrower receives 98 minus 2%");
        assertEq(supplyAfterDeposit - taxed.totalSupply(), 1.96 ether, "second fee burned");
        assertEq(100 ether - taxed.totalSupply(), 3.96 ether, "total burned across both legs");
        assertEq(taxed.balanceOf(address(0)), 0);
    }

    // Fee rounds down and scales with the configured rate.
    function testFuzz_feeOnTransferBurnMatchesFeeBps(uint16 feeSeed, uint96 amountSeed) public {
        uint256 feeBps = bound(feeSeed, 0, 5000);
        uint256 amount = bound(amountSeed, 1 ether, 1_000_000 ether);

        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, feeBps);
        listToken(address(taxed), 100);
        taxed.mint(borrower, amount);

        vm.startPrank(borrower);
        taxed.approve(address(pool), amount);
        pool.depositCollateral(address(taxed), amount);
        vm.stopPrank();

        uint256 expectedFee = (amount * feeBps) / 10_000;
        assertEq(taxed.balanceOf(address(pool)), amount - expectedFee, "pool holds amount minus fee");
        assertEq(taxed.totalSupply(), amount - expectedFee, "burned exactly the fee");
        assertEq(pool.getCollateralBalance(borrower, address(taxed)), amount - expectedFee, "credit matches");
        assertEq(taxed.balanceOf(borrower), 0);
    }

    // Mints and burns are exempt, so minting must not skim a fee to anyone.
    function test_feeOnTransferMintIsExemptFromTheFee() public {
        FeeOnTransferToken taxed = new FeeOnTransferToken("Taxed", "TAX", 18, 200);
        taxed.mint(borrower, 100 ether);

        assertEq(taxed.balanceOf(borrower), 100 ether);
        assertEq(taxed.totalSupply(), 100 ether);
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
