// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

import {IOracleAdapter} from "../interfaces/IOracleAdapter.sol";
import {IPriceFeed} from "../interfaces/IPriceFeed.sol";
import {ITwapSource} from "../interfaces/ITwapSource.sol";

/// @notice Prices assets from a Chainlink-style feed and freezes the protocol when a feed or its TWAP cross-check looks wrong.
contract OracleAdapter is IOracleAdapter, Ownable {
    uint256 private constant WAD = 1e18;
    uint256 private constant BPS = 10_000;

    // Ordered so the hot fields share one storage slot, every oracle read pays for it.
    struct Source {
        IPriceFeed feed;
        uint8 feedDecimals;
        uint32 maxStaleness;
        uint32 twapWindow;
        uint16 maxDeviationBps;
        ITwapSource twapSource;
        int128 minAnswer;
        int128 maxAnswer;
    }

    mapping(address asset => Source) public sources;
    address[] public assets;

    event SourceSet(address indexed asset, address feed, address twapSource);

    error InvalidSource();
    error InvalidPrice(address asset);

    constructor(address owner_) Ownable(owner_) {}

    /// @notice Sets the feed for an asset. Leave twapSource empty for assets with no AMM, like the stablecoin.
    function setSource(address asset, Source calldata source) external onlyOwner {
        if (address(source.feed) == address(0) || source.minAnswer > source.maxAnswer) revert InvalidSource();
        if (address(source.twapSource) != address(0) && source.twapWindow == 0) revert InvalidSource();

        if (address(sources[asset].feed) == address(0)) assets.push(asset);

        Source memory stored = source;
        stored.feedDecimals = source.feed.decimals();
        sources[asset] = stored;

        emit SourceSet(asset, address(source.feed), address(source.twapSource));
    }

    /// @notice Frozen as soon as any listed asset has an invalid feed or a TWAP that disagrees with it.
    function state() external view returns (State) {
        uint256 length = assets.length;
        for (uint256 i = 0; i < length; i++) {
            (bool valid,) = _read(assets[i]);
            if (!valid) return State.Frozen;
        }
        return State.Normal;
    }

    function getPrice(address asset) external view returns (uint256) {
        (bool valid, uint256 price) = _read(asset);
        if (!valid) revert InvalidPrice(asset);
        return price;
    }

    function assetCount() external view returns (uint256) {
        return assets.length;
    }

    function _read(address asset) internal view returns (bool valid, uint256 price) {
        Source storage source = sources[asset];
        if (address(source.feed) == address(0)) return (false, 0);

        try source.feed.latestRoundData() returns (uint80, int256 answer, uint256, uint256 updatedAt, uint80) {
            if (answer <= 0 || answer < source.minAnswer || answer > source.maxAnswer) return (false, 0);
            // forge-lint: disable-next-line(block-timestamp)
            if (block.timestamp > updatedAt + source.maxStaleness) return (false, 0);
            // forge-lint: disable-next-line(unsafe-typecast)
            price = (uint256(answer) * WAD) / (10 ** source.feedDecimals);
        } catch {
            return (false, 0);
        }

        if (address(source.twapSource) == address(0)) return (true, price);

        try source.twapSource.consult(source.twapWindow) returns (uint256 twap) {
            if (twap == 0) return (false, 0);
            uint256 difference = price > twap ? price - twap : twap - price;
            if (difference * BPS > price * source.maxDeviationBps) return (false, 0);
        } catch {
            return (false, 0);
        }

        return (true, price);
    }
}
