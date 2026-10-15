// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {IOracleAdapter} from "../interfaces/IOracleAdapter.sol";
import {InterestRateModel} from "../libraries/InterestRateModel.sol";

/// @notice All pool state lives in one ERC-7201 namespaced struct, so the implementation declares no state variables.
/// @dev Upgrades may only append fields to the end of a struct. Never reorder or remove.
library VigilStorage {
    // keccak256(abi.encode(uint256(keccak256("vigil.storage.PoolStorage")) - 1)) & ~bytes32(uint256(0xff))
    bytes32 internal constant POOL_STORAGE_SLOT = 0x8fd6f20a40f8005f935934dc5768fdf271775914f110500764ba439dfca05a00;
    uint256 internal constant MAX_COLLATERAL_ASSETS = 3;

    struct Position {
        uint256 scaledDebt;
        uint256 scaledSupply;
        // Bit i set means collateral asset i counts towards the health factor.
        uint256 collateralBitmap;
        mapping(uint256 assetId => uint256 amount) collateral;
    }

    struct AssetConfig {
        bool listed;
        uint8 id;
        uint8 decimals;
        uint16 liquidationThresholdBps;
    }

    struct PoolStorage {
        IOracleAdapter oracle;
        address debtAsset;
        address settlement;
        uint8 debtDecimals;
        bool paused;
        uint8 collateralCount;
        uint256 reserveFactorBps;
        InterestRateModel.RateParams rate;
        uint256 borrowIndex;
        uint256 liquidityIndex;
        uint256 lastAccrual;
        uint256 totalScaledDebt;
        uint256 totalScaledSupply;
        uint256 reserve;
        uint256 deficit;
        address[3] collateralAssets;
        mapping(address asset => AssetConfig) assets;
        mapping(address account => Position) positions;
    }

    function layout() internal pure returns (PoolStorage storage poolStorage) {
        bytes32 slot = POOL_STORAGE_SLOT;
        assembly {
            poolStorage.slot := slot
        }
    }
}
