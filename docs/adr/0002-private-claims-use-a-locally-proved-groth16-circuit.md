# Private Claims use a locally proved Groth16 circuit

Private Claims are proved with a hand-written circom Groth16 circuit, generated on the Payer's machine and verified on Solana with `groth16-solana`, not with a zkVM such as SP1. The proof's private inputs are the transaction ID and salt, the exact secret a Private Claim hides, so they must never leave the Payer's machine: a hosted prover network would see them, and SP1's final Groth16 wrap is likely too heavy for a 16 GB laptop without a GPU.

## Considered Options

- **SP1 with the Succinct prover network:** easiest to write (plain Rust), rejected because the network sees the witness.
- **SP1 proved locally:** rejected for hardware cost; revisit if proving hardware changes.
- **A proof system without a per-circuit setup:** rejected for now because no Solana verifier for one is proven in production.

## Consequences

- Groth16 needs a per-circuit trusted setup. Phase 2 is run as a ceremony with several contributors and a published transcript, and is redone before mainnet; whoever knew all the setup randomness could forge Private Claims.
- The circuit only hashes a transaction's Merkle path to its block's root (SHA-256d) plus membership in the Final Block Tree (Poseidon), never whole headers, to keep it small.
