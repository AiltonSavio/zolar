#!/usr/bin/env bash
# Prove one Private Claim locally. The input stays on this machine; nothing here uses the network.
#   scripts/prove.sh <depth> <input.json> <out-dir>
# Writes <out-dir>/proof.json and public.json, and checks the proof with snarkjs against the verification key.
# Key: the ceremony zkey (claim_final.zkey, proved with rapidsnark when found, else snarkjs) if present,
# otherwise the development key from dev-setup.sh (arkworks).
set -euo pipefail
source "$(dirname "$0")/env.sh"
depth="${1:?usage: prove.sh <depth> <input.json> <out-dir>}"
input="$(realpath "${2:?}")"
out="${3:?}"
mkdir -p "$out"
dir="$ZOLAR_DATA/build/d$depth"
zkey="${ZKEY:-$dir/claim_final.zkey}"
capped $SNARKJS wtns calculate "$dir/private_claim_d${depth}_js/private_claim_d$depth.wasm" "$input" "$out/witness.wtns"
if [ -f "$zkey" ] && [ -n "$RAPIDSNARK" ] && [ -x "$RAPIDSNARK" ]; then
  capped "$RAPIDSNARK" "$zkey" "$out/witness.wtns" "$out/proof.json" "$out/public.json"
elif [ -f "$zkey" ]; then
  capped $SNARKJS groth16 prove "$zkey" "$out/witness.wtns" "$out/proof.json" "$out/public.json"
else
  capped "$CIRCUITS/../target/release/dev-groth16" prove "$dir/private_claim_d$depth.r1cs" "$dir/dev_pk.bin" "$out/witness.wtns" "$out/proof.json" "$out/public.json"
fi
rm -f "$out/witness.wtns"
capped $SNARKJS groth16 verify "${VKEY:-$dir/verification_key.json}" "$out/public.json" "$out/proof.json"
