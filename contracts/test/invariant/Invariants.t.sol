// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilBase} from "../Base.t.sol";
import {Handler} from "./Handler.sol";
import {FixedPointMath} from "../../src/libraries/FixedPointMath.sol";

contract InvariantsTest is VigilBase {
    using FixedPointMath for uint256;

    Handler handler;

    function setUp() public override {
        super.setUp();
        handler = new Handler(this, pool, settlement, oracle, usdc, weth, wethFeed);
        targetContract(address(handler));
    }

    function _accounts() internal view returns (address[] memory accounts) {
        accounts = new address[](4);
        accounts[0] = lender;
        for (uint256 i = 0; i < 3; i++) {
            accounts[i + 1] = handler.actors(i);
        }
    }

    // the invariants only mean something if the handler really gets to liquidations and frozen periods
    function test_handlerReachesLiquidationsAndFrozenPeriods() public {
        handler.depositCollateral(1, 10 ether);
        handler.borrow(1, 15_000e6);
        handler.movePrice(1000);
        handler.startAuction(1);
        handler.passBlocks(30);
        handler.liquidate(1, 5000e6);

        assertEq(handler.auctionsStarted(), 1, "auction");
        assertEq(handler.liquidationsExecuted(), 1, "liquidation");

        handler.freezeOracle();
        handler.borrow(1, 1e6);
        assertEq(handler.actionsWhileFrozen(), 1, "frozen");
        assertFalse(handler.debtGrewWhileFrozen());
    }

    // I-1: balances recorded per account add up to the totals, within 1 wei per account
    function invariant_I1_scaledBalancesMatchTotals() public view {
        address[] memory accounts = _accounts();
        uint256 debtSum;
        uint256 supplySum;
        uint256 debtValueSum;
        uint256 supplyValueSum;

        VigilPoolState memory state = _state();
        for (uint256 i = 0; i < accounts.length; i++) {
            (uint256 scaledDebt, uint256 scaledSupply,) = pool.positionOf(accounts[i]);
            debtSum += scaledDebt;
            supplySum += scaledSupply;
            debtValueSum += scaledDebt.rayMulUp(state.borrowIndex);
            supplyValueSum += scaledSupply.rayMulDown(state.liquidityIndex);
        }

        assertEq(debtSum, state.totalScaledDebt, "scaled debt");
        assertEq(supplySum, state.totalScaledSupply, "scaled supply");
        assertApproxEqAbs(
            debtValueSum, state.totalScaledDebt.rayMulUp(state.borrowIndex), accounts.length, "debt value"
        );
        assertApproxEqAbs(
            supplyValueSum, state.totalScaledSupply.rayMulDown(state.liquidityIndex), accounts.length, "supply value"
        );
    }

    // I-2: nobody can take out more than went in plus interest, so what the pool holds and is owed
    // always covers what it owes to suppliers and the reserve
    function invariant_I2_poolStaysSolvent() public view {
        VigilPoolState memory state = _state();
        uint256 assets =
            usdc.balanceOf(address(pool)) + state.totalScaledDebt.rayMulUp(state.borrowIndex) + state.deficit;
        uint256 liabilities = state.totalScaledSupply.rayMulDown(state.liquidityIndex) + state.reserve;

        assertGe(assets, liabilities);
    }

    // I-3
    function invariant_I3_liquidationNeverLowersHealth() public view {
        assertFalse(handler.liquidationLoweredHealth());
    }

    // I-4
    function invariant_I4_bonusNeverDecreasesDuringAnAuction() public view {
        assertFalse(handler.bonusWentDown());
    }

    // I-5
    function invariant_I5_debtOnlyGrowsByInterestWhileFrozen() public view {
        assertFalse(handler.debtGrewWhileFrozen());
    }

    struct VigilPoolState {
        uint256 borrowIndex;
        uint256 liquidityIndex;
        uint256 totalScaledDebt;
        uint256 totalScaledSupply;
        uint256 reserve;
        uint256 deficit;
    }

    function _state() internal view returns (VigilPoolState memory state) {
        (
            state.borrowIndex,
            state.liquidityIndex,
            state.totalScaledDebt,
            state.totalScaledSupply,
            state.reserve,
            state.deficit,
        ) = _poolState();
    }

    function _poolState() internal view returns (uint256, uint256, uint256, uint256, uint256, uint256, bool) {
        return (
            pool.poolState().borrowIndex,
            pool.poolState().liquidityIndex,
            pool.poolState().totalScaledDebt,
            pool.poolState().totalScaledSupply,
            pool.poolState().reserve,
            pool.poolState().deficit,
            pool.poolState().paused
        );
    }
}
