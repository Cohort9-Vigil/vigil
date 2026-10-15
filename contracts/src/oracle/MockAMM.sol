// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {ITwapSource} from "../interfaces/ITwapSource.sol";

/// @notice Constant-product price pool with virtual reserves (no tokens move), exposing a TWAP like a Uniswap v2 pair.
contract MockAMM is ITwapSource {
    uint256 public constant MAX_OBSERVATIONS = 32;
    uint256 private constant WAD = 1e18;

    struct Observation {
        uint256 timestamp;
        uint256 cumulativePrice;
    }

    uint256 public baseReserve;
    uint256 public quoteReserve;
    uint256 public cumulativePrice;
    uint256 public lastUpdate;

    Observation[MAX_OBSERVATIONS] internal observations;
    uint256 public observationCount;
    uint256 public newestObservation;

    event Swap(bool baseIn, uint256 amountIn, uint256 amountOut);
    event ReservesSet(uint256 baseReserve, uint256 quoteReserve);
    event ObservationRecorded(uint256 timestamp, uint256 cumulativePrice);

    error ZeroReserve();
    error InsufficientHistory();
    error InvalidWindow();

    constructor(uint256 baseReserve_, uint256 quoteReserve_) {
        if (baseReserve_ == 0 || quoteReserve_ == 0) revert ZeroReserve();
        baseReserve = baseReserve_;
        quoteReserve = quoteReserve_;
        lastUpdate = block.timestamp;
        _record();
    }

    /// @notice Price of one base unit in quote units, scaled to 1e18.
    function spotPrice() public view returns (uint256) {
        return (quoteReserve * WAD) / baseReserve;
    }

    function swapBaseForQuote(uint256 amountIn) external returns (uint256 amountOut) {
        _accumulate();
        amountOut = (quoteReserve * amountIn) / (baseReserve + amountIn);
        baseReserve += amountIn;
        quoteReserve -= amountOut;
        emit Swap(true, amountIn, amountOut);
    }

    function swapQuoteForBase(uint256 amountIn) external returns (uint256 amountOut) {
        _accumulate();
        amountOut = (baseReserve * amountIn) / (quoteReserve + amountIn);
        quoteReserve += amountIn;
        baseReserve -= amountOut;
        emit Swap(false, amountIn, amountOut);
    }

    /// @notice Test helper to jump the price to a chosen level.
    function setReserves(uint256 newBaseReserve, uint256 newQuoteReserve) external {
        if (newBaseReserve == 0 || newQuoteReserve == 0) revert ZeroReserve();
        _accumulate();
        baseReserve = newBaseReserve;
        quoteReserve = newQuoteReserve;
        emit ReservesSet(newBaseReserve, newQuoteReserve);
    }

    /// @notice Records an observation without trading, so quiet periods still build TWAP history.
    function poke() external {
        _accumulate();
    }

    function consult(uint256 window) external view returns (uint256) {
        if (window == 0) revert InvalidWindow();

        uint256 currentCumulative = cumulativePrice + spotPrice() * (block.timestamp - lastUpdate);
        uint256 target = block.timestamp - window;

        // Walk back from the newest observation to the first one at least `window` seconds old.
        for (uint256 i = 0; i < observationCount; i++) {
            Observation memory observation = observations[(newestObservation + MAX_OBSERVATIONS - i) % MAX_OBSERVATIONS];
            if (observation.timestamp <= target) {
                return (currentCumulative - observation.cumulativePrice) / (block.timestamp - observation.timestamp);
            }
        }
        revert InsufficientHistory();
    }

    // The old price is weighted by the time it was live, so a same-block price push adds nothing.
    function _accumulate() internal {
        cumulativePrice += spotPrice() * (block.timestamp - lastUpdate);
        lastUpdate = block.timestamp;
        _record();
    }

    function _record() internal {
        // forge-lint: disable-next-line(block-timestamp)
        if (observationCount > 0 && observations[newestObservation].timestamp == block.timestamp) {
            observations[newestObservation].cumulativePrice = cumulativePrice;
        } else {
            newestObservation = observationCount == 0 ? 0 : (newestObservation + 1) % MAX_OBSERVATIONS;
            observations[newestObservation] = Observation(block.timestamp, cumulativePrice);
            if (observationCount < MAX_OBSERVATIONS) observationCount += 1;
        }
        emit ObservationRecorded(block.timestamp, cumulativePrice);
    }
}
