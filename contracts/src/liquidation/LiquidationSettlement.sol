// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {EIP712} from "openzeppelin-contracts/contracts/utils/cryptography/EIP712.sol";
import {ECDSA} from "openzeppelin-contracts/contracts/utils/cryptography/ECDSA.sol";
import {IERC20} from "openzeppelin-contracts/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "openzeppelin-contracts/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {SafeERC20} from "openzeppelin-contracts/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "openzeppelin-contracts/contracts/utils/math/Math.sol";

import {IVigilPool} from "../interfaces/IVigilPool.sol";
import {IOracleAdapter} from "../interfaces/IOracleAdapter.sol";
import {TransientLock} from "../libraries/TransientLock.sol";

/// @notice Runs Dutch auctions on unhealthy positions and settles liquidations from signed intents.
contract LiquidationSettlement is EIP712 {
    using SafeERC20 for IERC20;

    struct LiquidationIntent {
        address borrower;
        address collateralAsset;
        uint256 maxRepay;
        uint256 minCollateralOut;
        uint256 nonce;
        uint256 deadline;
    }

    struct Quote {
        uint256 healthBefore;
        uint256 debt;
        uint256 repayAmount;
        uint256 collateralSeized;
        uint256 bonusBps;
        bool underwater;
    }

    bytes32 public constant INTENT_TYPEHASH = keccak256(
        "LiquidationIntent(address borrower,address collateralAsset,uint256 maxRepay,uint256 minCollateralOut,uint256 nonce,uint256 deadline)"
    );
    uint256 public constant BPS = 10_000;
    uint256 public constant CLOSE_FACTOR_BPS = 5_000;
    uint256 public constant HEALTH_FACTOR_ONE = 1e18;

    IVigilPool public immutable pool;
    IOracleAdapter public immutable oracle;
    address public immutable debtAsset;
    uint256 public immutable auctionDuration;
    uint256 public immutable maxBonusBps;
    uint256 public immutable dustThreshold;

    mapping(address borrower => uint256 blockNumber) public auctionStartBlock;
    // Permit2 layout: the nonce picks a word and a bit inside it, so intents can be used in any order.
    mapping(address liquidator => mapping(uint256 word => uint256 bitmap)) public nonceBitmap;

    event AuctionStarted(address indexed borrower, uint256 startBlock);
    event AuctionReset(address indexed borrower);
    event LiquidationExecuted(
        address indexed borrower,
        address indexed liquidator,
        address indexed collateralAsset,
        uint256 repayAmount,
        uint256 collateralSeized,
        uint256 bonusBps
    );
    event NonceUsed(address indexed liquidator, uint256 nonce);
    event BadDebtSettled(address indexed borrower, uint256 coveredByReserve, uint256 deficit);

    error OracleFrozen();
    error PositionHealthy();
    error AuctionAlreadyActive();
    error NoActiveAuction();
    error IntentExpired();
    error InvalidSignature();
    error NonceAlreadyUsed();
    error NothingToRepay();
    error InsufficientCollateralOut();
    error HealthFactorDecreased();
    error CollateralRemaining();
    error NoDebt();
    error ReentrantCall();

    modifier nonReentrant() {
        if (!TransientLock.lock()) revert ReentrantCall();
        _;
        TransientLock.unlock();
    }

    constructor(
        IVigilPool pool_,
        IOracleAdapter oracle_,
        uint256 auctionDuration_,
        uint256 maxBonusBps_,
        uint256 dustThreshold_
    ) EIP712("VigilLiquidation", "1") {
        pool = pool_;
        oracle = oracle_;
        debtAsset = pool_.debtAsset();
        auctionDuration = auctionDuration_;
        maxBonusBps = maxBonusBps_;
        dustThreshold = dustThreshold_;
    }

    /// @notice Opens an auction on a borrower whose health factor is below 1.
    function startAuction(address borrower) external {
        if (oracle.state() == IOracleAdapter.State.Frozen) revert OracleFrozen();
        if (pool.getHealthFactor(borrower) >= HEALTH_FACTOR_ONE) revert PositionHealthy();
        if (auctionStartBlock[borrower] != 0) revert AuctionAlreadyActive();

        auctionStartBlock[borrower] = block.number;
        emit AuctionStarted(borrower, block.number);
    }

    /// @notice Clears an auction once the position is healthy again, so the next one starts from a zero bonus.
    function resetAuction(address borrower) external {
        if (auctionStartBlock[borrower] == 0) revert NoActiveAuction();
        if (pool.getHealthFactor(borrower) < HEALTH_FACTOR_ONE) revert PositionHealthy();

        delete auctionStartBlock[borrower];
        emit AuctionReset(borrower);
    }

    /// @notice Executes a liquidation signed by the liquidator, who supplies the repayment.
    function executeLiquidation(LiquidationIntent calldata intent, bytes calldata signature) external nonReentrant {
        // forge-lint: disable-next-line(block-timestamp)
        if (block.timestamp > intent.deadline) revert IntentExpired();
        if (oracle.state() == IOracleAdapter.State.Frozen) revert OracleFrozen();

        address liquidator = _recoverSigner(intent, signature);
        _useNonce(liquidator, intent.nonce);

        Quote memory quote = _quote(intent);

        IERC20(debtAsset).safeTransferFrom(liquidator, address(pool), quote.repayAmount);
        (uint256 healthAfter, bool collateralLeft) = pool.applyLiquidation(
            intent.borrower, intent.collateralAsset, liquidator, quote.repayAmount, quote.collateralSeized
        );

        emit LiquidationExecuted(
            intent.borrower,
            liquidator,
            intent.collateralAsset,
            quote.repayAmount,
            quote.collateralSeized,
            quote.bonusBps
        );

        _finishAuction(intent.borrower, quote, healthAfter, collateralLeft);
    }

    /// @notice Writes off debt that no collateral is left to cover.
    function settleBadDebt(address borrower) external nonReentrant {
        if (pool.hasCollateral(borrower)) revert CollateralRemaining();
        if (pool.getDebt(borrower) == 0) revert NoDebt();

        _writeOff(borrower);
    }

    /// @notice Current liquidator bonus for a borrower, zero when no auction is running.
    function currentBonusBps(address borrower) external view returns (uint256) {
        uint256 startBlock = auctionStartBlock[borrower];
        if (startBlock == 0) return 0;
        return _bonusBps(startBlock);
    }

    function _finishAuction(address borrower, Quote memory quote, uint256 healthAfter, bool collateralLeft) internal {
        bool finished = quote.debt == quote.repayAmount;

        if (!finished && !collateralLeft) {
            _writeOff(borrower);
            finished = true;
        } else if (!finished) {
            // An underwater position cannot get healthier, whatever is taken, so only solvent ones are checked.
            if (!quote.underwater && healthAfter < quote.healthBefore) revert HealthFactorDecreased();
            finished = healthAfter >= HEALTH_FACTOR_ONE;
        }

        if (finished) {
            delete auctionStartBlock[borrower];
            emit AuctionReset(borrower);
        }
    }

    function _writeOff(address borrower) internal {
        (uint256 coveredByReserve, uint256 deficit) = pool.writeOffBadDebt(borrower);
        emit BadDebtSettled(borrower, coveredByReserve, deficit);
    }

    function _useNonce(address liquidator, uint256 nonce) internal {
        uint256 word = nonce >> 8;
        uint256 bit = uint256(1) << (nonce & 0xff);
        uint256 bitmap = nonceBitmap[liquidator][word];

        if (bitmap & bit != 0) revert NonceAlreadyUsed();
        nonceBitmap[liquidator][word] = bitmap | bit;
        emit NonceUsed(liquidator, nonce);
    }

    function _recoverSigner(LiquidationIntent calldata intent, bytes calldata signature)
        internal
        view
        returns (address)
    {
        bytes32 structHash = keccak256(
            abi.encode(
                INTENT_TYPEHASH,
                intent.borrower,
                intent.collateralAsset,
                intent.maxRepay,
                intent.minCollateralOut,
                intent.nonce,
                intent.deadline
            )
        );
        (address signer, ECDSA.RecoverError err,) = ECDSA.tryRecover(_hashTypedDataV4(structHash), signature);
        if (err != ECDSA.RecoverError.NoError || signer == address(0)) revert InvalidSignature();
        return signer;
    }

    // Half the debt at most, unless that would leave a dust balance nobody would bother to clear.
    /// @dev If the collateral cannot cover the whole debt plus the bonus the position is underwater. Then the close
    /// factor is lifted so the liquidator can clear it out, and what is left is written off as bad debt.
    function _quote(LiquidationIntent calldata intent) internal view returns (Quote memory quote) {
        uint256 startBlock = auctionStartBlock[intent.borrower];
        if (startBlock == 0) revert NoActiveAuction();

        uint256 available;
        (quote.healthBefore, quote.debt, available) = pool.getLiquidationData(intent.borrower, intent.collateralAsset);
        if (quote.healthBefore >= HEALTH_FACTOR_ONE) revert PositionHealthy();

        quote.bonusBps = _bonusBps(startBlock);
        (uint256 rateNumerator, uint256 rateDenominator) = _conversionRate(intent.collateralAsset, quote.bonusBps);
        quote.underwater = Math.mulDiv(quote.debt, rateNumerator, rateDenominator) > available;

        if (quote.underwater) {
            quote.repayAmount = intent.maxRepay < quote.debt ? intent.maxRepay : quote.debt;
        } else {
            quote.repayAmount = _repayAmount(quote.debt, intent.maxRepay);
        }
        if (quote.repayAmount == 0) revert NothingToRepay();

        // Take everything that is left rather than reverting, the shortfall is handled as bad debt.
        quote.collateralSeized = Math.mulDiv(quote.repayAmount, rateNumerator, rateDenominator);
        if (quote.collateralSeized > available) quote.collateralSeized = available;
        if (quote.collateralSeized < intent.minCollateralOut) revert InsufficientCollateralOut();
    }

    function _repayAmount(uint256 debt, uint256 maxRepay) internal view returns (uint256) {
        uint256 cap = (debt * CLOSE_FACTOR_BPS) / BPS;
        if (debt - cap < dustThreshold) cap = debt;
        return maxRepay < cap ? maxRepay : cap;
    }

    // Linear in blocks, capped at the maximum. It can only grow while the auction stays open.
    function _bonusBps(uint256 startBlock) internal view returns (uint256) {
        uint256 elapsed = block.number - startBlock;
        if (elapsed >= auctionDuration) return maxBonusBps;
        return (maxBonusBps * elapsed) / auctionDuration;
    }

    // Collateral for a repayment is repayAmount * numerator / denominator, rounded down so the borrower never loses
    // more than the repayment and bonus justify.
    function _conversionRate(address collateralAsset, uint256 bonusBps)
        internal
        view
        returns (uint256 numerator, uint256 denominator)
    {
        uint256 debtPrice = oracle.getPrice(debtAsset);
        uint256 collateralPrice = oracle.getPrice(collateralAsset);
        uint256 debtDecimals = IERC20Metadata(debtAsset).decimals();
        uint256 collateralDecimals = IERC20Metadata(collateralAsset).decimals();

        numerator = 10 ** collateralDecimals * debtPrice * (BPS + bonusBps);
        denominator = collateralPrice * BPS * 10 ** debtDecimals;
    }
}
