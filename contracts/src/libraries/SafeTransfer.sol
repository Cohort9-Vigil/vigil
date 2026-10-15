// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice ERC20 transfers that also work for tokens which return nothing, like USDT.
library SafeTransfer {
    error TransferFailed();

    function safeTransfer(address token, address to, uint256 amount) internal {
        // transfer(address,uint256)
        _call(token, abi.encodeWithSelector(0xa9059cbb, to, amount));
    }

    function safeTransferFrom(address token, address from, address to, uint256 amount) internal {
        // transferFrom(address,address,uint256)
        _call(token, abi.encodeWithSelector(0x23b872dd, from, to, amount));
    }

    function _call(address token, bytes memory data) private {
        bool ok;
        assembly {
            let success := call(gas(), token, 0, add(data, 0x20), mload(data), 0, 0x20)
            switch returndatasize()
            // No return value is fine, but only if there is code at all (a call to an empty address also succeeds).
            case 0 { ok := and(success, gt(extcodesize(token), 0)) }
            default { ok := and(success, and(gt(returndatasize(), 0x1f), eq(mload(0), 1))) }
        }
        if (!ok) revert TransferFailed();
    }
}
