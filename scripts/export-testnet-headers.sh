#!/usr/bin/env bash
# Export raw Zcash testnet block headers (1487 bytes: 140 header + 3 compactSize + 1344 solution).
# Source: Tatum public Zcash testnet JSON-RPC (zcashd-compatible getblockhash/getblockheader).
# Usage: scripts/export-testnet-headers.sh <height>... ; writes crates/zcash-header/fixtures/testnet-<height>.hex
set -euo pipefail
RPC="${ZCASH_TESTNET_RPC:-https://zcash-testnet.gateway.tatum.io}"
OUT="$(dirname "$0")/../crates/zcash-header/fixtures"
mkdir -p "$OUT"
call() { for t in 1 2 3 4 5; do curl -sf -m 30 -X POST -H "content-type: application/json" -d "$1" "$RPC" && return; sleep 15; done; return 1; }
for h in "$@"; do
  sleep 13  # public endpoint allows 5 requests per minute
  hash=$(call "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getblockhash\",\"params\":[$h]}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])')
  sleep 13
  hex=$(call "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getblockheader\",\"params\":[\"$hash\",false]}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])')
  echo "$hex" > "$OUT/testnet-$h.hex"
  echo "height $h hash $hash bytes $((${#hex}/2)) source $RPC"
done
