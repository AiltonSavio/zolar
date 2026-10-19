#!/usr/bin/env bash
# Re-exports the recorded Zcash mainnet header fixture and cross-checks it against Blockchair.
#
# Source: 400 consecutive mainnet headers, heights 3,522,201 to 3,522,600 (mined 2026-10-18), read from
# public Zcash P2P peers (getheaders over the mainnet seeders, port 8233) by export-testnet-p2p.py --mainnet.
# The locator is block 3,522,200 as the Blockchair explorer prints it; the export checks every header links
# to the previous one by SHA-256d; linkage alone does not prove the chain is the real one, so
# crosscheck-mainnet-headers.py compares hash, time and bits of 41 headers (first, last, every 10th) with
# https://api.blockchair.com/zcash/dashboards/blocks/<heights>.
#
# Usage: scripts/export-mainnet-headers.sh [out-file]   (about a minute; needs python3 and network)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/../crates/zcash-header/fixtures/mainnet/mainnet-3522201-3522600.txt}"
tmp="$out.tmp"
trap 'rm -f "$tmp"' EXIT
python3 -I "$here/export-testnet-p2p.py" --mainnet 3522200 \
  000000000035424cfdfeceb49ae71093095c2e760ef6700d38ed115841120ff5 400 "$tmp"
python3 -I "$here/crosscheck-mainnet-headers.py" "$tmp" 10
mv "$tmp" "$out"
