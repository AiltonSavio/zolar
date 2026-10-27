#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
export CC_riscv64im_succinct_zkvm_elf="$HOME/.sp1/bin/riscv64-unknown-elf-gcc"
free -g
while (( $(awk '/MemAvailable:/ {print $2}' /proc/meminfo) < 3 * 1024 * 1024 )); do
  echo 'Waiting for 3 GiB available memory before building the SP1 guest.' >&2
  sleep 15
done
systemd-run --user --scope -q -p MemoryMax=5G -p MemorySwapMax=0 -- env CARGO_BUILD_JOBS=2 \
  cargo prove build --locked --features zkvm --output-directory elf --elf-name vending-guest
