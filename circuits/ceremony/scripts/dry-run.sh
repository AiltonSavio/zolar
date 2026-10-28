#!/usr/bin/env bash
# Full rehearsal of the ceremony on a reduced circuit with throwaway contributors, a real beacon block,
# verification, and a proof from the rehearsal key. It never touches ceremony/transcript.json or
# circuits/verification_key.json.
#   ceremony/scripts/dry-run.sh [depth]        default depth 2 (203,256 constraints, 2^18 ptau)
# Needs the R1CS and wasm from `scripts/build.sh <depth>` and the 2^18 ptau in $ZOLAR_DATA/ptau.
# Writes to $ZOLAR_DATA/ceremony-dry/d<depth>; prints time and peak memory of every step.
set -euo pipefail
source "$(dirname "$0")/../../scripts/env.sh"
depth="${1:-2}"
ptau="${ZOLAR_PTAU:-$ZOLAR_DATA/ptau/ppot_0080_18.ptau}"
work="${ZOLAR_DRY_DIR:-$ZOLAR_DATA/ceremony-dry}/d$depth"
name="private_claim_d$depth"
build="$ZOLAR_DATA/build/d$depth"
tool=(node "--max-old-space-size=${ZOLAR_NODE_MB:-2500}" "$CIRCUITS/ceremony/ceremony.mjs")
common=(--depth "$depth" --transcript "$work/transcript.json" --dir "$work" --vk "$work/verification_key.json" --ptau "$ptau")
# Zcash mainnet block 3522201, recomputed from its header in ceremony/test/fixtures.
beacon_height=3522201
beacon_hash=00000000006143a0c057a33990d9d4eff5820f86d3ead0e9c65b9f42f4a131c2

rm -rf "$work"
mkdir -p "$work/contributors"

echo "== init"
capped "${tool[@]}" init "${common[@]}"

for i in 1 2 3; do
  echo "== contributor $i (own process, own directory, sees only the previous file)"
  prev="$work/claim_$(printf '%04d' $((i - 1))).zkey"
  mine="$work/contributors/c$i"
  mkdir -p "$mine"
  out="$(capped "${tool[@]}" contribute --in "$prev" --name "Throwaway $i" --out "$mine/out.zkey" --no-prompt \
    --sha256 "$(sha256sum "$prev" | cut -d' ' -f1)" 2>&1 | tee /dev/stderr)"
  hash="$(printf '%s\n' "$out" | grep -A1 'contribution hash (publish this)' | tail -1 | tr -d ' ')"
  [ "${#hash}" -eq 128 ] || { echo "no contribution hash found"; exit 1; }
  echo "== coordinator adds contribution $i"
  capped "${tool[@]}" add "${common[@]}" --in "$mine/out.zkey" --hash "$hash"
done

echo "== beacon"
capped "${tool[@]}" announce-beacon "${common[@]}" --height "$beacon_height" --offline
capped "${tool[@]}" close "${common[@]}" --offline
capped "${tool[@]}" beacon "${common[@]}" --hash "$beacon_hash" --iterations 10
capped "${tool[@]}" finalize "${common[@]}"

echo "== anyone verifies"
capped "${tool[@]}" verify "${common[@]}" --r1cs "$build/$name.r1cs"   # online: recomputes the beacon block

echo "== a proof from the rehearsal key"
cp "$CIRCUITS/fixtures/swap2-d2.input.json" "$work/input.json"
capped $SNARKJS wtns calculate "$build/${name}_js/$name.wasm" "$work/input.json" "$work/witness.wtns"
capped $SNARKJS groth16 prove "$work/claim_final.zkey" "$work/witness.wtns" "$work/proof.json" "$work/public.json"
capped $SNARKJS groth16 verify "$work/verification_key.json" "$work/public.json" "$work/proof.json"
echo "dry run complete: $work"
