# Keyless vending prover

`zolar-vend-prove` builds a revision 2 payment witness from a Zcash-testnet
transaction, full P2P blocks and the Solana-devnet Header Relay. It rebuilds
the Final Block Tree from P2P headers, requires equality with the Relay root,
and checks the root is still known after preparation. It holds no spending keys.

## Machine configuration

By default, `RpcShops::shop(&self, machine: &str) -> Result<ShopConfig>` reads
`getAccountInfo` at finalized commitment. The account must be non-executable,
owned by vending program `8XqWKHQiesY5o2eDKyewb9ZHwAZ5gdDMtWGuJogvQ5X5`,
and exactly 291 bytes with discriminator/version `1,1`. The program branch's
layout is decoded without guessing: Relay `100..132`, diversifier `148..159`,
SVK hash `159..191`, full public `dk || ivk` at `227..291`. The decoder checks
that the key is valid and its SHA-256 matches the stored hash. The program ID
is pinned to the Codama definition; a deployment under another ID needs a
coordinated update.

`--shops public-shops.json` explicitly selects the offline metadata fallback:

```json
[{
  "machine": "BASE58_MACHINE",
  "shop_viewing_key": "128_HEX_CHARACTERS_DK_IVK",
  "svk_hash": "64_HEX_CHARACTERS_SHA256_DK_IVK",
  "diversifier": "22_HEX_CHARACTERS",
  "relay": "BASE58_RELAY",
  "scan_start": 4543890
}]
```

This fallback validates the key/hash but does not authenticate on-chain Machine
configuration. It reads public metadata only. It still fetches chain data.

The on-chain source scans the latest 1000 Relay-tip blocks. An offline
`scan_start` must keep its range below 1000 blocks; `0` selects the automatic
window. Payments ahead of the Relay tip appear once the Relay catches up.
The scanner is stateless and scans on each request. Supplying an older txid
directly still permits proof preparation if it follows the Relay Checkpoint.

## Build and execute

Host-only cargo checks use a 3 GiB/no-swap scope and two jobs. SP1 builds,
Clippy and execution run on the VPS with a 5 GiB/no-swap scope. Check `free -g`
and wait for at least 3 GiB available locally (5 GiB on the VPS).

```sh
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- \
  env CARGO_BUILD_JOBS=2 cargo build --manifest-path crates/vending-prover/Cargo.toml
```

Build the guest using `../vending-guest/build-elf.sh` on the VPS, then:

```sh
export PATH="$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH"
export PROTOC="$HOME/spikes/zolar-vending/protoc/bin/protoc"
export VENDING_GUEST_ELF="$PWD/crates/vending-guest/elf/vending-guest"
systemd-run --user --scope -q -p MemoryMax=5G -p MemorySwapMax=0 -- \
  env CARGO_BUILD_JOBS=2 cargo test --manifest-path crates/vending-prover/Cargo.toml \
  --features sp1 --test execute -- --nocapture
```

The tests compare all 249 SP1 output bytes to native verification, reject
nonzero exit codes, and check changed value/rseed/rho, wrong key/diversifier/
machine/nonce/payout, bad text memos, bad transaction and tree paths, and the
unrelated ticket 02 memo. The real fixture has real block and Final Block Tree
paths. SP1 execution uses `ProverClient::builder().light()`, avoiding the CPU
prover's roughly 6.9 GB allocation.

```sh
zolar-vend-prove --txid DISPLAY_HEX --machine BASE58 --rpc https://api.devnet.solana.com --execute
# Optional --nonce BASE58; otherwise discover it from the decrypted text memo.
zolar-vend-prove --witness crates/vending-guest/fixtures/testnet/witness.json --execute --out output
zolar-vend-prove serve --bind 127.0.0.1:8088 --rpc https://api.devnet.solana.com --execute
```

Compile with `--features sp1` for execution. Execution writes canonical
`public_values.bin` and `result.json`; `proof.bin` is written only after a real
proof succeeds. `--prove` requires 32 GB+ total/28 GB available, and a build
with `--features native-prove` for the native gnark backend. **No Groth16
proving, native gnark build or Docker was run on the laptop or VPS.**

## HTTP API

JSON, open CORS. All three request fields are required; machine and nonce
are canonical base58 encodings of exactly 32 bytes. Txids are explorer-order
hex. Jobs are bounded in memory: one preparation/proof and one scan at a time,
128 jobs, one-hour retention. The server owns no wallet keys.

- `GET /find-payment?machine=BASE58&nonce=BASE58`: HTTP 200 with
  `{"status":"pending"}` or
  `{"status":"found","txid":"HEX","height":4543894,"value_zat":100000,"payout":"BASE58"}`.
- `POST /prove` with `{"machine":"BASE58","nonce":"BASE58","txid":"HEX"}`:
  HTTP 202 with `{"job":"ID"}`.
- `GET /prove/ID`: `{"status":"waiting_finality"}`, `{"status":"proving"}`,
  `{"status":"failed","error":"TEXT"}`, or
  `{"status":"done","proof":"HEX","public_values":"HEX"}`.

An unmined or not-yet-final payment waits and retries every five seconds, for
up to an hour. Once witness preparation succeeds, state changes to `proving`.
An execute-only server finishes jobs as `failed` with an explicit no-proof
error; it never advertises an execution result as a claimable proof.
Busy/capacity errors are 429; malformed fields are 400; provider errors are
502 for scans and `failed` for jobs. Missing/expired jobs are 404.

Compact scanning only identifies candidates. Full decryption verifies the
txid, memo, value, diversifier and payout. Inclusion paths use complete P2P
blocks whose recomputed transaction Merkle root equals the block header.
Solana genesis must be devnet; the Relay must be running and Zcash testnet.

## Real fixture

Exactly one new payment was sent on 2026-10-26 using the existing testnet Payer
wallet and patched zcash-devtool to the ticket 02 Payee-view address:

- Txid: `5f4dd9423abe8530be89d6d904bc5b429a9fba739de49808b072fafe17cab514`.
- Payment: 100,000 zat (0.001 TAZ); fee: 10,000 zat (0.0001 TAZ).
- Height: 4,543,894; raw transaction: 9,166 bytes.
- Memo: `zv1:8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR:CktRuQ2mttgRGkXJtyksdKHjUdc2C4TgDzyB98oEzy8:GgBaCs3NCBuZN12kCJgAW63ydqohFkHEdfdEXBPzLHq`.

`examples/testnet_fixture.rs` fetches the transaction via ChainSafe and complete
transaction IDs via P2P. From a public Relay snapshot captured before payment
finality, it extends the Relay's actual frontier with real linked headers to
build the payment path, then requires its root to be known to a fresh live
Relay snapshot. It refuses to write the fixture until that check passes.
This avoids replaying the entire Relay history for a recent fixture.
The fixture stores the raw tx, witness, real header, txid list, public values,
and the live Relay snapshot that accepted the root. All are public chain data.
Archived root membership proves collection-time finality; live roots age out.

A fixture using a configured Machine or an on-chain Claim is separate evidence:
the payment uses fixed fixture machine/nonce/payout bytes `02`/`03`/`04` repeated
32 times. No Machine was created and no vending Claim was sent by this worker.

## Measured revision 2 execution

SP1 v6.10.0 executed the **real mined fixture** on the VPS: **24,108,690
cycles**, **13.47 s**, exit code 0, 249 bytes equal to native verification.
The complete synthetic positive statement took **24,137,454 cycles** (13.11 s
in the final ELF rerun).
These are execution results; no Groth16 proof or vending Claim was produced.

```text
ELF SHA-256: 878577fd6e009a3fb75060ea7977c87782f3bdfe6db2e7b50679d75bf089b77e
SP1 vkey:   0x00ed19d5de2524a0c4e2d1cacfd4f239e39f8cb9bcd6f671db19338354531217
```

The actual HTTP server's `GET /find-payment` returned the real fixture's txid,
height, `value_zat: 100000` and expected payout in **1.58 s**. That check used
the explicit offline metadata fallback and live ChainSafe/Relay data. The
Machine decoder and RPC source are tested against the byte-exact 291-byte
`tests/fixtures/machine-revision2.bin`, serialized by the program worker's
`Machine::write` (producer details in the fixture README). A deployed Machine
read remains unrun.
The test server was stopped afterwards.

## Final verification

Host-only checks run locally with a 3 GiB/no-swap cap; SP1 checks run on the VPS
with a 5 GiB/no-swap cap, two build jobs, and available-memory checks first.
For each crate below the commands are `cargo fmt --manifest-path PATH --check`,
`cargo clippy --manifest-path PATH --locked --all-targets -- -D warnings`, and
`cargo test --manifest-path PATH --locked`, where PATH is the crate's
`Cargo.toml`.

| Crate | fmt | Clippy | Tests |
|---|---|---|---|
| `crates/vending-types` | PASS, 0.19 s | PASS, 0.50 s | PASS, 4 tests, 0.61 s |
| `crates/vending-guest` | PASS, 0.12 s | PASS, 3.28 s | PASS, 10 tests, 8.23 s |
| `crates/vending-prover` | PASS, 0.10 s | PASS, 12.37 s | PASS, 15 tests, 7.37 s; 1 live-provider test ignored |

VPS `crates/vending-guest/build-elf.sh`: PASS, 0.92 s.
VPS `cargo clippy --locked --manifest-path crates/vending-prover/Cargo.toml
--features sp1 --all-targets -- -D warnings`: PASS, 34.65 s.
VPS `cargo run --locked --manifest-path crates/vending-prover/Cargo.toml
--features sp1 -- --witness crates/vending-guest/fixtures/testnet/witness.json
--execute --out target/vending-rev2-execute`: PASS, **127.97 s** including build,
execution and program-key setup. Output: `mode=execute cycles=24108690
public_values=249 proof=false`; the vkey hash is recorded above. The CLI
checks SP1's successful exit and equality with native public values before
writing output.

Native builds from the earlier full verification also passed: types 0.92 s,
guest 61.14 s, prover 18.34 s. No on-chain vending Claim, Groth16 proof, or
live deployed Machine read is established by these results.

The final ELF rerun of `cargo test --locked --manifest-path
crates/vending-prover/Cargo.toml --features sp1 --test execute -- --nocapture
--test-threads=1` passed **2 tests** in **210.89 s** including command overhead:
real and synthetic positive executions, plus **16 negative executions**, each
with exit code 1 and no committed public values. Real fixture execution alone
took 13.47 s; synthetic positive execution took 13.11 s. Both positive outputs
match all 249 native public-value bytes.

The Machine's `vkey_hash` stores the 32 decoded bytes of the SP1 vkey above.
The matching ELF is available on the VPS at
`~/spikes/zolar-vending/worktree-guest/crates/vending-guest/elf/vending-guest`
and locally at `crates/vending-guest/elf/vending-guest` (ignored build artifact).
CLI output is in the ignored local `target/vending-rev2-execute/` directory.
