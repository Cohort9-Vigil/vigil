#!/usr/bin/env bash
# Deploys and seeds the system on a local anvil node. Start anvil first: `anvil`
# The key below is anvil's well-known first test account, it holds no real funds.
set -euo pipefail

RPC_URL="${RPC_URL:-http://127.0.0.1:8545}"
DEPLOYER_KEY="${DEPLOYER_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"

forge script script/Deploy.s.sol --rpc-url "$RPC_URL" --private-key "$DEPLOYER_KEY" --broadcast

# the TWAP needs a full window of history before the oracle reports Normal
cast rpc --rpc-url "$RPC_URL" evm_increaseTime 3600 > /dev/null
cast rpc --rpc-url "$RPC_URL" evm_mine > /dev/null

forge script script/Seed.s.sol --rpc-url "$RPC_URL" --private-key "$DEPLOYER_KEY" --broadcast
