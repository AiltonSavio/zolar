#!/usr/bin/env bash
# Run inside an extracted public source bundle on an Ubuntu/Debian x86_64 host.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mode=${1:---plan}
case "$mode" in
  --plan|--help)
    cat <<'PLAN'
Public, keyless prover setup: SP1 v6.10.0, Rust 1.97.1, protoc 28.3, Go 1.24.0.
Supported host: Ubuntu/Debian x86_64 with systemd, sudo/root and internet access.
Before installing: >=32,000,000 KiB total RAM; >=28,000,000 KiB available.
--install installs dependencies only; --build builds the native prover only.
--prove proves the existing paid Reservation; --serve starts the keyless HTTP prover.
--run installs, builds and runs --prove. --plan changes nothing and contacts no host.
The guest ELF is pinned; it is checked, never rebuilt. No payment and no Claim is sent.
Proving uses --prove against a fresh Relay root. Recheck that root locally before Claim.
Logs include elapsed time and maximum process RSS; total proving memory is not yet measured.
PLAN
    exit 0 ;;
  --install|--build|--prove|--serve|--run) ;;
  *) echo 'Use --plan, --install, --build, --prove, --serve or --run' >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { echo 'Unexpected arguments' >&2; exit 2; }
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || { echo 'Linux x86_64 required' >&2; exit 1; }
python3 - <<'PY'
from pathlib import Path
values = {line.split(':')[0]: int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith(('MemTotal:', 'MemAvailable:'))}
if values['MemTotal'] < 32_000_000 or values['MemAvailable'] < 28_000_000:
    raise SystemExit('Use the rented host: >=32,000,000 KiB total and >=28,000,000 KiB available RAM required')
PY
cd "$root"
export PATH="$HOME/.local/zolar-go/go/bin:$HOME/.local/zolar-protoc/bin:$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
export PROTOC="$HOME/.local/zolar-protoc/bin/protoc"
export GOTOOLCHAIN=auto CGO_ENABLED=1
export CC_riscv64im_succinct_zkvm_elf="$HOME/.sp1/bin/riscv64-unknown-elf-gcc"
if (( EUID == 0 )); then
  sudo_cmd=()
  scope=(systemd-run --scope -q)
elif systemctl --user show-environment >/dev/null 2>&1; then
  sudo_cmd=(sudo)
  scope=(systemd-run --user --scope -q)
else
  sudo_cmd=(sudo)
  scope=(sudo systemd-run --scope -q)
fi
install_tools() {
  "${sudo_cmd[@]}" apt-get update
  "${sudo_cmd[@]}" apt-get install -y build-essential pkg-config libclang-dev libssl-dev \
    ca-certificates curl git unzip python3 time
  mkdir -p "$HOME/.local/zolar-protoc" "$HOME/.local/zolar-go" "$HOME/.sp1/bin"
  local downloads
  downloads=$(mktemp -d)
  curl --fail --location --retry 3 https://sh.rustup.rs -o "$downloads/rustup.sh"
  if ! command -v rustup >/dev/null; then
    sh "$downloads/rustup.sh" -y --no-modify-path --profile minimal --default-toolchain none
  fi
  rustup toolchain install 1.97.1 --profile minimal
  curl --fail --location --retry 3 \
    https://raw.githubusercontent.com/succinctlabs/sp1/v6.10.0/sp1up/sp1up -o "$downloads/sp1up"
  echo "55d4f6ef8e79e04a7ccd064f75c6a6b44a1694b21cd41aa8e7d0d53eb880a71d  $downloads/sp1up" | sha256sum -c -
  install -m 755 "$downloads/sp1up" "$HOME/.sp1/bin/sp1up"
  sp1up --version v6.10.0 --c-toolchain
  curl --fail --location --retry 3 \
    https://github.com/protocolbuffers/protobuf/releases/download/v28.3/protoc-28.3-linux-x86_64.zip -o "$downloads/protoc.zip"
  echo "0ad949f04a6a174da83cdcbdb36dee0a4925272a5b6d83f79a6bf9852076d53f  $downloads/protoc.zip" | sha256sum -c -
  unzip -o "$downloads/protoc.zip" -d "$HOME/.local/zolar-protoc"
  curl --fail --location --retry 3 https://go.dev/dl/go1.24.0.linux-amd64.tar.gz -o "$downloads/go.tgz"
  echo "dea9ca38a0b852a74e81c26134671af7c0fbe65d81b0dc1c5bfe22cf7d4c8858  $downloads/go.tgz" | sha256sum -c -
  tar -xzf "$downloads/go.tgz" -C "$HOME/.local/zolar-go"
  # Keep downloaded inputs for inspection. They live outside the source bundle.
  echo "Downloaded installers: $downloads"
}
verify_bundle() {
  python3 - <<'PY'
from pathlib import Path
import hashlib, json
manifest = json.loads(Path('prover-bundle.json').read_text())
for name, expected in manifest['sha256'].items():
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or path.is_symlink():
        raise SystemExit('Invalid bundle path')
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise SystemExit(f'Bundle checksum differs: {name}')
demo = json.loads(Path('scripts/vending-paid-demo.json').read_text())
if hashlib.sha256(Path('crates/vending-guest/elf/vending-guest').read_bytes()).hexdigest() != demo['guest_sha256']:
    raise SystemExit('Pinned guest ELF differs')
print('PASS source manifest and pinned guest ELF')
PY
}
verify_inputs() {
  verify_bundle
  [[ $("$PROTOC" --version) == 'libprotoc 28.3' ]] || { echo 'protoc 28.3 required' >&2; exit 1; }
  go version
  cargo +1.97.1 --version
  cargo prove --version
}
build_prover() {
  verify_inputs
  mkdir -p .zolar-work/rented
  # Go compiler limits apply only to compilation, not to the prover at runtime.
  "${scope[@]}" -p MemoryMax=3G -p MemorySwapMax=0 -- \
    env PATH="$PATH" PROTOC="$PROTOC" CARGO_BUILD_JOBS=1 GOMAXPROCS=2 GOMEMLIMIT=1500MiB \
    /usr/bin/time -v cargo +1.97.1 build --release --locked \
    --manifest-path crates/vending-prover/Cargo.toml --features native-prove --bin zolar-vend-prove \
    2>&1 | tee .zolar-work/rented/build.log
}
prove_payment() {
  verify_inputs
  local output=${ZOLAR_PROOF_OUT:-.zolar-work/rented/proof}
  [[ ! -e "$output" && ! -e "$output.log" ]] || { echo 'Output exists; reconcile it before proving again' >&2; exit 1; }
  mkdir -p "$(dirname -- "$output")"
  mapfile -t payment < <(python3 - <<'PY'
import json
d=json.load(open('scripts/vending-paid-demo.json'))
for key in ['txid','machine','nonce','reservation']:
    print(d[key])
PY
  )
  "${scope[@]}" -p MemoryMax=90% -p MemorySwapMax=0 -- \
    env PATH="$PATH" /usr/bin/time -v crates/vending-prover/target/release/zolar-vend-prove \
    --rpc "${ZOLAR_RPC_URL:-https://api.devnet.solana.com}" \
    --elf crates/vending-guest/elf/vending-guest --txid "${payment[0]}" \
    --machine "${payment[1]}" --nonce "${payment[2]}" --prove --out "$output" \
    2>&1 | tee "$output.log"
  python3 scripts/check-vending-execution.py --rpc "${ZOLAR_RPC_URL:-https://api.devnet.solana.com}" \
    --result "$output/result.json" --machine "${payment[1]}" --reservation "${payment[3]}" \
    --require-proof --snapshot-dir "$output/account-check"
  echo "Proof ready at $output. Copy it locally, recheck the live root, then sign Claim locally."
}
case "$mode" in
  --install) verify_bundle; install_tools ;;
  --build) build_prover ;;
  --prove) prove_payment ;;
  --run) verify_bundle; install_tools; build_prover; prove_payment ;;
  --serve)
    verify_inputs
    exec "${scope[@]}" -p MemoryMax=90% -p MemorySwapMax=0 -- \
      env PATH="$PATH" crates/vending-prover/target/release/zolar-vend-prove serve \
      --bind 127.0.0.1:8088 --rpc "${ZOLAR_RPC_URL:-https://api.devnet.solana.com}" \
      --elf crates/vending-guest/elf/vending-guest --prove ;;
esac
