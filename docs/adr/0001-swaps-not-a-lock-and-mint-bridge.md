# Swaps, not a lock-and-mint bridge

Zolar moves value between Zcash and Solana with Escrows that pay a Payer once their shielded payment is final, not by locking ZEC and minting a wrapped token. Zcash has no smart contracts, so a Solana program can verify ZEC arriving but can never release it; any wrapped-ZEC bridge needs a signer group to pay out, which is the custody Zolar exists to avoid.

## Consequences

- Every transfer needs a counterparty on the other side. Finding liquidity (market makers, an order book or solvers) is the product problem, not the protocol's.
- The Header Relay and Inclusion Proofs stay generic, so a custodial mint (such as a mining pool proving its blocks) can still be built on top by someone who accepts that custody.
