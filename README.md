# Zolar

A Solana-side light client for Zcash and an escrow-based swap built on it. See `GLOSSARY.md` and `docs/adr/`.

## Header Relay

`programs/header-relay` is a Pinocchio program that accepts Zcash block headers from anyone. A Relayer uploads a 1,487-byte header to its own buffer account, the program checks the Equihash (200,9) solution across several transactions, then checks the hash against the header's target and that the parent is known, stores the header and lets the Relayer close the buffer to recover its rent. A Header Relay is created once from a Checkpoint whose digest is its address.

The relay follows the chain with the most cumulative work (the first one seen wins a tie). A block is final once Finality Depth blocks follow it; final blocks are appended, in height order, to the Final Block Tree, a depth-20 Poseidon Merkle tree held inside the Relay account together with the roots after each of the last 128 appends. If the heaviest chain reorganises past a block already in the tree, the relay Halts and accepts no more headers. Other Solana programs read the tip, chain work, Finality Depth, Halt state and recent roots with `crates/header-relay-interface`.

- `crates/zcash-header`: `no_std` header parsing, SHA-256d, compact targets, chain work, Equihash, shared by the program and off-chain tools.
- `programs/header-relay`: the program. It has its own workspace and lockfile because `cargo build-sbf` uses an older toolchain.
- `clients/codama`: the program's Codama definition; `clients/rust` and `clients/js` are generated from it.
- `crates/header-relay-interface`: dependency-free `no_std` layout and `RelayView`, the read interface for programs that trust a Header Relay.
- `scripts/final-block-tree-ref`: independent circomlibjs computation of the Final Block Tree roots the tests compare against.
- `crates/relay-tests`: LiteSVM tests that drive the program through the generated Rust client.
- `tools/forge-header`: builds the one forged fixture (valid Equihash, hash above target).
- `crates/zolar-pczt`: standalone crate (own workspace, pinned to the librustzcash revision zcash-devtool uses). Computes the transaction ID of an unsigned Ironwood PCZT and decrypts a payee's output from its viewing key; `cd crates/zolar-pczt && cargo test`.
- `dashboard/`: read-only web page (Vite, React) for both Header Relays, open Escrows and Claims as they land; see `dashboard/README.md`.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build-sbf --manifest-path programs/header-relay/Cargo.toml
cargo test --workspace                                   # LiteSVM tests build the program first
cargo test -p relay-tests -- --nocapture                 # prints compute units per transaction
pnpm --dir clients/codama install && pnpm --dir clients/codama generate   # regenerate the clients
```

Fixtures come from `scripts/export-testnet-headers.sh <height>...` and `scripts/export-testnet-checkpoint.sh <height>` (public testnet RPC) and `tools/forge-header`.

Licensed under either of MIT (`LICENSE-MIT`) or Apache-2.0 (`LICENSE-APACHE`).

## Private swap

A Payer proves on their own machine that a Zcash payment is in the Final Block Tree and sends the Escrow only a 256-byte Groth16 proof and a root; the transaction ID, salt, block height and Merkle paths never reach Solana. `cli/` holds `zolar-payee` (`create-escrow --private-only` makes an Escrow that refuses the Reveal Claim) and `zolar-payer` (`private-claim`). `scripts/private-swap.sh` runs a whole swap; `scripts/check-txid-absent.sh <txid> [--address ADDR ...] <signatures...>` searches the swap's Solana transactions (raw bytes, `json`, `jsonParsed`, every string decoded from base58 and base64, both byte orders, 8-byte pieces) for the Zcash transaction ID and exits 1 if it finds it. It supports "no recognised encoding of the ID in these transactions", not "nowhere on Solana": it reads transactions, not account contents, and only what the RPC node still keeps. `python3 -I scripts/test_check_txid_absent.py` tests the search.

The proof uses local tooling only (snarkjs witness generation in node, then the arkworks prover): no prover service exists and the witness file lives in a mode 0700 directory that is deleted afterwards. The large circuit artifacts (wasm, R1CS, proving key) are not in git; see `circuits/README.md` for how to get them.

Privacy limits, stated plainly:

- The proving key is a single-party development key (ticket 12 replaces it with a ceremony). Whoever generated it could forge proofs.
- The Escrow is public: payee, payout wallet, mint, amount, creation tip and expiry height. Together they bound the payment's block to the roughly 20 to 30 blocks of the window, and a claim lands minutes after that block is final, so timing narrows the candidates further. Only the exact transaction is hidden, not the fact that a swap happened.
- Payer and Payee still talk to services that see their IP address: the Solana RPC node (everything sent to Solana), the Zcash light wallet server behind zcash-devtool (it receives the whole serialized payment, so it knows the transaction ID and, once mined, its height; the Payer's wallet therefore links itself to the payment on the Zcash side, though not to the Solana claim), and, for the Payee only, the block explorer, which `create-escrow` asks whether the transaction ID is new (`zolar-payee` sends the explorer the txid). The Payer asks no explorer anything.
- To find its block the Payer asks Zcash P2P peers for the blocks of the Escrow's height window, by hash and in order, each from a freshly chosen random peer, and checks each against the header's Merkle root. No peer is told a transaction ID. A single peer sees at most a few recent blocks requested; someone watching all of the Payer's network traffic sees the last block requested before the claim, which is the payment's block. That observer would also see the broadcast to the light wallet server, which already carries the whole transaction.
- A block with more than 256 transactions cannot be proved at depth 8. A private-only Escrow for such a payment can only be refunded.
- The Reveal Claim publishes the transaction ID and salt. Only create Escrows with `--private-only` when the Payer should never be tempted or tricked into it.
