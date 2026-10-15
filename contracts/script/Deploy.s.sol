// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

import {VigilPool} from "../src/core/VigilPool.sol";
import {OracleAdapter} from "../src/oracle/OracleAdapter.sol";
import {MockAMM} from "../src/oracle/MockAMM.sol";
import {MockChainlinkFeed} from "../src/mocks/MockChainlinkFeed.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {FeeOnTransferToken} from "../src/mocks/FeeOnTransferToken.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {IVigilPool} from "../src/interfaces/IVigilPool.sol";
import {IPriceFeed} from "../src/interfaces/IPriceFeed.sol";
import {ITwapSource} from "../src/interfaces/ITwapSource.sol";
import {InterestRateModel} from "../src/libraries/InterestRateModel.sol";

/// @notice Deploys the whole system for the acceptance scenario: a stablecoin and three collateral assets
/// (fee-on-transfer, 6 decimals, 8 decimals), their feeds and AMMs, the pool behind a proxy and the settlement.
/// Addresses are written to deployments/<chain id>.json for the Seed script and the services.
contract Deploy is Script {
    uint256 constant AUCTION_DURATION = 50;
    uint256 constant MAX_BONUS_BPS = 1000;
    uint256 constant DUST_THRESHOLD = 10e6;
    uint256 constant RESERVE_FACTOR_BPS = 1000;
    uint16 constant LIQUIDATION_THRESHOLD_BPS = 8000;
    uint256 constant TWAP_WINDOW = 30 minutes;
    uint16 constant MAX_DEVIATION_BPS = 500;
    uint256 constant FEED_MAX_STALENESS = 1 days;

    struct Collateral {
        IERC20Metadata token;
        MockChainlinkFeed feed;
        MockAMM amm;
    }

    function run() external {
        vm.startBroadcast();

        MockERC20 stablecoin = new MockERC20("Vigil USD", "vUSD", 6);
        MockChainlinkFeed stableFeed = new MockChainlinkFeed(8, 1e8);

        OracleAdapter oracle = new OracleAdapter(msg.sender);
        oracle.setSource(address(stablecoin), _source(stableFeed, ITwapSource(address(0))));

        VigilPool pool = _deployPool(stablecoin, oracle);

        // prices in whole dollars: the fee token is cheap, the 8 decimal token is a bitcoin-like asset
        Collateral memory feeToken = _listCollateral(
            pool, oracle, IERC20Metadata(address(new FeeOnTransferToken("Vigil Fee Token", "vFEE", 18, 200))), 100
        );
        Collateral memory sixDecimals =
            _listCollateral(pool, oracle, IERC20Metadata(address(new MockERC20("Vigil Six", "vSIX", 6))), 50);
        Collateral memory eightDecimals =
            _listCollateral(pool, oracle, IERC20Metadata(address(new MockERC20("Vigil Eight", "vEIGHT", 8))), 20_000);

        LiquidationSettlement settlement = new LiquidationSettlement(
            IVigilPool(address(pool)), IOracleAdapter(address(oracle)), AUCTION_DURATION, MAX_BONUS_BPS, DUST_THRESHOLD
        );
        pool.setSettlement(address(settlement));

        vm.stopBroadcast();

        _writeAddresses(pool, settlement, oracle, stablecoin, stableFeed, feeToken, sixDecimals, eightDecimals);
        console2.log("pool (proxy)", address(pool));
        console2.log("settlement  ", address(settlement));
        console2.log("oracle      ", address(oracle));
        console2.log("Let the twap window pass before seeding: script/local.sh on anvil, script/sepolia.sh elsewhere");
    }

    function _deployPool(MockERC20 stablecoin, OracleAdapter oracle) internal returns (VigilPool) {
        InterestRateModel.RateParams memory rate = InterestRateModel.RateParams({
            baseRate: 0.02e27, slope1: 0.04e27, slope2: 0.75e27, optimalUtilization: 0.8e27
        });
        bytes memory initData = abi.encodeCall(
            VigilPool.initialize,
            (msg.sender, address(stablecoin), IOracleAdapter(address(oracle)), RESERVE_FACTOR_BPS, rate)
        );
        return VigilPool(address(new ERC1967Proxy(address(new VigilPool()), initData)));
    }

    function _listCollateral(VigilPool pool, OracleAdapter oracle, IERC20Metadata token, uint256 priceUsd)
        internal
        returns (Collateral memory collateral)
    {
        MockChainlinkFeed feed = new MockChainlinkFeed(8, int128(int256(priceUsd * 1e8)));
        // an AMM quoting the same price as the feed, reserves are in 1e18 units whatever the token's decimals
        MockAMM amm = new MockAMM(1000e18, 1000 * priceUsd * 1e18);

        oracle.setSource(address(token), _source(feed, ITwapSource(address(amm))));
        pool.listCollateral(address(token), LIQUIDATION_THRESHOLD_BPS);

        collateral = Collateral(token, feed, amm);
    }

    function _source(MockChainlinkFeed feed, ITwapSource twapSource)
        internal
        pure
        returns (OracleAdapter.Source memory)
    {
        bool hasTwap = address(twapSource) != address(0);
        return OracleAdapter.Source({
            feed: IPriceFeed(address(feed)),
            feedDecimals: 0,
            maxStaleness: uint32(FEED_MAX_STALENESS),
            twapWindow: hasTwap ? uint32(TWAP_WINDOW) : 0,
            maxDeviationBps: hasTwap ? MAX_DEVIATION_BPS : 0,
            twapSource: twapSource,
            minAnswer: 1,
            maxAnswer: 1e15
        });
    }

    function _writeAddresses(
        VigilPool pool,
        LiquidationSettlement settlement,
        OracleAdapter oracle,
        MockERC20 stablecoin,
        MockChainlinkFeed stableFeed,
        Collateral memory feeToken,
        Collateral memory sixDecimals,
        Collateral memory eightDecimals
    ) internal {
        string memory json = "deployment";
        vm.serializeUint(json, "chainId", block.chainid);
        vm.serializeAddress(json, "pool", address(pool));
        vm.serializeAddress(json, "settlement", address(settlement));
        vm.serializeAddress(json, "oracle", address(oracle));
        vm.serializeAddress(json, "stablecoin", address(stablecoin));
        vm.serializeAddress(json, "stablecoinFeed", address(stableFeed));
        vm.serializeAddress(json, "feeToken", address(feeToken.token));
        vm.serializeAddress(json, "feeTokenFeed", address(feeToken.feed));
        vm.serializeAddress(json, "feeTokenAmm", address(feeToken.amm));
        vm.serializeAddress(json, "sixDecimalToken", address(sixDecimals.token));
        vm.serializeAddress(json, "sixDecimalTokenFeed", address(sixDecimals.feed));
        vm.serializeAddress(json, "sixDecimalTokenAmm", address(sixDecimals.amm));
        vm.serializeAddress(json, "eightDecimalToken", address(eightDecimals.token));
        vm.serializeAddress(json, "eightDecimalTokenFeed", address(eightDecimals.feed));
        string memory output = vm.serializeAddress(json, "eightDecimalTokenAmm", address(eightDecimals.amm));

        vm.writeJson(output, string.concat("./deployments/", vm.toString(block.chainid), ".json"));
    }
}
