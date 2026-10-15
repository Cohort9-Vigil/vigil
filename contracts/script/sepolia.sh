#!/usr/bin/env bash
# Deploys and seeds Vigil on a real testnet such as Sepolia. Start with a dry run: DRY_RUN=1 script/sepolia.sh
#
# Needs RPC_URL, a sender address SENDER, and one way to sign: ACCOUNT (a Foundry keystore name, created with
# `cast wallet import`) or DEPLOYER_KEY. Use a throwaway key that holds only testnet ETH.
# The twap needs a real half hour of history before the oracle reports Normal, so this script waits.
set -euo pipefail

: "${RPC_URL:?set RPC_URL}"
: "${SENDER:?set SENDER to the deployer address}"
TWAP_WAIT_SECONDS="${TWAP_WAIT_SECONDS:-1860}"

if [ -n "${ACCOUNT:-}" ]; then
    SIGNER=(--account "$ACCOUNT" --sender "$SENDER")
elif [ -n "${DEPLOYER_KEY:-}" ]; then
    SIGNER=(--private-key "$DEPLOYER_KEY")
else
    echo "set ACCOUNT (keystore name) or DEPLOYER_KEY" >&2
    exit 1
fi

if [ "${DRY_RUN:-0}" = "1" ]; then
    echo "dry run, nothing is sent"
    forge script script/Deploy.s.sol --rpc-url "$RPC_URL" "${SIGNER[@]}"
    exit 0
fi

# Sepolia charges far more gas for creating contracts than the local simulation assumes, so ask the network for
# each estimate instead of using forge's. --slow sends one transaction at a time so every estimate sees the last one.
forge script script/Deploy.s.sol --rpc-url "$RPC_URL" "${SIGNER[@]}" --broadcast --slow --skip-simulation

echo "waiting $TWAP_WAIT_SECONDS seconds for the twap window"
sleep "$TWAP_WAIT_SECONDS"

script/refresh-feeds.sh

forge script script/Seed.s.sol --rpc-url "$RPC_URL" "${SIGNER[@]}" --broadcast --slow --skip-simulation
