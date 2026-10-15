// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {ERC20} from "openzeppelin-contracts/contracts/token/ERC20/ERC20.sol";

/// @notice ERC20 that calls back into a target in the middle of every transfer, to test reentrancy guards.
contract ReentrantToken is ERC20 {
    uint8 private immutable tokenDecimals;

    address public callbackTarget;
    bytes public callbackData;
    bool public revertOnCallbackFailure;
    bool public lastCallbackSucceeded;
    bytes public lastCallbackReturnData;

    bool private inCallback;

    event CallbackConfigured(address indexed target, bytes data);
    event CallbackExecuted(address indexed target, bool success);

    constructor(string memory name_, string memory symbol_, uint8 decimals_) ERC20(name_, symbol_) {
        tokenDecimals = decimals_;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    /// @notice Pass a zero target to switch the callback off.
    function setCallback(address target, bytes calldata data, bool revertOnFailure) external {
        callbackTarget = target;
        callbackData = data;
        revertOnCallbackFailure = revertOnFailure;
        emit CallbackConfigured(target, data);
    }

    function decimals() public view override returns (uint8) {
        return tokenDecimals;
    }

    // The callback runs after balances move but before the caller finishes its own bookkeeping.
    function _update(address from, address to, uint256 amount) internal override {
        super._update(from, to, amount);

        if (callbackTarget == address(0) || inCallback || from == address(0) || to == address(0)) return;

        inCallback = true;
        (bool success, bytes memory returnData) = callbackTarget.call(callbackData);
        inCallback = false;

        lastCallbackSucceeded = success;
        lastCallbackReturnData = returnData;
        emit CallbackExecuted(callbackTarget, success);

        if (!success && revertOnCallbackFailure) {
            assembly {
                revert(add(returnData, 0x20), mload(returnData))
            }
        }
    }
}
