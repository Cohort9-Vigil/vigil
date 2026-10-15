// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {ERC20} from "openzeppelin-contracts/contracts/token/ERC20/ERC20.sol";

/// @notice ERC20 that burns a fee from every transfer, so receivers get less than the amount sent.
contract FeeOnTransferToken is ERC20 {
    uint256 public constant BPS = 10_000;

    uint8 private immutable tokenDecimals;
    uint256 public feeBps;

    event FeeChanged(uint256 newFeeBps);

    error FeeTooHigh();

    constructor(string memory name_, string memory symbol_, uint8 decimals_, uint256 feeBps_) ERC20(name_, symbol_) {
        if (feeBps_ > BPS) revert FeeTooHigh();
        tokenDecimals = decimals_;
        feeBps = feeBps_;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function setFeeBps(uint256 newFeeBps) external {
        if (newFeeBps > BPS) revert FeeTooHigh();
        feeBps = newFeeBps;
        emit FeeChanged(newFeeBps);
    }

    function decimals() public view override returns (uint8) {
        return tokenDecimals;
    }

    // Mints and burns are exempt, only real transfers pay the fee.
    function _update(address from, address to, uint256 amount) internal override {
        if (from == address(0) || to == address(0) || feeBps == 0) {
            super._update(from, to, amount);
            return;
        }

        uint256 fee = (amount * feeBps) / BPS;
        super._update(from, address(0), fee);
        super._update(from, to, amount - fee);
    }
}
