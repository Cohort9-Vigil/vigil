// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice The part of the oracle the settlement contract depends on.
interface IOracleAdapter {
    enum State {
        Normal,
        Frozen
    }

    function state() external view returns (State);

    /// @return price Price of one whole token in a common base, scaled to 1e18.
    function getPrice(address asset) external view returns (uint256 price);
}
