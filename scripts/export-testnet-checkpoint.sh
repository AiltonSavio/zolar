#!/usr/bin/env bash
# Export the 113 newest block times and compact difficulty values ending at <tip_height>, the context
# a Header Relay Checkpoint carries (102-block NU7 window + 11-block median).
# Source: Tatum public Zcash testnet JSON-RPC (the chain the header fixtures come from). Walks back from
# the tip with raw headers (`getblockheader <hash> false`), one request per block, and checks each header's
# SHA-256d equals the hash that was asked for. The endpoint allows 5 requests per minute, so this takes
# about 25 minutes.
# Output: crates/zcash-header/fixtures/testnet-checkpoint-<tip_height>.json (oldest first; hashes in
# display order, times in Unix seconds, bits as hex).
# Usage: scripts/export-testnet-checkpoint.sh <tip_height>
set -euo pipefail
tip="$1"
RPC="${ZCASH_TESTNET_RPC:-https://zcash-testnet.gateway.tatum.io}"
OUT="$(dirname "$0")/../crates/zcash-header/fixtures/testnet-checkpoint-$tip.json"
call() { for t in 1 2 3 4 5; do curl -sf -m 30 -X POST -H "content-type: application/json" -d "$1" "$RPC" && return; sleep 15; done; return 1; }
sleep 13
hash=$(call "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getblockhash\",\"params\":[$tip]}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])')
rows=()
for i in $(seq 0 112); do
  sleep 13
  raw=$(call "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getblockheader\",\"params\":[\"$hash\",false]}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])')
  row=$(python3 - "$raw" "$hash" "$((tip - i))" <<'PY'
import sys, json, hashlib
raw, want, height = bytes.fromhex(sys.argv[1]), sys.argv[2], int(sys.argv[3])
got = hashlib.sha256(hashlib.sha256(raw).digest()).digest()[::-1].hex()
assert got == want, (got, want)
print(json.dumps({"height": height, "hash": want, "prev": raw[4:36][::-1].hex(),
                  "time": int.from_bytes(raw[100:104], "little"), "bits": raw[104:108][::-1].hex()}))
PY
)
  rows+=("$row")
  hash=$(python3 -c 'import sys,json;print(json.loads(sys.argv[1])["prev"])' "$row")
done
printf '%s\n' "${rows[@]}" | python3 -c '
import sys, json
rows = [json.loads(l) for l in sys.stdin][::-1]
blocks = [{k: r[k] for k in ("height", "hash", "time", "bits")} for r in rows]
print(json.dumps({"source": "'"$RPC"' getblockheader <hash> false", "tip_height": rows[-1]["height"], "blocks": blocks}, indent=1))
' > "$OUT"
echo "wrote $OUT"
