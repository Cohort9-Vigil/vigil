// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test} from "forge-std/Test.sol";

import {VigilBase} from "../Base.t.sol";
import {VigilPool} from "../../src/core/VigilPool.sol";
import {LiquidationSettlement} from "../../src/liquidation/LiquidationSettlement.sol";
import {OracleAdapter} from "../../src/oracle/OracleAdapter.sol";
import {MockChainlinkFeed} from "../../src/mocks/MockChainlinkFeed.sol";
import {MockERC20} from "../../src/mocks/MockERC20.sol";
import {IOracleAdapter} from "../../src/interfaces/IOracleAdapter.sol";

/// @notice Random user actions for the invariant suite. Failed calls are expected and ignored.
contract Handler is Test {
    VigilBase internal immutable base;
    VigilPool internal immutable pool;
    LiquidationSettlement internal immutable settlement;
    OracleAdapter internal immutable oracle;
    MockERC20 internal immutable usdc;
    MockERC20 internal immutable weth;
    MockChainlinkFeed internal immutable wethFeed;

    address[3] public actors;
    uint256 internal nextNonce;
    uint256 internal wethPrice = 2000;

    // ghost variables read by the invariants
    bool public liquidationLoweredHealth;
    bool public bonusWentDown;
    bool public debtGrewWhileFrozen;
    uint256 public auctionsStarted;
    uint256 public liquidationsExecuted;
    uint256 public actionsWhileFrozen;
    mapping(address actor => uint256 bonus) internal lastBonus;
    mapping(address actor => uint256 startBlock) internal lastStartBlock;

    constructor(
        VigilBase base_,
        VigilPool pool_,
        LiquidationSettlement settlement_,
        OracleAdapter oracle_,
        MockERC20 usdc_,
        MockERC20 weth_,
        MockChainlinkFeed wethFeed_
    ) {
        base = base_;
        pool = pool_;
        settlement = settlement_;
        oracle = oracle_;
        usdc = usdc_;
        weth = weth_;
        wethFeed = wethFeed_;

        for (uint256 i = 0; i < actors.length; i++) {
            address actor = makeAddr(string.concat("actor", vm.toString(i)));
            actors[i] = actor;

            usdc.mint(actor, 10_000_000e6);
            weth.mint(actor, 10_000 ether);
            vm.startPrank(actor);
            usdc.approve(address(pool), type(uint256).max);
            weth.approve(address(pool), type(uint256).max);
            vm.stopPrank();
        }
    }

    function supply(uint256 actorSeed, uint256 amount) external {
        address actor = _actor(actorSeed);
        amount = bound(amount, 1e6, 500_000e6);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        vm.prank(actor);
        try pool.supply(amount) {} catch {}
        _afterAction(frozen, debtBefore);
    }

    function withdrawSupply(uint256 actorSeed, uint256 amount) external {
        address actor = _actor(actorSeed);
        amount = bound(amount, 1, 500_000e6);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        vm.prank(actor);
        try pool.withdrawSupply(amount) {} catch {}
        _afterAction(frozen, debtBefore);
    }

    function depositCollateral(uint256 actorSeed, uint256 amount) external {
        address actor = _actor(actorSeed);
        amount = bound(amount, 1, 1000 ether);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        vm.prank(actor);
        try pool.depositCollateral(address(weth), amount) {} catch {}
        _afterAction(frozen, debtBefore);
    }

    function withdrawCollateral(uint256 actorSeed, uint256 amount) external {
        address actor = _actor(actorSeed);
        amount = bound(amount, 1, 1000 ether);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        vm.prank(actor);
        try pool.withdrawCollateral(address(weth), amount) {} catch {}
        _afterAction(frozen, debtBefore);
    }

    function borrow(uint256 actorSeed, uint256 amount) external {
        address actor = _actor(actorSeed);
        amount = bound(amount, 1e6, 200_000e6);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        vm.prank(actor);
        try pool.borrow(amount) {} catch {}
        _afterAction(frozen, debtBefore);
    }

    function repay(uint256 actorSeed, uint256 amount) external {
        address actor = _actor(actorSeed);
        amount = bound(amount, 1, 200_000e6);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        vm.prank(actor);
        try pool.repay(amount) {} catch {}
        _afterAction(frozen, debtBefore);
    }

    function passTime(uint256 secondsToPass) external {
        vm.warp(block.timestamp + bound(secondsToPass, 1, 3 days));
        // keep the feeds fresh unless a test run has frozen the oracle on purpose
        if (oracle.state() == IOracleAdapter.State.Normal || wethFeed.updatedAt() != 0) _refreshFeeds();
    }

    function movePrice(uint256 priceSeed) external {
        wethPrice = bound(priceSeed, 700, 3000);
        _refreshFeeds();
    }

    function freezeOracle() external {
        wethFeed.setUpdatedAt(0);
    }

    function unfreezeOracle() external {
        _refreshFeeds();
    }

    function passBlocks(uint256 blocks) external {
        vm.roll(block.number + bound(blocks, 1, 40));

        for (uint256 i = 0; i < actors.length; i++) {
            address actor = actors[i];
            uint256 startBlock = settlement.auctionStartBlock(actor);
            uint256 bonus = settlement.currentBonusBps(actor);

            if (startBlock != 0 && startBlock == lastStartBlock[actor] && bonus < lastBonus[actor]) {
                bonusWentDown = true;
            }
            lastStartBlock[actor] = startBlock;
            lastBonus[actor] = bonus;
        }
    }

    function startAuction(uint256 actorSeed) external {
        address actor = _actor(actorSeed);
        try settlement.startAuction(actor) {
            auctionsStarted++;
        } catch {}

        lastStartBlock[actor] = settlement.auctionStartBlock(actor);
        lastBonus[actor] = settlement.currentBonusBps(actor);
    }

    function liquidate(uint256 actorSeed, uint256 repayAmount) external {
        address target = _actor(actorSeed);
        if (settlement.auctionStartBlock(target) == 0) return;
        repayAmount = bound(repayAmount, 1e6, 300_000e6);
        (bool frozen, uint256 debtBefore) = _beforeAction();

        uint256 healthBefore;
        try pool.getHealthFactor(target) returns (uint256 value) {
            healthBefore = value;
        } catch {
            return;
        }

        LiquidationSettlement.LiquidationIntent memory intent = base.makeIntent(target, repayAmount, nextNonce++);
        bytes memory signature = base.signIntent(intent);

        bool underwater = _isUnderwater(target, intent.collateralAsset);

        try settlement.executeLiquidation(intent, signature) {
            liquidationsExecuted++;
            // an underwater position cannot get healthier, that case is allowed to drop
            if (!underwater && pool.getDebt(target) > 0 && pool.hasCollateral(target)) {
                if (pool.getHealthFactor(target) < healthBefore) liquidationLoweredHealth = true;
            }
        } catch {}
        _afterAction(frozen, debtBefore);
    }

    // Collateral worth less than the debt plus the bonus, with the bonus at its maximum to stay on the safe side.
    function _isUnderwater(address target, address collateralAsset) internal view returns (bool) {
        (, uint256 debt, uint256 collateral) = pool.getLiquidationData(target, collateralAsset);
        uint256 collateralValue = collateral * oracle.getPrice(collateralAsset) / 1e18 / 1e12;
        return collateralValue * 10_000 < debt * (10_000 + settlement.maxBonusBps());
    }

    function _actor(uint256 seed) internal view returns (address) {
        return actors[seed % actors.length];
    }

    function _refreshFeeds() internal {
        wethFeed.setAnswer(int128(int256(wethPrice * 1e8)));
        base.refreshUsdcFeed();
    }

    function _beforeAction() internal view returns (bool frozen, uint256 scaledDebt) {
        frozen = oracle.state() == IOracleAdapter.State.Frozen;
        scaledDebt = pool.poolState().totalScaledDebt;
    }

    // Scaled debt only moves when someone borrows or repays, interest changes the index instead.
    function _afterAction(bool wasFrozen, uint256 scaledDebtBefore) internal {
        if (wasFrozen) actionsWhileFrozen++;
        if (wasFrozen && pool.poolState().totalScaledDebt > scaledDebtBefore) debtGrewWhileFrozen = true;
    }
}
