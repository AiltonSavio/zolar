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
