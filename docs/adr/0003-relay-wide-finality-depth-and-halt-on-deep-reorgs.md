# One relay-wide Finality Depth, and Halt on deep reorgs

The Header Relay has a single Finality Depth, fixed at the Checkpoint, and only blocks that deep enter the Final Block Tree; there is no per-Escrow confirmation depth. A Private Claim hides which block it proves, so nobody could tell if it pointed at a block later orphaned; only blocks already treated as final can be safe to prove against. If the heaviest chain ever reorganises deeper than Finality Depth, the relay Halts and accepts no more headers instead of rewriting final blocks.

## Consequences

- Payees cannot ask for extra confirmations on large trades; the Finality Depth must be chosen for the largest value the deployment should carry (30 by default on mainnet, a smaller labelled value on testnet).
- A Halt is visible and stops new Claims; Escrows still refund through the Solana-time backstop.
