#!/usr/bin/env bash
# Installs a rehearsal verification key in the Escrow (temporarily) and checks, in LiteSVM, that
#   1. the committed development-key proof (circuits/fixtures/swap2.proof.json) is rejected (InvalidProof, 22), and
#   2. the proof made with the rehearsal key is accepted by all 12 Private Claim tests.
# The two files it touches are restored byte for byte on exit.
#   ceremony/scripts/escrow-check.sh <verification_key.json> <proof.json>
# Run it under a memory cap, e.g. systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 -- ...
set -euo pipefail
vk="$(realpath "${1:?usage: escrow-check.sh <verification_key.json> <proof.json>}")"
proof="$(realpath "${2:?}")"
repo="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$repo"
# Restore the exact bytes present on entry (not the committed ones: a ceremony may have changed them).
backup="$(mktemp -d)"
files=(programs/escrow/src/verifying_key_generated.rs circuits/fixtures/swap2.proof.json)
for f in "${files[@]}"; do cp "$f" "$backup/$(basename "$f")"; done
restore() { for f in "${files[@]}"; do cp "$backup/$(basename "$f")" "$f"; done; rm -rf "$backup"; }
trap restore EXIT
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

cargo run -q -p zolar-claim --example gen_vk -- "$vk" programs/escrow/src

echo "== the development key's proof must be rejected by the rehearsal key"
if out="$(cargo test -p escrow-tests --test private_claim a_real_proof_pays_the_payer_and_closes_the_escrow 2>&1)"; then
  echo "$out"
  echo "FAIL: the development proof was accepted"
  exit 1
fi
grep -q 'Custom(22)' <<<"$out" || { echo "$out"; echo "FAIL: it failed, but not with InvalidProof (22)"; exit 1; }
echo "rejected with InvalidProof (22), as required"

echo "== the rehearsal proof must be accepted"
cp "$proof" circuits/fixtures/swap2.proof.json
out="$(cargo test -p escrow-tests --test private_claim -- --test-threads=2 --nocapture 2>&1)" || { echo "$out"; echo "FAIL: the rehearsal proof was rejected"; exit 1; }
grep -E '^test |test result|CU PrivateClaim' <<<"$out"
grep -q 'test result: ok. 12 passed' <<<"$out" || { echo "FAIL: expected 12 passing tests"; exit 1; }
echo "accepted: 12 Private Claim tests pass with the rehearsal key"
