# Zolar

Zolar lets a Solana program act on a shielded Zcash payment without a custodian, and without learning which payment it was.

It has two parts:

- **Header Relay**: a Zcash light client on Solana. Anyone can submit Zcash block headers; the program checks the Equihash proof of work, the difficulty adjustment and the median-time rule on-chain, follows the chain with the most work, and records every block that is final in a Poseidon Merkle tree.
- **Escrow**: a trustless swap built on the relay. A Payee locks SPL tokens against a commitment to a Zcash transaction. The Payer pays shielded ZEC, waits until the block is final and claims the tokens. With a **Private Claim** the Payer proves on their own machine, with a Groth16 proof, that the payment is in a final block, and Solana never sees the Zcash transaction ID.

Built for ZECATHON (Cross-Chain track). Vocabulary: [`GLOSSARY.md`](GLOSSARY.md). Design decisions: [`docs/adr/`](docs/adr).

## Checkout demo and SDK

The demo shop exchanges shielded Zcash testnet ZEC for a ticket token on Solana devnet. A merchant service locks the ticket; the Payer's local wallet helper asks for approval, pays and makes a Private Claim. The helper stands in for wallet support. See [Build on Zolar](sdk/README.md) for the SDK and [the shop README](shop/README.md) to run the demo.

### Vending checkout (in progress)

The planned Vending Machine lets the Payee set up stock once. The Payer
reserves a ticket before paying from a Zcash wallet; an SP1 Payment Proof
then authorizes minting without a per-sale Payee service. The machine stores
the full 64-byte public Shop Viewing Key and its SHA-256 hash on chain.
The ZIP-321 link requests the exact price and the ZIP-302 text memo
`zv1:<machine>:<nonce>:<payout>`, zero-padded to 512 bytes. Each field is a
canonical base58 encoding of 32 bytes. The proof's 249-byte public values
include that payout, and Claim requires it to match the Reservation's
payout, preventing payment redirection after release and nonce reuse.
Ana can reserve the same nonce again to claim her payment if stock remains.
The proof, Solana verifier, and checkout integration are not yet verified
end to end; wallet support for NU7 and exact text-memo preservation still
need validation. A stateless prover HTTP service holds no keys and provides
payment discovery, proof jobs, and job polling for the page.
See [ADR 0005](docs/adr/0005-vending-machines-claim-with-sp1-payment-proofs.md).

This mode makes shop sales public through a dedicated Shop Viewing Key.
Wallet misuse or a relay failure can strand the Payer's ZEC: releasing a
Reservation returns stock and rent, not ZEC. The guest does not check the
Halo2 transaction proof and relies on proof of work over the relay's
Finality Depth. Testnet tickets would mint without an extra settlement
delay; this design is for Zcash testnet and Solana devnet only.

Release takes two steps. Any signer can call RequestRelease after the relay
tip exceeds expiry plus Finality Depth, or when the relay is Halted and
24 hours of Solana time have passed since Reservation creation. It records
the Solana time and rejects a second request. A valid Claim still succeeds
during the next 15 minutes, giving Ana time to finish her Payment Proof;
a Halt still blocks Claims. Any signer can then call ReleaseReservation
to return stock and refund Reservation rent to the payout wallet, once
`release_requested_at + 900 <= now` in Solana seconds. A proof delayed past
this Claim window can still lose to release.
A running but stalled relay can block release. Unpaid Reservations can tie
up all stock and repeat after release. Payers need devnet SOL for Reserve's
fee and refundable rent; v1 has no fee pool. The Claim submitter (Payer or
prover service) pays the Claim fee, permanent Payment Tag rent, and any
missing associated token account's rent.

## Live on Solana devnet

| What | Address |
|---|---|
| Header Relay program | `6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D` |
| Escrow program | `HMew6N4iWjmWMbJBoK6WfQRucBt8ZfWpMadd9mpMtbML` |
| Relay following the Zcash testnet (NU7), Finality Depth 10 (demo only) | `C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS` |
| Relay following the Zcash mainnet, Finality Depth 20 | `2m1YC6pGTBpoKDHL5NQgDQDTewE7bYs4m6TdxkMG4Yhs` |

Both relays are kept at the tip by `zolar-relayer`. The mainnet relay runs on Solana devnet: it shows real mainnet headers verified at full mainnet difficulty, it is not a Solana mainnet deployment.

A settled Private Claim swap: Zcash testnet payment `3dbb300e...2e814`, Escrow `AKbiHejAGJ51KqvkBQxvhhKKaAEdudhznrnWxeFJAhUB`, CreateEscrow `4ufFWSDM...UdRdya`, PrivateClaim `SDGSDwPt...NZYZRi`. `scripts/check-txid-absent.sh` finds no form of the Zcash transaction ID in either Solana transaction; the same script finds it in a Reveal Claim swap, so the check is not vacuous.

## How a swap works

1. **Payer** builds an unsigned Zcash transaction (a PCZT) paying the Payee, with the Payee's challenge text in the memo. Its transaction ID is known before signing.
2. **Payee** receives a redacted copy of the PCZT (`created.payee.pczt`), decrypts their output with their viewing key, checks amount and memo, and creates the Escrow: tokens in a vault, the Payer's payout address, an expiry height and the Payment Commitment `Poseidon(txid, salt)`. The salt goes to the Payer privately. The unredacted PCZT stays with the Payer.
3. **Payer** checks the Escrow against the agreed terms and the relay against the Zcash chain, then signs and broadcasts the payment.
4. Once Finality Depth blocks follow the payment's block, the relay appends it to the Final Block Tree.
5. **Payer** claims:
   - **Private Claim**: proves locally that some transaction in a final block of the Escrow's height window hashes with the salt to the commitment. Solana receives 256 bytes of proof and a 32-byte tree root.
   - **Reveal Claim**: sends the transaction ID, salt and Merkle paths. Cheaper, but it publishes the transaction ID. `zolar-payee create-escrow --private-only` makes an Escrow that refuses it.
6. If no valid Claim arrives, the Payee asks for a refund once the tip passes expiry plus Finality Depth (or 24 hours after creation), then takes the tokens back after a 15-minute window in which a late Claim still wins.

An optional **Bond** makes the Payer lock tokens before a Claim can settle. It returns with the Claim and goes to the Payee on a refund, so a Payer cannot hold the Payee's tokens hostage for free.

## Run a testnet swap

This runs one swap of testnet ZEC (TAZ) for a devnet token on your machine. You play both Payer and Payee. It costs nothing but faucet funds.

### Tools

- Rust 1.91.1 (the repository's `rust-toolchain.toml` selects it), Solana CLI 2.3.11 (includes `spl-token`), Node 22 and pnpm 10.
- About 3 GB of free memory for the builds and 2 GiB for a Private Claim proof.

```sh
git clone https://github.com/AiltonSavio/zolar && cd zolar
cargo build --release --manifest-path cli/Cargo.toml    # zolar-payer and zolar-payee; about 10 minutes the first time

# zcash-devtool, with the patch that sets a PCZT's expiry height (the Escrow refuses expiries over 20 blocks ahead)
git clone https://github.com/zcash/zcash-devtool ../zcash-devtool
git -C ../zcash-devtool checkout 0b3436b
git -C ../zcash-devtool apply "$PWD/tools/zcash-devtool-0b3436b.patch"
cargo build --release --manifest-path ../zcash-devtool/Cargo.toml
export ZOLAR_DEVTOOL="$PWD/../zcash-devtool/target/release/zcash-devtool"
```

### Wallets and funds (once)

```sh
W=$HOME/zolar-wallets; mkdir -p "$W"

# Zcash testnet: a Payer wallet that spends, a Payee wallet whose viewing key checks the payment
$ZOLAR_DEVTOOL wallet -w "$W/payer" init --name payer -i "$W/payer-key.txt" --network test
$ZOLAR_DEVTOOL wallet -w "$W/payee" init --name payee -i "$W/payee-key.txt" --network test
$ZOLAR_DEVTOOL wallet -w "$W/payee" list-addresses | awk '/Default Address:/ {print $3}' > "$W/payee.address"
$ZOLAR_DEVTOOL wallet -w "$W/payee" list-accounts | awk '/UFVK:/ {print $2}' > "$W/payee.ufvk"
$ZOLAR_DEVTOOL wallet -w "$W/payer" list-addresses | awk '/Default Address:/ {print $3}'   # fund this address
```

Ask the [jinolabs testnet faucet](https://zcashfaucet.jinolabs.xyz) for TAZ at the Payer address (0.1 TAZ per address per day; a swap uses 0.001 TAZ plus fees), then wait for about 10 confirmations (a few minutes).

```sh
# Solana devnet: two keypairs, some SOL, and a test token standing in for USDC
solana-keygen new --no-bip39-passphrase -o "$W/payee-sol.json"
solana-keygen new --no-bip39-passphrase -o "$W/payer-sol.json"
solana airdrop 2 "$(solana-keygen pubkey "$W/payee-sol.json")" -u devnet   # or https://faucet.solana.com
solana airdrop 1 "$(solana-keygen pubkey "$W/payer-sol.json")" -u devnet
MINT=$(spl-token create-token -u devnet --decimals 6 --fee-payer "$W/payee-sol.json" --mint-authority "$W/payee-sol.json" | awk '/Address:/ {print $2}')
spl-token create-account "$MINT" -u devnet --owner "$W/payee-sol.json" --fee-payer "$W/payee-sol.json"
spl-token mint "$MINT" 100 -u devnet --mint-authority "$W/payee-sol.json" --fee-payer "$W/payee-sol.json" \
  --recipient-owner "$(solana-keygen pubkey "$W/payee-sol.json")"
```

The token has no freeze authority. The Payer refuses a mint that has one (Circle's devnet USDC does) unless told `--accept-freeze-authority`.

### The swap

```sh
export ZOLAR_ZCASH_WALLET="$W/payer" ZOLAR_ZCASH_IDENTITY="$W/payer-key.txt"
export ZOLAR_PAYER_KEYPAIR="$W/payer-sol.json" ZOLAR_PAYEE_KEYPAIR="$W/payee-sol.json"
export PAYEE_UA_FILE="$W/payee.address" PAYEE_UFVK_FILE="$W/payee.ufvk" MINT
```

**Reveal Claim** (no circuit files needed):

```sh
PRIVATE_ONLY=0 CLAIM=reveal scripts/private-swap.sh     # about 8 minutes; the last step finds the txid on Solana, as expected
```

**Private Claim**: the proof must be made with the proving key the devnet Escrow verifies, so download the circuit files rather than building your own key:

```sh
cargo build --release -p dev-groth16          # the local prover
pnpm --dir circuits install                    # snarkjs, for the witness
mkdir -p "$HOME/zolar-data/build" && cd "$HOME/zolar-data/build"
gh release download private-claim-d8-dev -R AiltonSavio/zolar   # wasm, R1CS, proving key, about 490 MB
sha256sum -c SHA256SUMS && tar -xf d8.tar && cd -
export ZOLAR_DATA="$HOME/zolar-data"
scripts/private-swap.sh                        # about 8 minutes: pay, wait for finality, prove (about 1 minute), claim, check
```

The script runs the steps back to back because the payment expires 18 blocks (about 8 minutes on testnet) after the PCZT is made:

1. `zolar-payer create-pczt`: the unsigned payment and its transaction ID, plus the redacted copy for the Payee (`created.payee.pczt`). An unredacted PCZT carries the Payer's full viewing key, note values and change; never hand it to anyone.
2. `zolar-payee create-escrow`: decrypts the PCZT with the viewing key, checks amount and memo, locks the tokens.
3. `zolar-payer pay`: checks the Escrow and the relay, proves and signs, broadcasts.
4. `zolar-payer private-claim` (or `claim`): waits for finality, finds the block over Zcash P2P, proves locally, claims.
5. `scripts/check-txid-absent.sh`: searches every Solana transaction of the Escrow for the Zcash transaction ID. Exit 0 means no form of it was found.

Run `zolar-payer --help` and `zolar-payee --help` for every option, `zolar-payee refund --escrow <address> --wait` to take an unclaimed Escrow back, and `zolar-payer verify-tree` to check that the relay's Final Block Tree matches the headers of live Zcash peers. The read-only dashboard is in [`dashboard/`](dashboard/README.md).

## What each party learns

| Who | Learns | Does not learn |
|---|---|---|
| Anyone reading Solana | The Escrow: Payee, payout wallet, mint, amount, creation tip, expiry height, commitment; when it is claimed or refunded | With a Private Claim: the Zcash transaction ID, its block, its position, the salt |
| Anyone reading Solana, Reveal Claim | Also the transaction ID and salt, so the Zcash payment is linked to the Escrow | |
| Payee | The transaction ID, and the amount and memo of its own output (it checks them before locking) | The Payer's viewing key, other notes, the values of the notes spent, or the change: the Payer hands it a redacted copy of the unsigned payment (`created.payee.pczt`) |
| Zcash P2P peers asked for blocks | That some node asked for the headers since the Checkpoint and for the blocks of a recent window, each block by hash from a random peer | Any transaction ID |
| The Payer's light wallet server (lightwalletd behind zcash-devtool) | The whole payment, and the Payer's IP address | Anything on Solana |
| The Solana RPC node | Every Solana transaction sent through it, with the sender's IP address | The Zcash transaction behind a Private Claim |
| Block explorers | Nothing: neither CLI asks an explorer anything | |

Plain limits on that privacy:

- The Escrow's height window covers about 20 to 30 blocks, and a Claim lands minutes after its block is final. On a quiet chain that narrows the candidates to the few transactions of a few blocks. Zolar hides which transaction paid, not that a swap happened.
- Someone watching all of the Payer's network traffic sees the last block fetched before the Claim, which is the payment's block, and the broadcast to the light wallet server, which carries the whole transaction.
- The Payee's challenge memo must be new for every trade; it is what stops an old payment from being presented again.

## Threat model

Zolar trusts:

- **The Checkpoint**: the block a relay starts from. Anyone can compare it with a Zcash node; the relay's address is a digest of it.
- **Zcash proof of work over Finality Depth**: a Claim is as safe as a reorganisation deeper than Finality Depth is unlikely. Testnet relays use 10 blocks, labelled demo only; the mainnet relay uses 20 (about 25 minutes).
- **The Groth16 setup** for Private Claims. Today it is a single-party development key (see Limitations).
- Solana itself, and the Escrow and relay programs, which are not audited.

What the programs enforce, with tests for each in `crates/relay-tests` and `crates/escrow-tests`:

- A Relayer cannot add a header without valid Equihash (200,9), a hash under its target, the target the difficulty adjustment requires, a timestamp above the median of the last 11 blocks, and a known parent. Forged headers (bad solution, easier target, wrong version) are rejected, and recorded mainnet headers are replayed at full mainnet difficulty.
- The relay follows the chain with the most work. A reorganisation deeper than Finality Depth Halts it: it accepts no more headers and Claims stop, and Escrows are refunded through the 24-hour backstop. A Halt cannot be undone; a new relay is started from a new Checkpoint.
- A Claim pays only the payout wallet named in the Escrow, so a copied or front-run Claim pays the Payer anyway. A second Claim fails. A Claim needs a block from the Escrow's own height window, a root among the relay's last 128, and a running relay.
- A refund cannot beat an honest Claim: it needs the tip above expiry plus Finality Depth (or the 24-hour backstop), then a 15-minute window in which a Claim still wins.
- The Payer's CLI refuses an Escrow whose mint, amount, Bond, payout, expiry, relay or commitment differ from what was agreed, a vault with a delegate or close authority, a mint with a freeze authority unless accepted, a relay of another Zcash network or whose Final Block Tree differs from live peers' headers, and a private-only Escrow unless accepted (a payment in a block of more than 256 transactions cannot be claimed privately).

## Compute units

Measured in LiteSVM and on devnet (`cargo test -p relay-tests -- --nocapture`, `cargo test -p escrow-tests -- --nocapture`).

| Operation | Compute units |
|---|---|
| Relaying one header | 4 transactions; the two heaviest about 1.08M and 1.2M of the 1.4M cap |
| Equihash (200,9) verification | 2.17M in total, split into two steps of 256 leaves (a stock implementation measured 4.46M) |
| `AcceptHeader` (target, difficulty, median time, parent, heaviest chain) | 47k to 65k; up to 120k on devnet when it also finalises a block into the tree |
| `CreateEscrow` | 5.5k |
| `RevealClaim` | 25k |
| `PrivateClaim` (Groth16 verification over BN254) | 95k |
| `DepositBond`, `RequestRefund`, `Refund` | 1.8k, 0.6k, 3.2k |

Keeping a relay at the tip costs 20,000 lamports per header in fees: about 0.07 SOL a day for the testnet relay and 0.02 SOL a day for mainnet. The header buffer's rent comes back after each header.

## Limitations

- **The Private Claim key is a single-party development key.** Whoever generated it could forge a Private Claim. That is acceptable for a devnet demo with test tokens, not for real value. The phase-2 ceremony tooling is ready and rehearsed ([`circuits/ceremony/`](circuits/ceremony/README.md)); the ceremony itself runs after the hackathon with outside contributors, and one honest contributor is enough.
- Devnet only. Nothing is deployed to Solana mainnet, and the programs are not audited.
- The testnet Finality Depth of 10 is for the demo only.
- The mainnet relay's Checkpoint predates NU7 on mainnet; after NU7 activates, a new relay with the activation height is needed.
- A Private Claim needs the payment's block to hold at most 256 transactions (the circuit's depth 8), about 2 GiB of memory and about a minute of proving.
- No Relayer rewards: anyone can relay, and the demo relays are run by the author.
- A flood of about 250 valid sibling blocks could make the relay's store refuse an honest fork block. Each costs real proof of work.

## Prior art and credit

- **Skyhook** (Strahn Cryptography) introduced swaps keyed to a ZIP 244 transaction ID, on Ethereum and Base, with Equihash verified on the EVM and FlyClient-style finality. Per its proposal, it enforces a trusted difficulty floor rather than the difficulty adjustment, and the recipient's note data is visible on-chain. Zolar's swap design credits it. Zolar differs in enforcing the full difficulty adjustment on-chain, in relaying contiguously instead of sampling, and in the Private Claim, which reveals neither the recipient nor the transaction.
- **ZULU** (Starknet, Zypherpunk 2025) relayed Zcash headers with on-chain Equihash and proved transaction inclusion. Zolar brings the idea to Solana, where a header does not fit one transaction or one compute budget.

## Repository layout

- `crates/zcash-header`: `no_std` header parsing, SHA-256d, compact targets, chain work, Equihash and the difficulty rules, shared by the program and off-chain tools.
- `programs/header-relay`: the Header Relay program (Pinocchio). It has its own workspace and lockfile because `cargo build-sbf` uses an older toolchain.
- `crates/header-relay-interface`: dependency-free layout and `RelayView` (owner, address and Halt checked) for programs that trust a relay.
- `programs/escrow`: the Escrow program (Pinocchio), own workspace.
- `clients/codama`: Codama definitions; `clients/rust`, `clients/js`, `clients/escrow-rust` and `clients/escrow-js` are generated from them.
- `crates/relayer`: `zolar-relayer`, which follows Zcash peers over P2P and keeps a relay at the tip.
- `crates/zolar-claim`: Payment Commitment, Merkle paths and Private Claim witness. `crates/zolar-pczt`: transaction ID and viewing-key decryption of an unsigned PCZT (own workspace).
- `cli/`: `zolar-payer`, `zolar-payee`, `zolar-merchant` and `zolar-wallet` (own workspace).
- `sdk/`: TypeScript merchant and wallet clients and the checkout flow. See [Build on Zolar](sdk/README.md).
- `shop/`: demo ticket checkout using the SDK. See [the shop README](shop/README.md).
- `circuits/`: the Private Claim circuit (circom, Groth16), proving scripts and the ceremony tooling. See [`circuits/README.md`](circuits/README.md).
- `dashboard/`: read-only web page for both relays, open Escrows and Claims.
- `crates/relay-tests`, `crates/escrow-tests`: LiteSVM tests that drive the programs through the generated clients.
- `scripts/`, `tools/`: fixture export, forged-header fixtures, the swap script and the transaction ID search.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build-sbf --manifest-path programs/header-relay/Cargo.toml
cargo build-sbf --manifest-path programs/escrow/Cargo.toml
cargo test --workspace                       # LiteSVM tests build the programs first; about 5 minutes
cargo test --manifest-path cli/Cargo.toml
pnpm --dir clients/codama install && pnpm --dir clients/codama generate   # regenerate the clients
python3 -I scripts/test_check_txid_absent.py
pnpm --dir sdk typecheck && pnpm --dir sdk test
pnpm --dir shop lint && pnpm --dir shop test
npx --prefix shop --no-install tsc --noEmit -p shop
pnpm --dir shop build
pnpm --dir sdk build
```

## License

Licensed under either of MIT ([`LICENSE-MIT`](LICENSE-MIT)) or Apache-2.0 ([`LICENSE-APACHE`](LICENSE-APACHE)).
