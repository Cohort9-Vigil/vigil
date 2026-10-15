// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Test fixture for the Rust indexer. It emits fixed events on demand so the indexer's integration and
/// reorg tests have something to deploy on anvil. It is not the real pool, see src/core/VigilPool.sol.
contract EventEmitterPool {
    event Deposit(address indexed user, address indexed asset, uint256 amount);
    event Borrow(address indexed user, uint256 amount);
    event Repay(address indexed user, uint256 amount);
    event LiquidationCall(
        address indexed collateralAsset,
        address indexed user,
        uint256 debtToCover,
        uint256 liquidatedCollateralAmount,
        address liquidator
    );

    function emitDeposit(address user, address asset, uint256 amount) external {
        emit Deposit(user, asset, amount);
    }

    function emitBorrow(address user, uint256 amount) external {
        emit Borrow(user, amount);
    }

    function emitRepay(address user, uint256 amount) external {
        emit Repay(user, amount);
    }

    function emitLiquidationCall(
        address collateralAsset,
        address user,
        uint256 debtToCover,
        uint256 liquidatedCollateralAmount,
        address liquidator
    ) external {
        emit LiquidationCall(collateralAsset, user, debtToCover, liquidatedCollateralAmount, liquidator);
    }
}
