#!/usr/bin/env bash
# Usage: scripts/check-txid-absent.sh [--url RPC] [--address ADDR ...] <zcash-txid> [solana-signature ...]
# Exit 0: the Zcash transaction ID appears in none of the Solana transactions. See check_txid_absent.py.
exec python3 -I "$(dirname "$0")/check_txid_absent.py" "$@"
