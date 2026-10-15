// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";

import {VigilStorage} from "./VigilStorage.sol";
import {IVigilPool} from "../interfaces/IVigilPool.sol";
import {IOracleAdapter} from "../interfaces/IOracleAdapter.sol";
import {CollateralBitmap} from "../libraries/CollateralBitmap.sol";
import {FixedPointMath} from "../libraries/FixedPointMath.sol";
import {InterestRateModel} from "../libraries/InterestRateModel.sol";
import {SafeTransfer} from "../libraries/SafeTransfer.sol";
import {TransientLock} from "../libraries/TransientLock.sol";

/// @notice Lending pool with one stablecoin debt asset and up to three collateral assets.
contract VigilPool is IVigilPool, Initializable, OwnableUpgradeable, UUPSUpgradeable {
    using FixedPointMath for uint256;
    using CollateralBitmap for uint256;

    uint256 public constant BPS = 10_000;
    uint256 public constant HEALTH_FACTOR_ONE = 1e18;

    struct PoolState {
        uint256 borrowIndex;
        uint256 liquidityIndex;
        uint256 totalScaledDebt;
        uint256 totalScaledSupply;
        uint256 reserve;
        uint256 deficit;
        bool paused;
    }

    struct Accrual {
        uint256 borrowIndex;
        uint256 liquidityIndex;
        uint256 reserveIncrease;
    }

    event Supplied(address indexed account, uint256 amount, uint256 scaledAmount);
    event SupplyWithdrawn(address indexed account, uint256 amount, uint256 scaledAmount);
    event CollateralDeposited(address indexed account, address indexed asset, uint256 amount);
    event CollateralWithdrawn(address indexed account, address indexed asset, uint256 amount);
    event CollateralEnabledChanged(address indexed account, address indexed asset, bool enabled);
    event Borrowed(address indexed account, uint256 amount, uint256 scaledAmount);
    event Repaid(address indexed account, uint256 amount, uint256 scaledAmount);
    event Liquidated(
        address indexed borrower,
        address indexed liquidator,
        address indexed collateralAsset,
        uint256 repayAmount,
        uint256 collateralAmount
    );
    event BadDebtWrittenOff(address indexed borrower, uint256 coveredByReserve, uint256 deficit);
    event InterestAccrued(uint256 borrowIndex, uint256 liquidityIndex, uint256 reserveIncrease);
    event CollateralListed(address indexed asset, uint8 id, uint16 liquidationThresholdBps);
    event SettlementSet(address indexed settlement);
    event Paused();
    event Unpaused();

    error ReentrantCall();
    error PoolPaused();
    error OracleFrozen();
    error OracleNotNormal();
    error NotSettlement();
    error ZeroAmount();
    error AssetNotListed();
    error AssetAlreadyListed();
    error TooManyCollateralAssets();
    error InvalidParameter();
    error HealthFactorTooLow();
    error InsufficientBalance();
    error InsufficientLiquidity();
    error CollateralRemaining();

    modifier nonReentrant() {
        if (!TransientLock.lock()) revert ReentrantCall();
        _;
        TransientLock.unlock();
    }

    modifier whenNotPaused() {
        if (VigilStorage.layout().paused) revert PoolPaused();
        _;
    }

    // Frozen only allows repay and adding collateral, so everything else carries this check.
    modifier whenOracleNormal() {
        if (VigilStorage.layout().oracle.state() == IOracleAdapter.State.Frozen) revert OracleFrozen();
        _;
    }

    modifier onlySettlement() {
        if (msg.sender != VigilStorage.layout().settlement) revert NotSettlement();
        _;
    }

    /// @custom:oz-upgrades-unsafe-allow constructor
    constructor() {
        _disableInitializers();
    }

    /// @dev The debt asset must be a plain ERC20 without transfer fees, repayments are credited at face value.
    function initialize(
        address owner_,
        address debtAsset_,
        IOracleAdapter oracle_,
        uint256 reserveFactorBps_,
        InterestRateModel.RateParams calldata rate_
    ) external initializer {
        if (reserveFactorBps_ > BPS) revert InvalidParameter();
        InterestRateModel.validate(rate_);

        __Ownable_init(owner_);

        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        s.debtAsset = debtAsset_;
        s.debtDecimals = IERC20Metadata(debtAsset_).decimals();
        s.oracle = oracle_;
        s.reserveFactorBps = reserveFactorBps_;
        s.rate = rate_;
        s.borrowIndex = FixedPointMath.RAY;
        s.liquidityIndex = FixedPointMath.RAY;
        s.lastAccrual = block.timestamp;
    }

    function listCollateral(address asset, uint16 liquidationThresholdBps) external onlyOwner {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        if (s.assets[asset].listed || asset == s.debtAsset) revert AssetAlreadyListed();
        if (s.collateralCount >= VigilStorage.MAX_COLLATERAL_ASSETS) revert TooManyCollateralAssets();
        if (liquidationThresholdBps == 0 || liquidationThresholdBps > BPS) revert InvalidParameter();

        uint8 id = s.collateralCount;
        s.assets[asset] = VigilStorage.AssetConfig({
            listed: true,
            id: id,
            decimals: IERC20Metadata(asset).decimals(),
            liquidationThresholdBps: liquidationThresholdBps
        });
        s.collateralAssets[id] = asset;
        s.collateralCount = id + 1;

        emit CollateralListed(asset, id, liquidationThresholdBps);
    }

    function setSettlement(address settlement) external onlyOwner {
        VigilStorage.layout().settlement = settlement;
        emit SettlementSet(settlement);
    }

    function pause() external onlyOwner {
        VigilStorage.layout().paused = true;
        emit Paused();
    }

    /// @notice Unpausing into a frozen oracle would reopen borrowing on a bad price, so it is refused.
    function unpause() external onlyOwner {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        if (s.oracle.state() != IOracleAdapter.State.Normal) revert OracleNotNormal();
        s.paused = false;
        emit Unpaused();
    }

    /// @notice Lends the debt asset to the pool to earn interest.
    function supply(uint256 amount) external nonReentrant whenNotPaused whenOracleNormal {
        if (amount == 0) revert ZeroAmount();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        _accrue(s);

        uint256 received = _pull(s.debtAsset, msg.sender, amount);
        uint256 scaled = received.rayDivDown(s.liquidityIndex);

        s.positions[msg.sender].scaledSupply += scaled;
        s.totalScaledSupply += scaled;

        emit Supplied(msg.sender, received, scaled);
    }

    function withdrawSupply(uint256 amount) external nonReentrant whenNotPaused whenOracleNormal {
        if (amount == 0) revert ZeroAmount();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        _accrue(s);

        VigilStorage.Position storage position = s.positions[msg.sender];
        if (amount > position.scaledSupply.rayMulDown(s.liquidityIndex)) revert InsufficientBalance();
        if (amount > _availableLiquidity(s)) revert InsufficientLiquidity();

        // Burning rounds up so a withdrawal can never take more than it costs.
        uint256 scaled = amount.rayDivUp(s.liquidityIndex);
        if (scaled > position.scaledSupply) scaled = position.scaledSupply;

        position.scaledSupply -= scaled;
        s.totalScaledSupply -= scaled;

        emit SupplyWithdrawn(msg.sender, amount, scaled);
        SafeTransfer.safeTransfer(s.debtAsset, msg.sender, amount);
    }

    /// @notice Allowed while the oracle is frozen, adding collateral only makes the pool safer.
    function depositCollateral(address asset, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        VigilStorage.AssetConfig memory config = s.assets[asset];
        if (!config.listed) revert AssetNotListed();

        // Credit what arrived, a fee-on-transfer token delivers less than the amount argument.
        uint256 received = _pull(asset, msg.sender, amount);

        VigilStorage.Position storage position = s.positions[msg.sender];
        position.collateral[config.id] += received;
        position.collateralBitmap = position.collateralBitmap.set(config.id);

        emit CollateralDeposited(msg.sender, asset, received);
    }

    function withdrawCollateral(address asset, uint256 amount) external nonReentrant whenNotPaused whenOracleNormal {
        if (amount == 0) revert ZeroAmount();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        VigilStorage.AssetConfig memory config = s.assets[asset];
        if (!config.listed) revert AssetNotListed();

        VigilStorage.Position storage position = s.positions[msg.sender];
        uint256 balance = position.collateral[config.id];
        if (amount > balance) revert InsufficientBalance();

        _accrue(s);
        position.collateral[config.id] = balance - amount;
        if (balance == amount) position.collateralBitmap = position.collateralBitmap.clear(config.id);

        if (_healthFactor(s, msg.sender, s.borrowIndex) < HEALTH_FACTOR_ONE) revert HealthFactorTooLow();

        emit CollateralWithdrawn(msg.sender, asset, amount);
        SafeTransfer.safeTransfer(asset, msg.sender, amount);
    }

    function setCollateralEnabled(address asset, bool enabled) external nonReentrant whenNotPaused whenOracleNormal {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        VigilStorage.AssetConfig memory config = s.assets[asset];
        if (!config.listed) revert AssetNotListed();

        _accrue(s);
        VigilStorage.Position storage position = s.positions[msg.sender];
        position.collateralBitmap =
            enabled ? position.collateralBitmap.set(config.id) : position.collateralBitmap.clear(config.id);

        if (!enabled && _healthFactor(s, msg.sender, s.borrowIndex) < HEALTH_FACTOR_ONE) revert HealthFactorTooLow();

        emit CollateralEnabledChanged(msg.sender, asset, enabled);
    }

    function borrow(uint256 amount) external nonReentrant whenNotPaused whenOracleNormal {
        if (amount == 0) revert ZeroAmount();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        _accrue(s);
        if (amount > _availableLiquidity(s)) revert InsufficientLiquidity();

        // Debt rounds up, so the borrower always owes at least what was lent.
        uint256 scaled = amount.rayDivUp(s.borrowIndex);
        s.positions[msg.sender].scaledDebt += scaled;
        s.totalScaledDebt += scaled;

        if (_healthFactor(s, msg.sender, s.borrowIndex) < HEALTH_FACTOR_ONE) revert HealthFactorTooLow();

        emit Borrowed(msg.sender, amount, scaled);
        SafeTransfer.safeTransfer(s.debtAsset, msg.sender, amount);
    }

    /// @notice Allowed in every state, repaying is always good for the pool.
    function repay(uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        _accrue(s);

        VigilStorage.Position storage position = s.positions[msg.sender];
        uint256 debt = position.scaledDebt.rayMulUp(s.borrowIndex);
        if (amount > debt) amount = debt;
        if (amount == 0) revert ZeroAmount();

        uint256 received = _pull(s.debtAsset, msg.sender, amount);

        // Reducing the scaled debt rounds down, a partial repayment never wipes more debt than it paid.
        uint256 scaled = received.rayDivDown(s.borrowIndex);
        if (scaled > position.scaledDebt) scaled = position.scaledDebt;

        position.scaledDebt -= scaled;
        s.totalScaledDebt -= scaled;

        emit Repaid(msg.sender, received, scaled);
    }

    /// @inheritdoc IVigilPool
    function applyLiquidation(
        address borrower,
        address collateralAsset,
        address liquidator,
        uint256 repayAmount,
        uint256 collateralAmount
    ) external onlySettlement nonReentrant returns (uint256 healthFactorAfter, bool collateralLeft) {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        VigilStorage.AssetConfig memory config = s.assets[collateralAsset];
        if (!config.listed) revert AssetNotListed();
        _accrue(s);

        VigilStorage.Position storage position = s.positions[borrower];
        uint256 balance = position.collateral[config.id];
        if (collateralAmount > balance) revert InsufficientBalance();

        uint256 scaled = repayAmount.rayDivDown(s.borrowIndex);
        if (scaled > position.scaledDebt) scaled = position.scaledDebt;

        position.scaledDebt -= scaled;
        s.totalScaledDebt -= scaled;
        position.collateral[config.id] = balance - collateralAmount;
        if (balance == collateralAmount) position.collateralBitmap = position.collateralBitmap.clear(config.id);

        collateralLeft = _hasCollateral(s, borrower);
        if (collateralLeft) healthFactorAfter = _healthFactor(s, borrower, s.borrowIndex);

        emit Liquidated(borrower, liquidator, collateralAsset, repayAmount, collateralAmount);
        SafeTransfer.safeTransfer(collateralAsset, liquidator, collateralAmount);
    }

    /// @inheritdoc IVigilPool
    function writeOffBadDebt(address borrower)
        external
        onlySettlement
        nonReentrant
        returns (uint256 coveredByReserve, uint256 deficitAdded)
    {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        if (_hasCollateral(s, borrower)) revert CollateralRemaining();
        _accrue(s);

        VigilStorage.Position storage position = s.positions[borrower];
        uint256 scaled = position.scaledDebt;
        uint256 debt = scaled.rayMulUp(s.borrowIndex);

        // The reserve absorbs what it can. What it cannot is recorded, never reverted on.
        coveredByReserve = debt < s.reserve ? debt : s.reserve;
        deficitAdded = debt - coveredByReserve;

        s.reserve -= coveredByReserve;
        s.deficit += deficitAdded;
        position.scaledDebt = 0;
        s.totalScaledDebt -= scaled;

        emit BadDebtWrittenOff(borrower, coveredByReserve, deficitAdded);
    }

    /// @inheritdoc IVigilPool
    function getHealthFactor(address borrower) external view returns (uint256) {
        _requireUnlocked();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        return _healthFactor(s, borrower, _previewAccrual(s).borrowIndex);
    }

    /// @notice Oracle price, readable only outside a call into the pool so token callbacks cannot see mid-update state.
    function getPrice(address asset) external view returns (uint256) {
        _requireUnlocked();
        return VigilStorage.layout().oracle.getPrice(asset);
    }

    /// @inheritdoc IVigilPool
    function getDebt(address borrower) external view returns (uint256) {
        _requireUnlocked();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        return s.positions[borrower].scaledDebt.rayMulUp(_previewAccrual(s).borrowIndex);
    }

    /// @inheritdoc IVigilPool
    function getLiquidationData(address borrower, address collateralAsset)
        external
        view
        returns (uint256 healthFactor, uint256 debt, uint256 collateralBalance)
    {
        _requireUnlocked();
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        VigilStorage.Position storage position = s.positions[borrower];
        uint256 currentBorrowIndex = _previewAccrual(s).borrowIndex;

        healthFactor = _healthFactor(s, borrower, currentBorrowIndex);
        debt = position.scaledDebt.rayMulUp(currentBorrowIndex);

        VigilStorage.AssetConfig memory config = s.assets[collateralAsset];
        if (config.listed) collateralBalance = position.collateral[config.id];
    }

    /// @inheritdoc IVigilPool
    function getCollateralBalance(address borrower, address asset) external view returns (uint256) {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        VigilStorage.AssetConfig memory config = s.assets[asset];
        if (!config.listed) return 0;
        return s.positions[borrower].collateral[config.id];
    }

    /// @inheritdoc IVigilPool
    function hasCollateral(address borrower) external view returns (bool) {
        return _hasCollateral(VigilStorage.layout(), borrower);
    }

    /// @inheritdoc IVigilPool
    function debtAsset() external view returns (address) {
        return VigilStorage.layout().debtAsset;
    }

    function poolState() external view returns (PoolState memory) {
        VigilStorage.PoolStorage storage s = VigilStorage.layout();
        return PoolState({
            borrowIndex: s.borrowIndex,
            liquidityIndex: s.liquidityIndex,
            totalScaledDebt: s.totalScaledDebt,
            totalScaledSupply: s.totalScaledSupply,
            reserve: s.reserve,
            deficit: s.deficit,
            paused: s.paused
        });
    }

    function positionOf(address account)
        external
        view
        returns (uint256 scaledDebt, uint256 scaledSupply, uint256 bitmap)
    {
        VigilStorage.Position storage position = VigilStorage.layout().positions[account];
        return (position.scaledDebt, position.scaledSupply, position.collateralBitmap);
    }

    function assetConfig(address asset) external view returns (VigilStorage.AssetConfig memory) {
        return VigilStorage.layout().assets[asset];
    }

    function _authorizeUpgrade(address) internal override onlyOwner {}

    function _requireUnlocked() internal view {
        if (TransientLock.isLocked()) revert ReentrantCall();
    }

    function _pull(address token, address from, uint256 amount) internal returns (uint256 received) {
        uint256 balanceBefore = IERC20(token).balanceOf(address(this));
        SafeTransfer.safeTransferFrom(token, from, address(this), amount);
        received = IERC20(token).balanceOf(address(this)) - balanceBefore;
    }

    // The reserve is owed to the protocol, so it is not lendable.
    function _availableLiquidity(VigilStorage.PoolStorage storage s) internal view returns (uint256) {
        uint256 cash = IERC20(s.debtAsset).balanceOf(address(this));
        return cash > s.reserve ? cash - s.reserve : 0;
    }

    function _hasCollateral(VigilStorage.PoolStorage storage s, address borrower) internal view returns (bool) {
        VigilStorage.Position storage position = s.positions[borrower];
        for (uint256 id = 0; id < s.collateralCount; id++) {
            if (position.collateral[id] != 0) return true;
        }
        return false;
    }

    /// @dev Collateral value rounds down and debt value rounds up, so the health factor never flatters the borrower.
    function _healthFactor(VigilStorage.PoolStorage storage s, address borrower, uint256 currentBorrowIndex)
        internal
        view
        returns (uint256)
    {
        VigilStorage.Position storage position = s.positions[borrower];
        if (position.scaledDebt == 0) return type(uint256).max;

        uint256 debt = position.scaledDebt.rayMulUp(currentBorrowIndex);
        uint256 debtValue = FixedPointMath.mulDivUp(debt, s.oracle.getPrice(s.debtAsset), 10 ** s.debtDecimals);

        uint256 borrowingPower;
        uint256 bitmap = position.collateralBitmap;
        // Only enabled assets are visited, so cost follows what the user enabled, not what is listed.
        while (bitmap != 0) {
            uint256 id = bitmap.lowestSetBit();
            bitmap = bitmap.clearLowest();

            address asset = s.collateralAssets[id];
            VigilStorage.AssetConfig memory config = s.assets[asset];
            uint256 value =
                FixedPointMath.mulDivDown(position.collateral[id], s.oracle.getPrice(asset), 10 ** config.decimals);
            borrowingPower += (value * config.liquidationThresholdBps) / BPS;
        }

        return borrowingPower.wadDivDown(debtValue);
    }

    function _accrue(VigilStorage.PoolStorage storage s) internal {
        Accrual memory accrual = _previewAccrual(s);
        s.borrowIndex = accrual.borrowIndex;
        s.liquidityIndex = accrual.liquidityIndex;
        s.reserve += accrual.reserveIncrease;

        // forge-lint: disable-next-line(block-timestamp)
        if (block.timestamp != s.lastAccrual) {
            s.lastAccrual = block.timestamp;
            emit InterestAccrued(accrual.borrowIndex, accrual.liquidityIndex, accrual.reserveIncrease);
        }
    }

    /// @dev Shared by state changes and views, so both always agree on the indexes.
    function _previewAccrual(VigilStorage.PoolStorage storage s) internal view returns (Accrual memory accrual) {
        accrual.borrowIndex = s.borrowIndex;
        accrual.liquidityIndex = s.liquidityIndex;

        uint256 elapsed = block.timestamp - s.lastAccrual;
        if (elapsed == 0 || s.totalScaledDebt == 0) return accrual;

        uint256 totalDebt = s.totalScaledDebt.rayMulUp(s.borrowIndex);
        uint256 totalSupply = s.totalScaledSupply.rayMulDown(s.liquidityIndex);

        uint256 utilization = FixedPointMath.RAY;
        if (totalSupply != 0) {
            utilization = totalDebt.rayDivDown(totalSupply);
            if (utilization > FixedPointMath.RAY) utilization = FixedPointMath.RAY;
        }

        uint256 ratePerSecond = InterestRateModel.borrowRatePerSecond(utilization, s.rate);
        uint256 growth = FixedPointMath.compoundedGrowth(ratePerSecond, elapsed);
        accrual.borrowIndex = s.borrowIndex.rayMulUp(growth);

        uint256 interest = s.totalScaledDebt.rayMulUp(accrual.borrowIndex) - totalDebt;
        uint256 reserveCut = (interest * s.reserveFactorBps) / BPS;

        // With no suppliers left the whole interest goes to the reserve.
        if (totalSupply == 0) {
            accrual.reserveIncrease = interest;
            return accrual;
        }

        accrual.reserveIncrease = reserveCut;
        accrual.liquidityIndex =
            s.liquidityIndex + FixedPointMath.mulDivDown(s.liquidityIndex, interest - reserveCut, totalSupply);
    }
}
