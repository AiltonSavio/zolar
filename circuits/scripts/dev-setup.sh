#!/usr/bin/env bash
# Throwaway single-party Groth16 setup for a built circuit, in a minute and about 1.3 GB at depth 8 (arkworks).
# Development only: no Powers of Tau, no contributions, so whoever ran it could forge proofs. The ceremony
# (ticket 12) uses scripts/setup.sh instead (snarkjs), whose `powersoftau prepare phase2` alone ran for
# hours without finishing on the development machine.
#   scripts/dev-setup.sh <depth>       writes dev_pk.bin and verification_key.json next to the R1CS
set -euo pipefail
source "$(dirname "$0")/env.sh"
depth="${1:?usage: dev-setup.sh <depth>}"
dir="$ZOLAR_DATA/build/d$depth"
repo="$CIRCUITS/.."
(cd "$repo" && capped cargo build --release -p dev-groth16)
capped "$repo/target/release/dev-groth16" setup "$dir/private_claim_d$depth.r1cs" "$dir/dev_pk.bin" "$dir/verification_key.json"
