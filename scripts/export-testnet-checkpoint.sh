#!/usr/bin/env bash
# Export the 113 newest block times and compact difficulty values ending at <tip_height>, the context
# a Header Relay Checkpoint carries (102-block NU7 window + 11-block median).
# Source: zecblock public Zcash testnet API (https://api.testnet.zecblock.com/api/block/<height>).
# Output: crates/zcash-header/fixtures/testnet-checkpoint-<tip_height>.json (oldest first).
# Usage: scripts/export-testnet-checkpoint.sh <tip_height>
set -euo pipefail
tip="$1"
API="${ZCASH_TESTNET_API:-https://api.testnet.zecblock.com/api/block}"
OUT="$(dirname "$0")/../crates/zcash-header/fixtures/testnet-checkpoint-$tip.json"
tmp="$(mktemp)"
for h in $(seq $((tip - 112)) "$tip"); do
  for try in 1 2 3 4 5; do
    curl -sf -m 30 "$API/$h" >> "$tmp" && echo >> "$tmp" && break
    sleep 5
  done
done
python3 - "$tmp" "$tip" "$API" > "$OUT" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
tip = int(sys.argv[2])
assert len(rows) == 113, len(rows)
blocks = []
for i, r in enumerate(rows):
    assert int(r["height"]) == tip - 112 + i
    if i:
        assert r["previous_block_hash"] == rows[i - 1]["hash"], r["height"]
    blocks.append({"height": int(r["height"]), "hash": r["hash"], "time": int(r["timestamp"]), "bits": r["bits"]})
print(json.dumps({"source": sys.argv[3] + "/{height}", "tip_height": tip, "blocks": blocks}, indent=1))
PY
rm -f "$tmp"
echo "wrote $OUT"
