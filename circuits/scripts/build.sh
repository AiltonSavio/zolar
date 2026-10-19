#!/usr/bin/env bash
# Compile the Private Claim circuit for a transaction-tree depth: R1CS, wasm witness generator, symbols.
#   scripts/build.sh <depth>        depth 8 is the production circuit; 1 and 2 are reduced test instances
set -euo pipefail
source "$(dirname "$0")/env.sh"
depth="${1:?usage: build.sh <depth>}"
src="$CIRCUITS/src/private_claim_d$depth.circom"
if [ ! -f "$src" ]; then
  printf 'pragma circom 2.1.0;\n\ninclude "private_claim.circom";\n\ncomponent main {public [root, commitment, lower, expiry]} = PrivateClaim(%s, 20);\n' "$depth" > "$src"
fi
out="$ZOLAR_DATA/build/d$depth"
mkdir -p "$out"
capped "$CIRCOM" "$src" --r1cs --wasm --sym --O1 -l "$CIRCUITS/node_modules" -o "$out"
sha256sum "$out/private_claim_d$depth.r1cs"
