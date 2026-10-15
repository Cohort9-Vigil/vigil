// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";

import {VigilBase} from "./Base.t.sol";
import {VigilPool} from "../src/core/VigilPool.sol";
import {VigilPoolV2} from "../src/core/VigilPoolV2.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {InterestRateModel} from "../src/libraries/InterestRateModel.sol";

contract UpgradeTest is VigilBase {
    uint256 constant POSITIONS = 50;

    struct Snapshot {
        uint256 scaledDebt;
        uint256 scaledSupply;
        uint256 bitmap;
        uint256 collateral;
    }

    address[] users;

    // SC-03
    function test_upgradePreservesEveryBalance() public {
        _openFiftyPositions();
        vm.warp(block.timestamp + 30 days);
        setWethPrice(2000);
        usdcFeed.setAnswer(1e8);

        Snapshot[] memory before = new Snapshot[](POSITIONS);
        for (uint256 i = 0; i < POSITIONS; i++) {
            before[i] = _snapshot(users[i]);
        }
        VigilPool.PoolState memory stateBefore = pool.poolState();
        uint256 lenderSupplyBefore = pool.getHealthFactor(users[0]);

        pool.upgradeToAndCall(address(new VigilPoolV2()), "");

        for (uint256 i = 0; i < POSITIONS; i++) {
            Snapshot memory afterUpgrade = _snapshot(users[i]);
            assertEq(afterUpgrade.scaledDebt, before[i].scaledDebt, "scaled debt");
            assertEq(afterUpgrade.scaledSupply, before[i].scaledSupply, "scaled supply");
            assertEq(afterUpgrade.bitmap, before[i].bitmap, "bitmap");
            assertEq(afterUpgrade.collateral, before[i].collateral, "collateral");
            assertEq(VigilPoolV2(address(pool)).lastActionBlock(users[i]), 0, "new field starts empty");
        }

        VigilPool.PoolState memory stateAfter = pool.poolState();
        assertEq(stateAfter.borrowIndex, stateBefore.borrowIndex);
        assertEq(stateAfter.liquidityIndex, stateBefore.liquidityIndex);
        assertEq(stateAfter.totalScaledDebt, stateBefore.totalScaledDebt);
        assertEq(stateAfter.totalScaledSupply, stateBefore.totalScaledSupply);
        assertEq(stateAfter.reserve, stateBefore.reserve);
        assertEq(pool.getHealthFactor(users[0]), lenderSupplyBefore);
    }

    function test_newFieldWorksAfterTheUpgrade() public {
        _openFiftyPositions();
        pool.upgradeToAndCall(address(new VigilPoolV2()), "");

        vm.roll(777);
        vm.prank(users[3]);
        VigilPoolV2(address(pool)).recordAction();

        assertEq(VigilPoolV2(address(pool)).lastActionBlock(users[3]), 777);
        // the neighbours and the old balances are untouched
        assertEq(VigilPoolV2(address(pool)).lastActionBlock(users[4]), 0);
        assertGt(pool.getCollateralBalance(users[3], address(weth)), 0);
    }

    function test_poolKeepsWorkingAfterTheUpgrade() public {
        _openFiftyPositions();
        pool.upgradeToAndCall(address(new VigilPoolV2()), "");

        vm.prank(users[1]);
        pool.borrow(100e6);
        assertGt(pool.getDebt(users[1]), 100e6);
    }

    function test_onlyTheOwnerCanUpgrade() public {
        VigilPoolV2 next = new VigilPoolV2();
        vm.prank(borrower);
        vm.expectRevert();
        pool.upgradeToAndCall(address(next), "");
    }

    function test_upgradeImplementationCannotBeInitialized() public {
        VigilPoolV2 next = new VigilPoolV2();
        InterestRateModel.RateParams memory rate = InterestRateModel.RateParams(0.02e27, 0.04e27, 0.75e27, 0.8e27);

        vm.expectRevert(Initializable.InvalidInitialization.selector);
        next.initialize(address(this), address(usdc), IOracleAdapter(address(oracle)), 1000, rate);
    }

    function _openFiftyPositions() internal {
        for (uint256 i = 0; i < POSITIONS; i++) {
            address user = makeAddr(string.concat("user", vm.toString(i)));
            users.push(user);
            // every third position borrows nothing, the rest borrow different amounts
            uint256 collateral = (i + 1) * 1 ether;
            uint256 debt = i % 3 == 0 ? 0 : (i + 1) * 100e6;
            openPosition(user, collateral, debt);

            usdc.mint(user, 1000e6);
            vm.startPrank(user);
            usdc.approve(address(pool), type(uint256).max);
            pool.supply((i + 1) * 10e6);
            vm.stopPrank();
        }
    }

    function _snapshot(address user) internal view returns (Snapshot memory) {
        (uint256 scaledDebt, uint256 scaledSupply, uint256 bitmap) = pool.positionOf(user);
        return Snapshot(scaledDebt, scaledSupply, bitmap, pool.getCollateralBalance(user, address(weth)));
    }
}
