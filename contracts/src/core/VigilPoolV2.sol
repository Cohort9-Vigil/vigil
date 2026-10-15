// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {VigilPool} from "./VigilPool.sol";
import {VigilStorage} from "./VigilStorage.sol";
import {IOracleAdapter} from "../interfaces/IOracleAdapter.sol";
import {InterestRateModel} from "../libraries/InterestRateModel.sol";

/// @notice Same layout as V1 except Position gains one trailing field. Every other field must stay where it is.
library VigilStorageV2 {
    struct Position {
        uint256 scaledDebt;
        uint256 scaledSupply;
        uint256 collateralBitmap;
        mapping(uint256 assetId => uint256 amount) collateral;
        uint256 lastActionBlock;
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
        mapping(address asset => VigilStorage.AssetConfig) assets;
        mapping(address account => Position) positions;
    }

    function layout() internal pure returns (PoolStorage storage poolStorage) {
        bytes32 slot = VigilStorage.POOL_STORAGE_SLOT;
        assembly {
            poolStorage.slot := slot
        }
    }
}

/// @notice Upgrade target that records the last block each account touched the pool.
contract VigilPoolV2 is VigilPool {
    event ActionRecorded(address indexed account, uint256 blockNumber);

    function recordAction() external {
        VigilStorageV2.layout().positions[msg.sender].lastActionBlock = block.number;
        emit ActionRecorded(msg.sender, block.number);
    }

    /// @notice Zero for accounts that have not acted since the upgrade, their old balances are untouched.
    function lastActionBlock(address account) external view returns (uint256) {
        return VigilStorageV2.layout().positions[account].lastActionBlock;
    }
}
