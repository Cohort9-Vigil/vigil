// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test} from "forge-std/Test.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

import {VigilPool} from "../src/core/VigilPool.sol";
import {OracleAdapter} from "../src/oracle/OracleAdapter.sol";
import {MockChainlinkFeed} from "../src/mocks/MockChainlinkFeed.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {LiquidationSettlement} from "../src/liquidation/LiquidationSettlement.sol";
import {IOracleAdapter} from "../src/interfaces/IOracleAdapter.sol";
import {IVigilPool} from "../src/interfaces/IVigilPool.sol";
import {IPriceFeed} from "../src/interfaces/IPriceFeed.sol";
import {ITwapSource} from "../src/interfaces/ITwapSource.sol";
import {InterestRateModel} from "../src/libraries/InterestRateModel.sol";

/// @notice Deploys a USDC / WETH pool with a funded lender, shared by most tests.
abstract contract VigilBase is Test {
    uint256 internal constant LIQUIDATOR_KEY = 0xA11CE;
    uint256 internal constant AUCTION_DURATION = 50;
    uint256 internal constant MAX_BONUS_BPS = 1000;
    uint256 internal constant DUST_THRESHOLD = 10e6;
    uint256 internal constant RAY = 1e27;
    uint256 internal constant WAD = 1e18;

    bytes32 internal constant INTENT_TYPEHASH = keccak256(
        "LiquidationIntent(address borrower,address collateralAsset,uint256 maxRepay,uint256 minCollateralOut,uint256 nonce,uint256 deadline)"
    );

    MockERC20 internal usdc;
    MockERC20 internal weth;
    MockChainlinkFeed internal usdcFeed;
    MockChainlinkFeed internal wethFeed;
    OracleAdapter internal oracle;
    VigilPool internal pool;
    LiquidationSettlement internal settlement;

    address internal lender = makeAddr("lender");
    address internal borrower = makeAddr("borrower");
    address internal liquidator = vm.addr(LIQUIDATOR_KEY);

    function setUp() public virtual {
        vm.warp(1_000_000);

        usdc = new MockERC20("USD Coin", "USDC", 6);
        weth = new MockERC20("Wrapped Ether", "WETH", 18);
        usdcFeed = new MockChainlinkFeed(8, 1e8);
        wethFeed = new MockChainlinkFeed(8, 2000e8);

        oracle = new OracleAdapter(address(this));
        oracle.setSource(address(usdc), plainSource(usdcFeed));
        oracle.setSource(address(weth), plainSource(wethFeed));

        InterestRateModel.RateParams memory rate = InterestRateModel.RateParams(0.02e27, 0.04e27, 0.75e27, 0.8e27);
        bytes memory initData = abi.encodeCall(
            VigilPool.initialize, (address(this), address(usdc), IOracleAdapter(address(oracle)), 1000, rate)
        );
        pool = VigilPool(address(new ERC1967Proxy(address(new VigilPool()), initData)));
        pool.listCollateral(address(weth), 8000);

        settlement = new LiquidationSettlement(
            IVigilPool(address(pool)), IOracleAdapter(address(oracle)), AUCTION_DURATION, MAX_BONUS_BPS, DUST_THRESHOLD
        );
        pool.setSettlement(address(settlement));

        usdc.mint(lender, 1_000_000e6);
        vm.startPrank(lender);
        usdc.approve(address(pool), type(uint256).max);
        pool.supply(1_000_000e6);
        vm.stopPrank();

        usdc.mint(liquidator, 500_000e6);
        vm.prank(liquidator);
        usdc.approve(address(settlement), type(uint256).max);
    }

    function plainSource(MockChainlinkFeed feed) internal pure returns (OracleAdapter.Source memory) {
        return OracleAdapter.Source({
            feed: IPriceFeed(address(feed)),
            feedDecimals: 0,
            maxStaleness: 1 days,
            twapWindow: 0,
            maxDeviationBps: 0,
            twapSource: ITwapSource(address(0)),
            minAnswer: 1,
            maxAnswer: 1e14
        });
    }

    /// @dev Lists a new collateral token priced in whole dollars.
    function listToken(address token, uint256 priceUsd) internal returns (MockChainlinkFeed feed) {
        feed = new MockChainlinkFeed(8, int128(int256(priceUsd * 1e8)));
        oracle.setSource(token, plainSource(feed));
        pool.listCollateral(token, 8000);
    }

    function openPosition(address user, uint256 wethAmount, uint256 borrowAmount) internal {
        weth.mint(user, wethAmount);
        vm.startPrank(user);
        weth.approve(address(pool), type(uint256).max);
        pool.depositCollateral(address(weth), wethAmount);
        if (borrowAmount > 0) pool.borrow(borrowAmount);
        vm.stopPrank();
    }

    function refreshUsdcFeed() public {
        usdcFeed.setAnswer(1e8);
    }

    function setWethPrice(uint256 priceUsd) internal {
        wethFeed.setAnswer(int128(int256(priceUsd * 1e8)));
    }

    function makeIntent(address target, uint256 maxRepay, uint256 nonce)
        public
        view
        returns (LiquidationSettlement.LiquidationIntent memory)
    {
        return LiquidationSettlement.LiquidationIntent({
            borrower: target,
            collateralAsset: address(weth),
            maxRepay: maxRepay,
            minCollateralOut: 0,
            nonce: nonce,
            deadline: block.timestamp + 1 hours
        });
    }

    function signIntent(LiquidationSettlement.LiquidationIntent memory intent) public view returns (bytes memory) {
        bytes32 domainSeparator = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("VigilLiquidation"),
                keccak256("1"),
                block.chainid,
                address(settlement)
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
        (uint8 v, bytes32 r, bytes32 s) =
            vm.sign(LIQUIDATOR_KEY, keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash)));
        return abi.encodePacked(r, s, v);
    }

    /// @dev Crashes WETH, opens the auction and lets some blocks pass so the bonus is above zero.
    function startAuctionAndWait(address target, uint256 blocksToWait) internal {
        settlement.startAuction(target);
        vm.roll(block.number + blocksToWait);
    }

    function liquidate(address target, uint256 maxRepay, uint256 nonce) internal {
        LiquidationSettlement.LiquidationIntent memory intent = makeIntent(target, maxRepay, nonce);
        settlement.executeLiquidation(intent, signIntent(intent));
    }
}
