# Zolar

A Solana-side light client for Zcash and an escrow-based swap built on it. See `GLOSSARY.md` and `docs/adr/`.

## Equihash on Solana (ticket 01 spike)

`crates/zcash-header` verifies a Zcash Equihash (200,9) solution in resumable steps; `programs/header-relay` is the Pinocchio program that holds a header in a buffer account and runs the steps, one transaction each (two steps of 256 leaves, about 1.08M compute units each).

```sh
scripts/export-testnet-headers.sh <height>...   # refresh fixtures from a public testnet RPC
cargo test --workspace                          # builds the program with cargo build-sbf, runs LiteSVM tests
cargo test -p header-relay -- --nocapture       # prints compute units per transaction
```
