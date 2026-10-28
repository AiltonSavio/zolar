# Vending Machine

`programs/vending` sells SPL tickets for shielded ZEC without a per-sale service on the Payee's side. It is a separate Pinocchio workspace; the Codama source is `clients/codama/vending.mjs` and the generated clients are `clients/vending-rust` and `clients/vending-js/src/generated`. Design: [ADR 0005](../../docs/adr/0005-vending-machines-claim-with-sp1-payment-proofs.md). Devnet program: `J9paKuXLpRr4rmWVp8vBZ6ZMwQhrTSk6w3Rmyv5JHFW5`; [Make your own demo shop](#make-your-own-demo-shop) deploys it under your own address.

> [!WARNING]
> A vending Claim mint with a real Payment Proof has **not been verified**. Machine setup, Reserve, one real payment, payment discovery and SP1 execution were checked live on 2026-10-27; a full Groth16 Payment Proof, a successful Claim mint and a full browser purchase through an external Zcash wallet were not. Zcash testnet and Solana devnet only.

Ben configures 100 tickets at **0.001 TAZ** each, once. Ana connects a Solana devnet wallet, reserves one ticket and opens the payment request in her Zcash testnet wallet. She pays 100,000 zat plus her wallet fee from shielded funds, keeping the requested memo. After Finality Depth, a keyless prover makes the Payment Proof and Ana signs the Claim that mints one ticket. Ben takes no per-sale action; the Header Relay and the prover still need to run.

Ana's Solana wallet signs Reserve and Claim and receives the ticket; it sends no ZEC. Her Zcash wallet sends the payment to Ben's dedicated shop address. Ana pays Reserve's fee and its refundable rent. Whoever submits the Claim (Ana or a prover service) pays its fee, the permanent Payment Tag rent and any new token account's rent. V1 has no fee sponsorship.

Commands below run from the repository root. The browser page is described in the [shop README](../../shop/README.md#vending-checkout) and the SDK binding in the [SDK README](../../sdk/README.md#vending-checkout-testnet--devnet).

## Trust and limits

- Shop sales are public: the Machine stores the full 64-byte Shop Viewing Key on chain. Vending does not have the Escrow Private Claim's transaction ID privacy.
- Releasing a Reservation returns stock and rent, never ZEC. Wallet misuse or a relay failure can strand the Payer's ZEC.
- The guest does not check the Halo2 transaction proof; it relies on proof of work over the relay's Finality Depth.
- Tickets are transferable at once, with no settlement delay.
- A running but stalled relay can block release. Unpaid Reservations can tie up all stock, and repeat after release.

## How it works

Ana reserves one of Ben's tickets, paying the Reservation's Solana rent. The Reservation
snapshots the price and expiry, and records Ana as payout. A verified Payment Proof
creates a permanent Payment Tag, mints one ticket into Ana's associated token account
(creating it if missing), and returns the Reservation rent to Ana. The Claim submitter
pays for the Tag and any new token account. RequestRelease starts a 15-minute Claim
window; ReleaseReservation then returns stock and Reservation rent;
it does not refund Zcash. Mint decimals are zero, mint authority is the PDA, and freeze
authority is absent. Testnet v1 tickets are immediately transferable: there is **no
non-transferable settlement window**.

The program uses the real `sp1-solana-v6` verifier and the same `vending-types`
public-values type as the guest. The verifier exposes
`verify_proof(proof: &[u8], public_values: &[u8], vkey_hash: &[u8; 32]) -> Result<(), Error>`.
`PublicValues::encode()` and `PublicValues::decode(&[u8])` share the fixed 249-byte
wire format. Decoding checks version 1, Ironwood pool 3, and the zero reserved
suffix; the typed amount is `value_zat`. Payout occupies `189..221`, with only
`221..249` reserved. See the [types](../../crates/vending-types/README.md),
[guest](../../crates/vending-guest/README.md), [prover](../../crates/vending-prover/README.md)
and [verifier](../../crates/sp1-solana-v6/README.md) for APIs and fixture provenance.

Account/authority bindings are checked before proof verification. Every subsequent
Claim policy rule is in the native-testable pure function
`rules::validate_claim(&PublicValues, ClaimAccounts<'_>) -> Result<ClaimPlan, VendingError>`.
A successful plan fixes one ticket and closure to the stored payout. There is no test
proof bypass in the program. Successful Tag creation, ATA creation, MintTo and Claim closure still require a
valid vending proof to verify in LiteSVM and remain unverified. Release closure and rent return are verified in LiteSVM.

Revision 2 binds the memo and proof to the payout: the text memo is exactly
`zv1:<machine>:<nonce>:<payout>` with canonical base58 encodings of three 32-byte
values, zero-padded to 512 bytes. Claim requires the proven payout to equal the
Reservation's payout. After release, the same Payer can reserve the nonce again and
claim an old unclaimed payment while stock remains; another payout cannot use it.
The full 64-byte Shop Viewing Key is stored publicly, and InitMachine computes its
SHA-256 hash with the Solana syscall.

RequestRelease is permissionless when `relay_tip > expiry_height + finality_depth`
**or** when the Relay is Halted and 24 hours have passed since Reservation creation.
It records Solana time once; duplicate requests fail without resetting the window.
ReleaseReservation is permissionless at or after `release_requested_at + 900`.
A Claim remains eligible during the window and afterwards until release closes the
Reservation. A later Halt does not reset or extend an existing release window.
A running Relay stalled below the height threshold does not qualify for the Halt
backstop and can leave stock reserved indefinitely. Releasing stock never refunds ZEC.
Claims use `RelayView` for owner/address/Halt and recent-root checks. The program accepts
only the testnet network and NU7 branch ID `0x77190ad9`.

## Live demo evidence (2026-10-27)

The deployed Machine is `GiF2qqYCRHL6L4DVVDMPfNDk9G5f4qzovMCM62UHFjkV`, with
mint `DARKR7E6KGL8L6oWeNXwnbhdSBL8oyZku8PVkxejVFjg`, price 100,000 zat,
stock 100 and a 20-block Reservation duration. It uses the running testnet Relay
`C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS` (demo Finality Depth 10).
The public browser configuration is in [`.env.vending.example`](../../shop/.env.vending.example)
and [`vending-abi.json`](../../shop/public/vending-abi.json).

One 0.001 TAZ payment mined at height 4,546,839:
`c9cea1a5a3da02bc89a6afc58e7132eb0ce0610938cb647a81b354c9c8ac575e`.
Its Reservation expires at height 4,546,854. The actual HTTP service found the
correct value and payout by reading the full Shop Viewing Key from the Machine.
The live payment executed in SP1 at **24,103,795 cycles**, committing 249 bytes
identical to native verification; witness preparation, execution and guest-key
setup took **199.52 s** on the VPS. Independent account checks matched machine,
nonce, payout, price, diversifier, key hash, NU7 branch, guest vkey and a root
still known to the live Relay. This produced **no Groth16 proof and no ticket**.

Live InitMachine used 13,075 CU (13,225 including ComputeBudget); Reserve used
8,830 CU (8,980 including ComputeBudget). The deploy transaction is
`2jNpaUbid2BDpFE4ydPpPtBq9ZJG8AZLe1KY9wgxR2kcrdWEUEheYdNtwv6RontgWMmBgCQnRnBwGNVeo4Ndv6bw`.
The deployed 96,360-byte ELF matches SHA-256
`2a7feca3504d3d0a05bab4c850b58ce2a58434c69ba0c99b2b002e4175eacf65`.
Claim minting with a real vending Payment Proof remains unverified.

## Prepare a proving host

The existing paid demo Reservation must not be paid again or released.
Its public identifiers and guest hash are in
[`scripts/vending-paid-demo.json`](../../scripts/vending-paid-demo.json).

On the source machine, package tracked working files and the pinned public
ELF. This includes uncommitted source edits, records every file hash, and
excludes ignored files and wallet data. It does not require a GitHub push:

```sh
python3 -I scripts/prepare-vending-prover.py --out /tmp/zolar-prover.tgz
bash scripts/setup-vending-prover.sh --plan
scp /tmp/zolar-prover.tgz rented:zolar-prover.tgz
ssh rented 'mkdir zolar-vending-paid && tar -xzf zolar-prover.tgz -C zolar-vending-paid'
ssh -t rented 'cd zolar-vending-paid && bash scripts/setup-vending-prover.sh --run'
```

Choose a fresh remote directory and preserve the printed archive SHA-256.
The host must be Ubuntu/Debian x86_64, have systemd and sudo/root access,
and provide at least 32,000,000 KiB total RAM with 28,000,000 KiB available
(the prover checks KiB, so a provider's nominal 32 GB may be insufficient).
SP1 v6.10.0, Rust 1.97.1, protoc 28.3, and Go 1.24.0 are installed there.
The guest is checked against the deployed Machine's pinned artifact and is
never rebuilt by this script. Source and installer references:
[SP1 installer](https://github.com/succinctlabs/sp1/blob/v6.10.0/sp1up/sp1up),
[native gnark build](https://github.com/succinctlabs/sp1/blob/v6.10.0/crates/recursion/gnark-ffi/build.rs),
and [Go releases](https://go.dev/dl/).

`--run` installs, builds under a 3 GiB/no-swap cap, and proves only the
existing payment. `--install`, `--build`, and `--prove` run each stage
separately. `--serve` exposes the keyless prover on remote loopback port
8088 for the browser demo; forward it with
`ssh -N -L 8088:127.0.0.1:8088 rented`. No spending key, seed, identity,
or Solana signer goes to the rented machine.

Proving logs and `/usr/bin/time -v` measurements go under
`.zolar-work/rented/`. Maximum process RSS is not aggregate prover memory.
The proof uses a fresh Relay root; the script checks root membership again
when it finishes. If the root has aged out, make a new proof against a fresh
root. Do not weaken the root check. The setup, native gnark build, full proof
resource usage, and Claim remain unverified on a rented host.

Copy the `proof/` directory back locally. Before signing Claim, collect a
read-only baseline (replace the identifiers using the public demo JSON):

```sh
node sdk/scripts/check-vending-settlement.mjs before \
  --result /path/to/proof/result.json \
  --machine MACHINE_ADDRESS --reservation RESERVATION_ADDRESS \
  --out /tmp/vending-before.json
```

This requires the real Groth16 output and checks its Machine bindings and
live Relay root. It refuses a release request that needs reconciliation.
The default Claim submitter is the payout wallet; supply
`--submitter PUBLIC_ADDRESS` if another wallet pays the fee. The checker
never signs or sends transactions. To submit this exact one-shot proof, sign locally:

```sh
node sdk/scripts/submit-vending-claim.mjs \
  --before /tmp/vending-before.json --result /path/to/proof/result.json \
  --keypair /path/to/claim-submitter-devnet.json --out /tmp/vending-claim.json
```

This command rechecks the live root, simulates, signs, saves the expected
signature before broadcast, and sends once. After any timeout, reconcile
that journal and signature before retrying. Keep the signer local. Wait
for finalization, then verify the signature:

```sh
node sdk/scripts/check-vending-settlement.mjs after \
  --before /tmp/vending-before.json --result /path/to/proof/result.json \
  --signature CLAIM_SIGNATURE --out /tmp/vending-settlement.json
```

The checker requires the exact Claim instruction and proof, a one-ticket
transaction balance increase to the fixed payout, a closed Reservation,
a permanent Payment Tag, the expected mint authority, and a replay
simulation rejected specifically with `MissingReservation`. It records
Claim CU and fees. It sends no replay transaction. Keep the baseline,
proof, signature, and output together; a green execute run alone cannot
pass settlement verification.

## Make your own demo shop

Ben creates a dedicated testnet account, publishes its incoming Shop Viewing Key
in a Machine, and offers 100 tickets at 0.001 TAZ each. Ana reserves one ticket
with her Solana devnet wallet, pays 100,000 zat plus her Zcash wallet fee, and
waits for the Relay's Finality Depth. A suitable proving host must produce the
Payment Proof before Claim can mint her ticket. Execute mode produces no proof.

Use Zcash testnet and Solana devnet only. Keep all seed, identity and keypair
files outside git. The tools accept paths and never print their private contents.
Shop metadata contains only the dedicated public incoming viewing key; never
substitute a Payer key or a spending/full viewing key.

From the repository root, prepare Node 22, pnpm 10, Solana CLI and Rust, then:

```sh
free -g
cargo_capped() {
  systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- \
    env CARGO_BUILD_JOBS=2 cargo "$@"
}
export VENDING_DATA="$HOME/zolar-vending"
mkdir -p "$VENDING_DATA" "$HOME/.config/zolar"
umask 077
# Choose an unused path; solana-keygen refuses an existing file.
export PROGRAM_KEY="$HOME/.config/zolar/vending-keypair.json"
solana-keygen new --no-bip39-passphrase --silent --outfile "$PROGRAM_KEY"
export PROGRAM_ID=$(solana-keygen pubkey "$PROGRAM_KEY")
python3 scripts/set-vending-program-id.py "$PROGRAM_ID"
pnpm --dir clients/codama install --frozen-lockfile
pnpm --dir clients/codama generate
cargo_capped fmt --all
cargo_capped fmt --manifest-path programs/vending/Cargo.toml
cargo_capped build-sbf --manifest-path programs/vending/Cargo.toml
python3 scripts/check-vending-sbf.py
cargo_capped test -p vending-tests -- --test-threads=2
# Your funded devnet deployer, supplied as a path only.
export DEPLOYER="$HOME/.config/solana/devnet.json"
solana-keygen new --no-bip39-passphrase --silent --outfile "$VENDING_DATA/deploy-buffer.json"
solana program deploy programs/vending/target/deploy/vending.so --url devnet \
  --keypair "$DEPLOYER" --upgrade-authority "$DEPLOYER" \
  --program-id "$PROGRAM_KEY" --buffer "$VENDING_DATA/deploy-buffer.json" --use-rpc
pnpm --dir clients/vending-js install --frozen-lockfile
pnpm --dir clients/vending-js build
pnpm --dir sdk install --frozen-lockfile
# When changing program IDs in an existing checkout, refresh its local client copy:
# pnpm --dir sdk install --frozen-lockfile --force
pnpm --dir sdk build
pnpm --dir shop install --frozen-lockfile
cargo_capped build --manifest-path crates/vending-prover/Cargo.toml --bin zolar-vend-shop
crates/vending-prover/target/debug/zolar-vend-shop \
  --create --seed "$VENDING_DATA/shop.seed" --out "$VENDING_DATA/shop-public.json"
```

The seed is raw 32-byte ZIP-32 material written with mode 600. The helper derives
`m/32'/1'/0'`, external address index 0, validates the receiving address against
the 64-byte key, and writes an Ironwood-only testnet Unified Address. Running
again without `--create` and with a new `--out` reproduces the public metadata.
The seed stays local; neither the prover nor the browser needs it.

The immutable Machine must pin the SP1 vkey from the exact ELF you will use.
The integration artifact is documented in the [guest README](../../crates/vending-guest/README.md).
Build/execution instructions are in the [prover README](../../crates/vending-prover/README.md).
On a VPS, use SP1 v6.10.0 and a 5 GiB/no-swap scope to execute the recorded witness;
check both the ELF SHA-256 and the emitted `vkey_hash` before initializing:

```sh
sha256sum crates/vending-guest/elf/vending-guest
systemd-run --user --scope -q -p MemoryMax=5G -p MemorySwapMax=0 -- \
  crates/vending-prover/target/debug/zolar-vend-prove \
  --witness crates/vending-guest/fixtures/testnet/witness.json --execute --out vkey-check
# For the pinned integration ELF only; recompute if rebuilding the guest.
export GUEST_VKEY=000eeda101ba026e2f37c86e96abc5372ddc0330ea99d56e7ddfbef8aae711e7
export RELAY=C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS
solana-keygen new --no-bip39-passphrase --silent --outfile "$VENDING_DATA/mint.json"
scripts/vending-admin.sh init-machine --keypair "$DEPLOYER" \
  --mint-keypair "$VENDING_DATA/mint.json" --shop "$VENDING_DATA/shop-public.json" \
  --relay "$RELAY" --vkey-hash "$GUEST_VKEY" --price-zat 100000 --stock 100 \
  --reservation-blocks 20 --out "$VENDING_DATA/machine.json"
```

`init-machine` checks devnet genesis, the running testnet Relay and the public
shop metadata, simulates, submits, and waits for finalization. It writes the
Machine, mint, signature and CU, plus `machine.binding.json` for the browser.
The 20-block duration is this demo tool's default, not a protocol default.
Configuration is immutable; changing the guest or shop account requires a new
Machine. The CLI records the signed transaction identity before submission;
a timeout requires checking that signature and account before retrying.

Inspect the resulting Machine and mint before paying. On the Payer's machine,
with their own devnet keypair and devnet SOL:

```sh
scripts/vending-admin.sh inspect --machine MACHINE_ADDRESS
scripts/vending-admin.sh reserve --machine MACHINE_ADDRESS \
  --keypair /path/to/payer-devnet.json --shop "$VENDING_DATA/shop-public.json" \
  --out "$VENDING_DATA/reservation.json"
```

The Reservation output includes the exact ZIP-321 payment URI with amount,
shop address and payout-bound text memo. Pay it once from a Zcash-testnet wallet
supporting Ironwood. Wallet fees are additional. Check the saved Reservation
and signature before retrying an interrupted command; never blindly repeat a
payment. Faucets are not part of this workflow.

Run the keyless prover on a host with SP1 installed. Compile with `--features sp1`
for execute mode, following the capped build command in its README:

```sh
systemd-run --user --scope -q -p MemoryMax=5G -p MemorySwapMax=0 -- \
  crates/vending-prover/target/debug/zolar-vend-prove serve \
  --rpc https://api.devnet.solana.com --bind 127.0.0.1:8088 --execute
curl 'http://127.0.0.1:8088/find-payment?machine=MACHINE_ADDRESS&nonce=BASE58_NONCE'
```

Omit `--shops`: the prover reads the full Shop Viewing Key from the deployed
Machine. `--execute` deliberately fails proof jobs with a no-proof error; it
cannot return a claimable Payment Proof. On a rented 32 GB+ machine with at
least 28 GB available, compile with `--features native-prove`, then use `--prove`
instead. Complete Groth16 proving and a real vending Claim remain unverified.

Copy the public `machine.binding.json` to `shop/public/vending-abi.json`, and
set the public browser configuration (the dedicated incoming key is read from
chain, not placed in an environment variable):

```sh
export VITE_CHECKOUT_MODE=vending VITE_CLUSTER=devnet
export VITE_VENDING_PROGRAM="$PROGRAM_ID" VITE_VENDING_MACHINE=MACHINE_ADDRESS
export VITE_VENDING_ADDRESS=IRONWOOD_TESTNET_SHOP_ADDRESS
export VITE_VENDING_RPC=https://api.devnet.solana.com
export VITE_PROVER_URL=http://127.0.0.1:8088
export VITE_VENDING_ABI_URL=/vending-abi.json
devserve --port 5175 -- pnpm --dir shop dev --host 0.0.0.0 --port 5175 --strictPort
```

Open `http://localhost:5175` and connect a Solana Wallet Standard wallet on
devnet. The live page reads the Machine and Relay, reserves before displaying
the payment link, finds the payment and waits for finality. It can mint only
when the prover returns a real proof accepted by Claim. Stop the named preview
with `devservers stop NAME` afterwards. Public RPCs may throttle large program
uploads; use a devnet RPC with sufficient capacity and reconcile the existing
upload buffer before resuming. If the CLI's parallel writes keep hitting the
public RPC limit, stop that uploader, then resume the existing authorized buffer
sequentially (the helper checks devnet, authority, size and complete ELF readback):

```sh
node sdk/scripts/resume-vending-upload.mjs --rpc https://api.devnet.solana.com \
  --keypair "$DEPLOYER" --buffer BUFFER_ADDRESS \
  --elf programs/vending/target/deploy/vending.so --ledger "$VENDING_DATA/upload.jsonl"
solana program deploy --url devnet --keypair "$DEPLOYER" \
  --upgrade-authority "$DEPLOYER" --program-id "$PROGRAM_KEY" --buffer BUFFER_ADDRESS
```

The portable BLAKE3 fallback avoids the
`sol_blake3` import rejected by devnet during this deployment.

## Vending wire interface

All integers are little-endian. Instructions start with one byte: InitMachine `0`,
Reserve `1`, Claim `2`, ReleaseReservation `3`, RequestRelease `4`.
`seed` and `nonce` are exactly 32 bytes.
Init arguments follow the shared contract, in this order:
`seed32, price_zat u64, stock u32, diversifier11, svk64, relay32, vkey_hash32,
reservation_blocks u32` (187 argument bytes; 188 including discriminator).
The generated argument for `relay` is named
`relayAddress` / `relay_address`, to distinguish it from the account. Reserve carries
only `nonce32`; RequestRelease and ReleaseReservation carry no arguments. Claim carries
`proof_len u16, proof[proof_len], public_values[249]`, with no extra prefix on public
values and no trailing data.

| Account | Bytes | Fields as `offset:name` |
|---|---:|---|
| Machine | 291 | `0:disc=1, 1:version, 2:bump, 3:mint_bump, 4:payee32, 36:seed32, 68:mint32, 100:relay32, 132:price_zat u64, 140:available u32, 144:stock u32, 148:diversifier11, 159:svk_hash32, 191:vkey_hash32, 223:reservation_blocks u32, 227:svk64` |
| Reservation | 128 | `0:disc=2, 1:version, 2:bump, 3:machine32, 35:nonce32, 67:payout32, 99:price_zat u64, 107:expiry_height u32, 111:created_at i64, 119:release_requested bool(u8), 120:release_requested_at i64` |
| Payment Tag | 2 | `0:disc=3, 1:version` |

All versions are `1`; legacy 227/119-byte accounts and 155-byte Init arguments are
rejected. No Vending deployment needs migration. `release_requested` distinguishes
a request at Unix time zero from no request. PDAs are `["machine", payee, seed]`,
`["mint", machine]`,
`["reservation", machine, nonce]`, `["tag", machine, payment_tag]`.

Account order (w = writable, s = signer):

- InitMachine: `payee(ws), machine(w), mint(ws), mintAuthority, relay, tokenProgram, systemProgram`.
- Reserve: `payer(ws), machine(w), reservation(w), relay, systemProgram`.
- Claim: `submitter(ws), machine(w), reservation(w), relay, payout(w), mint(w), mintAuthority, payoutToken(w), paymentTag(w), tokenProgram, associatedTokenProgram, systemProgram`.
- RequestRelease: `submitter(s), machine, reservation(w), relay`.
- ReleaseReservation: `submitter(s), machine(w), reservation(w), relay, payout(w)`.

The numbered errors are defined in `programs/vending/src/error.rs` and mirrored in
Codama. New errors: WrongPayout `21`, ReleaseAlreadyRequested `22`,
ReleaseNotRequested `23`, ClaimWindowOpen `24`. The real SP1 v6 proof is 356 bytes;
with 249 public bytes it fits the packet limit. The conservative 388-byte proof
payload test produces a **1226-byte legacy transaction** including compute-limit and compute-price instructions, with submitter as
fee payer. A separate fee payer exceeds the 1232-byte packet limit with both budget
instructions; use the submitter as fee payer or an address lookup table.

## Verify Vending locally

Check `free -g` first and wait for at least 3 GiB available. Run cargo commands under
`systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- env CARGO_BUILD_JOBS=2`:

```sh
cargo build-sbf --manifest-path programs/vending/Cargo.toml -- --locked
cargo fmt --all --check
cargo fmt --manifest-path programs/vending/Cargo.toml --check
cargo fmt --manifest-path crates/vending-types/Cargo.toml --check
cargo clippy --locked -p vending-tests -p vending-client --all-targets -- -D warnings
cargo clippy --locked --manifest-path programs/vending/Cargo.toml --all-targets -- -D warnings
cargo clippy --locked --manifest-path crates/vending-types/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path crates/vending-types/Cargo.toml
cargo test --locked --manifest-path programs/vending/Cargo.toml
cargo test --locked -p vending-tests -- --nocapture --test-threads=1
```

The LiteSVM harness reuses `relay-tests` to initialize and advance a real Relay with
known Final Block Tree roots. Synthetic headers isolate consumer behavior; they do not
test Equihash. Halt tests inject the frozen Relay state to exercise the consumer's
24-hour path. The suite also checks the syscall hash, payout binding, strict height
threshold, request idempotence, the exact 900-second boundary, a later Halt, and
Unix time zero. Native policy tests accept a Claim during and after the window.
LiteSVM rejects invalid Payment Proofs through the real verifier. Successful
Claim settlement and a real SP1 vending proof remain unverified.
The October 27 live run verified deployment, Reserve, one mined payment,
discovery, and SP1 execution; browser-to-Zkool payment behavior remains unverified.
The verifier has a positive SP1 fixture test, but that fixture is contributor-authored
and is not a vending Payment Proof. See its README for this limitation.
