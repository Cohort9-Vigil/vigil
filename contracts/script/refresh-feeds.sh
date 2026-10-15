#!/usr/bin/env bash
# Re-posts the mock feed prices so their timestamps are fresh. The mock feeds go stale after a day, and a stale feed
# freezes the oracle. Run it before seeding on a real network and now and then while demoing.
#
# Needs RPC_URL and one way to sign: ACCOUNT (a Foundry keystore name) or DEPLOYER_KEY.
set -euo pipefail

: "${RPC_URL:?set RPC_URL}"
CHAIN_ID="$(cast chain-id --rpc-url "$RPC_URL")"
DEPLOYMENT="deployments/${CHAIN_ID}.json"

if [ -n "${ACCOUNT:-}" ]; then
    SIGNER=(--account "$ACCOUNT")
elif [ -n "${DEPLOYER_KEY:-}" ]; then
    SIGNER=(--private-key "$DEPLOYER_KEY")
else
    echo "set ACCOUNT (keystore name) or DEPLOYER_KEY" >&2
    exit 1
fi

address_of() {
    python3 -I -c "import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])" "$DEPLOYMENT" "$1"
}

# feed name in the deployment file, price in whole dollars
refresh() {
    local feed
    feed="$(address_of "$1")"
    cast send "$feed" "setAnswer(int128)" "$(( $2 * 100000000 ))" --rpc-url "$RPC_URL" "${SIGNER[@]}" > /dev/null
    echo "refreshed $1 at \$$2"
}

refresh stablecoinFeed 1
refresh feeTokenFeed 100
refresh sixDecimalTokenFeed 50
refresh eightDecimalTokenFeed 20000
