// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice Helpers for a uint256 where bit i means collateral asset i is enabled.
library CollateralBitmap {
    function set(uint256 bitmap, uint256 id) internal pure returns (uint256) {
        return bitmap | (uint256(1) << id);
    }

    function clear(uint256 bitmap, uint256 id) internal pure returns (uint256) {
        return bitmap & ~(uint256(1) << id);
    }

    /// @dev The bitmap must not be zero.
    function lowestSetBit(uint256 bitmap) internal pure returns (uint256 id) {
        uint256 isolated;
        unchecked {
            isolated = bitmap & (~bitmap + 1);
        }
        if (isolated >> 128 != 0) {
            id += 128;
            isolated >>= 128;
        }
        if (isolated >> 64 != 0) {
            id += 64;
            isolated >>= 64;
        }
        if (isolated >> 32 != 0) {
            id += 32;
            isolated >>= 32;
        }
        if (isolated >> 16 != 0) {
            id += 16;
            isolated >>= 16;
        }
        if (isolated >> 8 != 0) {
            id += 8;
            isolated >>= 8;
        }
        if (isolated >> 4 != 0) {
            id += 4;
            isolated >>= 4;
        }
        if (isolated >> 2 != 0) {
            id += 2;
            isolated >>= 2;
        }
        if (isolated >> 1 != 0) id += 1;
    }

    /// @dev Drops the lowest set bit, so a loop of lowestSetBit and clearLowest visits only enabled assets.
    function clearLowest(uint256 bitmap) internal pure returns (uint256) {
        return bitmap & (bitmap - 1);
    }
}
