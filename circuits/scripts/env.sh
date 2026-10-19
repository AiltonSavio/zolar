# Shared settings for the circuit scripts. Source it; do not run it.
#   ZOLAR_DATA      where large artifacts live (ptau, r1cs, wasm, zkey), never in git
#   ZOLAR_MEM_MAX   hard memory cap per step (systemd scope), default 2000M; size it to `free -m`
#   ZOLAR_NODE_MB   node heap cap for snarkjs, default 1500
CIRCUITS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZOLAR_DATA="${ZOLAR_DATA:-$CIRCUITS/../../../.zolar-data/t10}"
mkdir -p "$ZOLAR_DATA"
ZOLAR_DATA="$(cd "$ZOLAR_DATA" && pwd)"
CIRCOM="${CIRCOM:-$ZOLAR_DATA/bin/circom}"
RAPIDSNARK="${RAPIDSNARK:-$(compgen -G "$ZOLAR_DATA/rs/rapidsnark-*/bin/prover" | head -1 || true)}"
SNARKJS="node --max-old-space-size=${ZOLAR_NODE_MB:-1500} $CIRCUITS/node_modules/snarkjs/build/cli.cjs"

# capped <command...>: run under a hard memory cap and print wall time and peak RSS.
capped() {
  if [ -n "${ZOLAR_NO_CAP:-}" ]; then
    python3 "$CIRCUITS/scripts/measure.py" "$@"
    return
  fi
  systemd-run --user --scope -q -p "MemoryMax=${ZOLAR_MEM_MAX:-2000M}" -p MemorySwapMax=0 -- \
    python3 "$CIRCUITS/scripts/measure.py" "$@"
}
