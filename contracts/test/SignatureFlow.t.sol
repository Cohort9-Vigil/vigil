// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "./Base.t.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {IVigilPool} from "../src/interfaces/IVigilPool.sol";

/// @notice Follows the money through signed intents: the signer is the one who pays and the one who is paid.
contract SignatureFlowTest is VigilBase {
    uint256 internal constant ATTACKER_KEY = 0xBAD;
    // order of the secp256k1 curve, used to build a malleable twin of a valid signature
    uint256 internal constant SECP256K1_N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;

    address internal attacker = vm.addr(ATTACKER_KEY);
    address internal relayer = makeAddr("relayer");

    // 10 WETH at $2000 backing $10,000 of debt, crashed to $1200 with the auction running at full bonus
    function setUp() public override {
        super.setUp();
        openPosition(borrower, 10 ether, 10_000e6);
        setWethPrice(1200);
        startAuctionAndWait(borrower, AUCTION_DURATION);
    }

    // ---------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------

    function _sign(uint256 key, address verifyingContract, LiquidationSettlement.LiquidationIntent memory intent)
        internal
        view
        returns (bytes memory)
    {
        bytes32 domainSeparator = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("VigilLiquidation"),
                keccak256("1"),
                block.chainid,
                verifyingContract
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                INTENT_TYPEHASH,
                intent.borrower,
                intent.collateralAsset,
                intent.maxRepay,
                intent.minCollateralOut,
                intent.nonce,
                intent.deadline
            )
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash)));
        return abi.encodePacked(r, s, v);
    }

    function _fundAttacker(uint256 amount) internal {
        usdc.mint(attacker, amount);
        vm.prank(attacker);
        usdc.approve(address(settlement), type(uint256).max);
    }

    /// @dev Every balance that matters, hashed, so a test can say "no funds moved at all".
    function _balances() internal view returns (bytes32) {
        uint256[] memory values = new uint256[](14);
        values[0] = usdc.balanceOf(address(pool));
        values[1] = weth.balanceOf(address(pool));
        values[2] = usdc.balanceOf(liquidator);
        values[3] = weth.balanceOf(liquidator);
        values[4] = usdc.balanceOf(attacker);
        values[5] = weth.balanceOf(attacker);
        values[6] = usdc.balanceOf(relayer);
        values[7] = weth.balanceOf(relayer);
        values[8] = usdc.balanceOf(borrower);
        values[9] = weth.balanceOf(borrower);
        values[10] = usdc.balanceOf(address(settlement));
        values[11] = weth.balanceOf(address(settlement));
        values[12] = pool.getCollateralBalance(borrower, address(weth));
        values[13] = pool.getDebt(borrower);
        return keccak256(abi.encode(values));
    }

    function _bitmap(address account, uint256 nonce) internal view returns (uint256) {
        return settlement.nonceBitmap(account, nonce >> 8);
    }

    // ---------------------------------------------------------------------
    // The signer pays and the signer is paid
    // ---------------------------------------------------------------------

    function test_relayerSubmitsButTheSignerPaysAndReceives() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);
        uint256 liquidatorUsdcBefore = usdc.balanceOf(liquidator);

        vm.prank(relayer);
        settlement.executeLiquidation(intent, signature);

        assertEq(liquidatorUsdcBefore - usdc.balanceOf(liquidator), 5000e6, "signer paid");
        assertGt(weth.balanceOf(liquidator), 0, "signer received the collateral");
        assertEq(usdc.balanceOf(relayer), 0, "relayer paid nothing");
        assertEq(weth.balanceOf(relayer), 0, "relayer received nothing");
    }

    function test_frontRunnerCannotRedirectTheProceeds() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);

        // someone copies the signature from the mempool and submits it first
        vm.prank(attacker);
        settlement.executeLiquidation(intent, signature);

        assertEq(weth.balanceOf(attacker), 0, "front runner got nothing");
        assertEq(usdc.balanceOf(attacker), 0, "front runner paid nothing");
        assertEq(weth.balanceOf(liquidator), (5500e6 * 1e12 * 1e18) / uint256(1200e18), "intended party got it all");
    }

    function test_borrowerSubmittingTheIntentGetsNothing() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);
        uint256 borrowerUsdcBefore = usdc.balanceOf(borrower);

        vm.prank(borrower);
        settlement.executeLiquidation(intent, signature);

        assertEq(weth.balanceOf(borrower), 0, "no collateral flows back to the borrower");
        assertEq(usdc.balanceOf(borrower), borrowerUsdcBefore, "borrower paid nothing, the signer repaid the debt");
        assertEq(pool.getDebt(borrower), 5000e6);
    }

    function test_attackerSignatureOnlyMovesTheAttackersOwnFunds() public {
        _fundAttacker(5000e6);
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = _sign(ATTACKER_KEY, address(settlement), intent);

        uint256 liquidatorUsdcBefore = usdc.balanceOf(liquidator);
        settlement.executeLiquidation(intent, signature);

        assertEq(usdc.balanceOf(attacker), 0, "attacker paid with their own funds");
        assertGt(weth.balanceOf(attacker), 0, "attacker received the collateral");
        assertEq(usdc.balanceOf(liquidator), liquidatorUsdcBefore, "the other liquidator was not touched");
        assertEq(weth.balanceOf(liquidator), 0);
    }

    function test_repaymentLandsInThePoolAndCollateralLeavesThePool() public {
        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));
        uint256 poolWethBefore = weth.balanceOf(address(pool));

        liquidate(borrower, 5000e6, 0);

        assertEq(usdc.balanceOf(address(pool)) - poolUsdcBefore, 5000e6);
        assertEq(poolWethBefore - weth.balanceOf(address(pool)), weth.balanceOf(liquidator));
        assertEq(usdc.balanceOf(address(settlement)), 0, "settlement is only a pass-through");
        assertEq(weth.balanceOf(address(settlement)), 0);
    }

    function test_allowanceFallsByExactlyTheRepayment() public {
        vm.prank(liquidator);
        usdc.approve(address(settlement), 7000e6);

        liquidate(borrower, 5000e6, 0);

        assertEq(usdc.allowance(liquidator, address(settlement)), 2000e6);
    }

    function test_allowanceCapsWhatASignatureCanSpend() public {
        vm.prank(liquidator);
        usdc.approve(address(settlement), 5000e6);

        liquidate(borrower, 5000e6, 0);
        assertEq(usdc.allowance(liquidator, address(settlement)), 0);

        // a second signed intent cannot pull more than the signer approved
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 1);
        bytes memory signature = signIntent(intent);
        bytes32 before = _balances();
        vm.expectRevert();
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before);
        assertEq(_bitmap(liquidator, 1), 1, "only nonce 0 was ever spent");
    }

    // ---------------------------------------------------------------------
    // A failed intent moves nothing and burns nothing
    // ---------------------------------------------------------------------

    function test_signerWithoutFundsMovesNothingAndKeepsTheNonce() public {
        vm.prank(attacker);
        usdc.approve(address(settlement), type(uint256).max);
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 3);
        bytes memory signature = _sign(ATTACKER_KEY, address(settlement), intent);
        bytes32 before = _balances();

        vm.expectRevert();
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before);
        assertEq(_bitmap(attacker, 3), 0, "nonce not burned");
    }

    function test_signerWithoutAllowanceMovesNothingAndKeepsTheNonce() public {
        usdc.mint(attacker, 5000e6);
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 3);
        bytes memory signature = _sign(ATTACKER_KEY, address(settlement), intent);
        bytes32 before = _balances();

        vm.expectRevert();
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before);
        assertEq(usdc.balanceOf(attacker), 5000e6);
        assertEq(_bitmap(attacker, 3), 0);
    }

    function test_minCollateralOutFailureMovesNothingAndKeepsTheNonce() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 9);
        intent.minCollateralOut = 100 ether;
        bytes memory signature = signIntent(intent);
        bytes32 before = _balances();

        vm.expectRevert(LiquidationSettlement.InsufficientCollateralOut.selector);
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before);
        assertEq(_bitmap(liquidator, 9), 0, "a reverted intent can be retried");

        // the same nonce works once the terms are acceptable
        liquidate(borrower, 5000e6, 9);
        assertEq(_bitmap(liquidator, 9), 1 << 9);
    }

    function test_replayMovesNoFundsTheSecondTime() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 7);
        bytes memory signature = signIntent(intent);
        settlement.executeLiquidation(intent, signature);
        bytes32 afterFirst = _balances();

        vm.expectRevert(LiquidationSettlement.NonceAlreadyUsed.selector);
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), afterFirst, "the replay cost and paid nothing");
    }

    function test_oldIntentCannotBeReplayedInALaterAuction() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);
        settlement.executeLiquidation(intent, signature);

        // the first auction is over because the position recovered, then the price falls again
        assertEq(settlement.auctionStartBlock(borrower), 0);
        setWethPrice(1000);
        startAuctionAndWait(borrower, AUCTION_DURATION);
        bytes32 before = _balances();

        vm.expectRevert(LiquidationSettlement.NonceAlreadyUsed.selector);
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before);
    }

    function test_expiredIntentMovesNothing() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        bytes memory signature = signIntent(intent);

        vm.warp(intent.deadline + 1);
        setWethPrice(1200);
        refreshUsdcFeed();
        bytes32 before = _balances();

        vm.expectRevert(LiquidationSettlement.IntentExpired.selector);
        settlement.executeLiquidation(intent, signature);

        assertEq(_balances(), before);
        assertEq(_bitmap(liquidator, 0), 0);
    }

    function test_intentIsValidOnItsDeadlineBlock() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 5000e6, 0);
        intent.deadline = block.timestamp;
        bytes memory signature = signIntent(intent);

        settlement.executeLiquidation(intent, signature);

        assertEq(pool.getDebt(borrower), 5000e6);
    }

    // ---------------------------------------------------------------------
    // The signature covers every field
    // ---------------------------------------------------------------------

    function test_changingAnySignedFieldMovesNoFunds() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory signature = signIntent(intent);
        bytes32 before = _balances();

        LiquidationSettlement.LiquidationIntent memory changed = intent;
        changed.borrower = makeAddr("someoneElse");
        vm.expectRevert();
        settlement.executeLiquidation(changed, signature);

        changed = intent;
        changed.collateralAsset = address(usdc);
        vm.expectRevert();
        settlement.executeLiquidation(changed, signature);

        changed = intent;
        changed.maxRepay = 5000e6;
        vm.expectRevert();
        settlement.executeLiquidation(changed, signature);

        changed = intent;
        changed.minCollateralOut = 1;
        vm.expectRevert();
        settlement.executeLiquidation(changed, signature);

        changed = intent;
        changed.nonce = 1;
        vm.expectRevert();
        settlement.executeLiquidation(changed, signature);

        changed = intent;
        changed.deadline = intent.deadline + 1;
        vm.expectRevert();
        settlement.executeLiquidation(changed, signature);

        assertEq(_balances(), before, "none of the forged variants moved a token");
        assertEq(_bitmap(liquidator, 0), 0);
    }

    function test_signatureForAnotherSettlementContractMovesNothing() public {
        LiquidationSettlement other = new LiquidationSettlement(
            IVigilPool(address(pool)), IOracleAdapter(address(oracle)), AUCTION_DURATION, MAX_BONUS_BPS, DUST_THRESHOLD
        );
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory foreign = _sign(LIQUIDATOR_KEY, address(other), intent);
        bytes32 before = _balances();

        vm.expectRevert();
        settlement.executeLiquidation(intent, foreign);

        assertEq(_balances(), before);
    }

    function test_malleableTwinOfAValidSignatureIsRejected() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes memory signature = signIntent(intent);

        bytes32 r;
        bytes32 s;
        uint8 v;
        // forge-lint: disable-next-line(unsafe-typecast)
        assembly {
            r := mload(add(signature, 32))
            s := mload(add(signature, 64))
            v := byte(0, mload(add(signature, 96)))
        }
        bytes memory twin = abi.encodePacked(r, bytes32(SECP256K1_N - uint256(s)), v == 27 ? uint8(28) : uint8(27));
        bytes32 before = _balances();

        vm.expectRevert(LiquidationSettlement.InvalidSignature.selector);
        settlement.executeLiquidation(intent, twin);

        assertEq(_balances(), before);
        assertEq(_bitmap(liquidator, 0), 0);

        // the original still works, the twin could not have bypassed the nonce anyway
        settlement.executeLiquidation(intent, signature);
    }

    function test_emptyAndZeroSignaturesMoveNothing() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        bytes32 before = _balances();

        vm.expectRevert(LiquidationSettlement.InvalidSignature.selector);
        settlement.executeLiquidation(intent, "");

        vm.expectRevert(LiquidationSettlement.InvalidSignature.selector);
        settlement.executeLiquidation(intent, new bytes(65));

        assertEq(_balances(), before);
    }

    // ---------------------------------------------------------------------
    // Nonces
    // ---------------------------------------------------------------------

    function test_nonceEventNamesTheSigner() public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 42);
        bytes memory signature = signIntent(intent);

        vm.expectEmit(true, false, false, true, address(settlement));
        emit LiquidationSettlement.NonceUsed(liquidator, 42);

        vm.prank(relayer);
        settlement.executeLiquidation(intent, signature);
    }

    function test_noncesAreTrackedPerSigner() public {
        _fundAttacker(2000e6);

        liquidate(borrower, 1000e6, 0);

        // the attacker has their own nonce 0, the liquidator's spent nonce does not block it
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 0);
        settlement.executeLiquidation(intent, _sign(ATTACKER_KEY, address(settlement), intent));

        assertEq(_bitmap(liquidator, 0), 1);
        assertEq(_bitmap(attacker, 0), 1);
        assertEq(usdc.balanceOf(attacker), 1000e6, "attacker paid exactly their own 1000");
    }

    function test_oneSignerSpendingANonceDoesNotBurnAnotherSignersNonce() public {
        _fundAttacker(1000e6);
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 1000e6, 5);
        settlement.executeLiquidation(intent, _sign(ATTACKER_KEY, address(settlement), intent));

        assertEq(_bitmap(liquidator, 5), 0, "the liquidator's nonce 5 is untouched");
        liquidate(borrower, 1000e6, 5);
        assertEq(_bitmap(liquidator, 5), 1 << 5);
    }

    function testFuzz_anyNonceWorksOnceAndOnlyFlipsItsOwnBit(uint256 nonce) public {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(borrower, 500e6, nonce);
        bytes memory signature = signIntent(intent);
        uint256 wordBefore = _bitmap(liquidator, nonce);
        uint256 liquidatorUsdcBefore = usdc.balanceOf(liquidator);

        settlement.executeLiquidation(intent, signature);

        assertEq(_bitmap(liquidator, nonce), wordBefore | (uint256(1) << (nonce & 0xff)), "exactly one bit flipped");
        assertEq(liquidatorUsdcBefore - usdc.balanceOf(liquidator), 500e6, "paid once");

        bytes32 afterFirst = _balances();
        vm.expectRevert(LiquidationSettlement.NonceAlreadyUsed.selector);
        settlement.executeLiquidation(intent, signature);
        assertEq(_balances(), afterFirst, "the replay moved nothing");
    }

    function testFuzz_totalPaidEqualsSumOfRepaymentsAcrossNonces(uint8 count) public {
        uint256 n = bound(count, 1, 5);
        uint256 liquidatorUsdcBefore = usdc.balanceOf(liquidator);
        uint256 poolUsdcBefore = usdc.balanceOf(address(pool));

        for (uint256 i = 0; i < n; i++) {
            liquidate(borrower, 100e6, i * 37);
        }

        assertEq(liquidatorUsdcBefore - usdc.balanceOf(liquidator), n * 100e6);
        assertEq(usdc.balanceOf(address(pool)) - poolUsdcBefore, n * 100e6);
        assertEq(usdc.balanceOf(address(settlement)), 0);
    }
}
