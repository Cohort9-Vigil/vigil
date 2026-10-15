// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/// @notice Reentrancy lock kept in EIP-1153 transient storage, so it costs no persistent storage and resets each transaction.
library TransientLock {
    // keccak256("vigil.lock"), inline assembly only accepts a literal slot.
    bytes32 private constant LOCK_SLOT = 0x995b64f1feddb287ed43bb01042376682a9c125d4c6e9f33a981a98f658615ef;

    /// @return acquired False when the lock was already held.
    function lock() internal returns (bool acquired) {
        assembly {
            if iszero(tload(LOCK_SLOT)) {
                tstore(LOCK_SLOT, 1)
                acquired := 1
            }
        }
    }

    function unlock() internal {
        assembly {
            tstore(LOCK_SLOT, 0)
        }
    }

    function isLocked() internal view returns (bool locked) {
        assembly {
            locked := tload(LOCK_SLOT)
        }
    }
}
