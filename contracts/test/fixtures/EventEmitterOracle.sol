// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Test fixture for the Rust indexer. It emits a price event on demand, see EventEmitterPool. It is not the
/// real oracle, see src/oracle/OracleAdapter.sol.
contract EventEmitterOracle {
    event PriceUpdated(address indexed asset, uint256 price, uint256 timestamp);

    function emitPriceUpdated(address asset, uint256 price, uint256 timestamp) external {
        emit PriceUpdated(asset, price, timestamp);
    }
}
