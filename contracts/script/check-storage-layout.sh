#!/usr/bin/env bash
# Fails if the pool's storage layout no longer matches the committed baseline (SC-02).
# After an intended change, refresh it with: script/check-storage-layout.sh --update
set -euo pipefail

BASELINE="storage-layout/baseline.json"
CURRENT="$(forge inspect VigilPool storageLayout --json)"

if [ "${1:-}" = "--update" ]; then
    echo "$CURRENT" > "$BASELINE"
    echo "baseline updated"
    exit 0
fi

if ! diff <(echo "$CURRENT") "$BASELINE"; then
    echo "storage layout changed, see the diff above" >&2
    exit 1
fi
