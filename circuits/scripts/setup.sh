#!/usr/bin/env bash
# Single-party throwaway Groth16 setup for a built circuit (ticket 10). The real multi-party ceremony
# (ticket 12) replaces steps 3 and 4 with its contributions and beacon; steps 1, 2 and 5 are the same.
#   scripts/setup.sh <depth> <ptau>      ptau: a Perpetual Powers of Tau file with 2^power >= constraints + 2
set -euo pipefail
source "$(dirname "$0")/env.sh"
depth="${1:?usage: setup.sh <depth> <ptau>}"
ptau="${2:?usage: setup.sh <depth> <ptau>}"
dir="$ZOLAR_DATA/build/d$depth"
r1cs="$dir/private_claim_d$depth.r1cs"
prepared="${ptau%.ptau}_prepared.ptau"
# 1. Prepare the Powers of Tau for phase 2 (once per ptau).
[ -f "$prepared" ] || capped $SNARKJS powersoftau prepare phase2 "$ptau" "$prepared"
# 2. Proving key with no contribution.
capped $SNARKJS groth16 setup "$r1cs" "$prepared" "$dir/claim_0000.zkey"
# 3. One throwaway contribution (entropy from the OS). Ceremony: one per contributor, in turn.
capped $SNARKJS zkey contribute "$dir/claim_0000.zkey" "$dir/claim_final.zkey" --name="throwaway ticket 10" -e="$(head -c 64 /dev/urandom | base64)"
# 4. (Ceremony only) random beacon: snarkjs zkey beacon <in> <out> <hex> 10
# 5. Check the key against the circuit and the Powers of Tau, then export the verifying key.
capped $SNARKJS zkey verify "$r1cs" "$prepared" "$dir/claim_final.zkey"
capped $SNARKJS zkey export verificationkey "$dir/claim_final.zkey" "$dir/verification_key.json"
sha256sum "$dir/claim_final.zkey" "$dir/verification_key.json"
