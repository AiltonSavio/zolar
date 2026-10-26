# Keyless vending prover

`zolar-vend-prove` builds the guest witness from a Zcash-testnet transaction,
full P2P blocks and the Solana-devnet Header Relay. It rebuilds every Final
Block Tree leaf from P2P headers and requires the resulting root to equal the
Relay snapshot, then checks that root is still known after preparation. It
never signs or sends a transaction and holds no spending keys.

The HTTP server has only bounded in-memory jobs: one proof job and one payment
scan at a time, at most 128 jobs, expiring after an hour. There is no durable
payment database. The browser can poll again after a service restart.

## Machine integration boundary

The shared contract says the Vending Machine publishes `dk || ivk`, but its
`InitMachine` arguments contain only `svk_hash`, and it specifies no account
layout. This crate therefore does **not invent an on-chain decoder**.
`backend::ShopSource::shop(&self, machine: &str) -> Result<ShopConfig>` is an
async adapter seam for the vending program's authenticated account decoder.
The supplied `StaticShops` adapter reads an explicit JSON array of public
metadata via `--shops` (never a wallet/key file):

```json
[{
  "machine": "BASE58_MACHINE",
  "shop_viewing_key": "128_HEX_CHARACTERS_DK_IVK",
  "svk_hash": "64_HEX_CHARACTERS_SHA256_DK_IVK",
  "diversifier": "22_HEX_CHARACTERS",
  "relay": "BASE58_RELAY",
  "scan_start": 4520000
}]
```

It validates the incoming viewing key and its hash but is **not evidence of an
on-chain machine configuration**. Replace `StaticShops` with the real account
adapter before claiming integrated vending acceptance. The on-chain Claim must
also check the guest's `svk_hash` against the actual machine.

The stateless scanner scans `scan_start..=relay_tip`, with a 1000-block work
limit. Configure a recent scan start appropriate to the Reservation window;
older payments can still be proved by supplying their txid. This public
metadata adapter and scan-window integration remain an explicit handoff.

## Build and execution

Host-only code builds/tests locally without SP1 or protoc:

```sh
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- \
  env CARGO_BUILD_JOBS=2 cargo build --manifest-path crates/vending-prover/Cargo.toml
```

Build the guest as described in `../vending-guest/README.md`, then on the VPS
from the repository root:

```sh
export PATH="$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
export PROTOC="$HOME/spikes/zolar-vending/protoc/bin/protoc"
export VENDING_GUEST_ELF="$PWD/crates/vending-guest/elf/vending-guest"
free -g
systemd-run --user --scope -q -p MemoryMax=5G -p MemorySwapMax=0 -- \
  env CARGO_BUILD_JOBS=2 cargo test --manifest-path crates/vending-prover/Cargo.toml \
  --features sp1 --test execute -- --nocapture
```

The execute test runs the full statement and rejects changed value/rseed/rho,
wrong diversifier/key/machine/nonce, bad memo version, transaction Merkle path
mismatch, Final Block Tree root mismatch, an out-of-field sibling, and the
recorded text memo. It compares SP1's 249 output bytes to the native result and
prints `FULL_GUEST cycles=...` and execution time. SP1 can return an execution
report for a panicking guest: the runner explicitly requires `exit_code == 0`;
negative checks require a nonzero exit code and no committed public values.
The Solana verifier must likewise require a successful guest exit code.

```sh
# Use the resulting binary with a supplied public metadata adapter.
zolar-vend-prove --txid DISPLAY_HEX --machine BASE58 --rpc https://api.devnet.solana.com \
  --shops public-shops.json --execute
# Optional --nonce HEX; otherwise infer it from a matching decrypted binary memo.
# Offline, synthetic statement only:
zolar-vend-prove --witness crates/vending-guest/fixtures/synthetic/valid.json --execute --out /tmp/vending-output
# Server (execute only, no claimable proof):
zolar-vend-prove serve --bind 127.0.0.1:8088 --shops public-shops.json --execute
```

Compile with `--features sp1` to enable SP1 execution. Every execute uses
`ProverClient::builder().light()`; it never initializes the roughly 6.9 GB CPU
prover. `public_values.bin` and `result.json` are written on execution;
`proof.bin` is written **only** after a real proof succeeds.

`--prove` is wired through CPU setup → Groth16 → local verification → SP1 v6
wire proof bytes, with public values checked against native execution. It
refuses before prover allocation unless the machine has 32 GB+ total and 28 GB
available, and requires rebuilding with `--features native-prove` for the
Docker-free native gnark backend. **No proving or native gnark build has been
run on the 15 GB laptop/VPS.** This path requires validation on the larger
machine. No Ethereum or prover-network credentials are used.

## HTTP API

`POST /prove` accepts:

```json
{"txid":"64_HEX_DISPLAY_ORDER","machine":"BASE58","reservation_nonce":"64_HEX"}
```

`nonce` is also accepted as an alias. HTTP 202 returns
`{"job_id":"...","status":"running"}`. `GET /jobs/{job_id}` returns
`running`, `failed` (with `error`), or `succeeded` with `result`:

```json
{"mode":"execute","proof":null,"public_values":"498_HEX_CHARACTERS","cycles":24000000,"vkey_hash":"0x..."}
```

The cycle number above is illustrative. `mode: "groth16"` includes hex-encoded
SP1 v6 proof bytes in `proof`; execute mode must **never** be submitted as a
Claim. Busy/capacity errors are HTTP 429; malformed input is 400.

`GET /find-payment?machine=BASE58&nonce=64_HEX` scans compact Ironwood outputs
with the public incoming viewing key, then fetches/decrypts the full candidate
tx and checks its txid, memo, machine, nonce and diversifier. HTTP 200 returns
`{"txid":"DISPLAY_HEX","height":4515934}`; 202 returns
`{"status":"not_found"}`. Provider failures are 502, never a fabricated
zero payment. Compact txid lists are not trusted to construct inclusion paths;
the prover obtains full P2P blocks and checks their transaction Merkle root.

Both supported networks are enforced: the Solana RPC genesis must be devnet,
and the Relay network must be Zcash testnet. The backend rejects a Halted Relay.

## Read-only provider check

```sh
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- \
  env CARGO_BUILD_JOBS=2 cargo test --manifest-path crates/vending-prover/Cargo.toml \
  --test live -- --ignored --nocapture
```

This verifies ChainSafe still serves the recorded 9,166-byte ticket 02 payment
at height 4,515,934. It proves neither a binary-memo payment nor an on-chain
vending Claim. The ordinary suite also parses a recorded full block and checks
its txids against the independent existing swap-block fixture.

## Measured execution (2026-10-26)

The full synthetic binary-memo statement executed successfully on the VPS:
**24,082,480 cycles**, 249 public bytes, exit code zero, 13.41 seconds with the
debug host. One positive plus 12 negative guest executions passed in 146.72
seconds. Every negative case exited with code 1 and committed zero bytes.
The CLI's execute plus program-key setup completed in 116.77 seconds and wrote
canonical public values with `proof: null`.

For that exact build:

```text
ELF SHA-256: 704613515cf8a0904cdb6d70f30d204dfe8a6d8152cefbcc89bc10c5700f3551
SP1 vkey:   0x00da7fadad246a1c5d59b035c4eea6744f737a47982921c87ba8cba593415787
```

Recompute the program key after any guest/dependency/toolchain change. This is
an execution measurement on explicitly synthetic block/tree roots, not a
Groth16 proof, a mined binary-memo payment, or an accepted vending Claim.
