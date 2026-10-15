// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {IPriceFeed} from "../interfaces/IPriceFeed.sol";

/// @notice Chainlink-style feed whose answer and timestamp tests can set freely, including bad values.
contract MockChainlinkFeed is IPriceFeed {
    uint8 public immutable decimals;

    // One slot, so a price read costs a single storage load like a real aggregator.
    int128 public answer;
    uint96 public updatedAt;
    uint32 public roundId;

    event AnswerSet(int128 answer, uint32 roundId);
    event UpdatedAtSet(uint96 updatedAt);

    constructor(uint8 decimals_, int128 initialAnswer) {
        decimals = decimals_;
        _setAnswer(initialAnswer);
    }

    function setAnswer(int128 newAnswer) external {
        _setAnswer(newAnswer);
    }

    /// @notice Lets tests make the feed look stale.
    function setUpdatedAt(uint96 newUpdatedAt) external {
        updatedAt = newUpdatedAt;
        emit UpdatedAtSet(newUpdatedAt);
    }

    function latestRoundData() external view returns (uint80, int256, uint256, uint256, uint80) {
        return (roundId, answer, updatedAt, updatedAt, roundId);
    }

    function _setAnswer(int128 newAnswer) internal {
        answer = newAnswer;
        updatedAt = uint96(block.timestamp);
        roundId += 1;
        emit AnswerSet(newAnswer, roundId);
    }
}
