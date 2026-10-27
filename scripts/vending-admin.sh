#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
pnpm --dir "$root/clients/vending-js" exec esbuild \
  "$root/sdk/scripts/vending-admin.mjs" --bundle --platform=node --format=esm \
  --packages=external --outfile="$root/sdk/dist/vending-admin.mjs"
exec node "$root/sdk/dist/vending-admin.mjs" "$@"
