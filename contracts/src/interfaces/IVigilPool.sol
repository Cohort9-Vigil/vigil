// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice The part of the pool the settlement contract depends on.
interface IVigilPool {
    function debtAsset() external view returns (address);

    /// @return healthFactor Scaled to 1e18, so 1e18 is exactly at the liquidation line.
    function getHealthFactor(address borrower) external view returns (uint256 healthFactor);

    /// @return debt Current debt including accrued interest, in debt asset units.
    function getDebt(address borrower) external view returns (uint256 debt);

    function getCollateralBalance(address borrower, address asset) external view returns (uint256 balance);

    function hasCollateral(address borrower) external view returns (bool);

    /// @notice Health factor, debt and collateral balance in one call, so liquidations read the pool once.
    function getLiquidationData(address borrower, address collateralAsset)
        external
        view
        returns (uint256 healthFactor, uint256 debt, uint256 collateralBalance);

    /// @notice Settlement only. The repay amount has already been sent to the pool.
    /// @return healthFactorAfter Zero when no collateral is left.
    /// @return collateralLeft False when the borrower has no collateral of any asset left.
    function applyLiquidation(
        address borrower,
        address collateralAsset,
        address liquidator,
        uint256 repayAmount,
        uint256 collateralAmount
    ) external returns (uint256 healthFactorAfter, bool collateralLeft);

    /// @notice Settlement only. Writes remaining debt off against the reserve and records any shortfall as deficit.
    function writeOffBadDebt(address borrower) external returns (uint256 coveredByReserve, uint256 deficit);
}
