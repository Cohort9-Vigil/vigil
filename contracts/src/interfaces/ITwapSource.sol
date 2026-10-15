// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice A source of time-weighted average prices.
interface ITwapSource {
    /// @return price Average price over the last `window` seconds, scaled to 1e18.
    function consult(uint256 window) external view returns (uint256 price);
}
