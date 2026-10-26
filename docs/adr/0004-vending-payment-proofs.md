# Vending uses SP1 for a public payment statement

A Vending Machine mints a ticket for a payment into a dedicated shop account on Zcash testnet. Its Payment Proof verifies Ironwood output decryption, the fixed binary Reservation memo, transaction inclusion and Final Block Tree membership. The Shop Viewing Key and the witness are public, so a hosted prover does not disclose the secret transaction ID and salt protected by an Escrow Private Claim. ADR 0002 continues to govern Escrow Private Claims.

The Vending program pins immutable price, stock, diversifier, Shop Viewing Key hash, Relay and guest verification-key hash. The mint has zero decimals, a PDA mint authority and no freeze authority. V1 tickets are immediately transferable; there is no non-transferable settlement window. The Payee's chosen verification-key hash must identify the agreed vending guest.

## Consequences

- This is testnet-only fulfillment, not wrapped ZEC or an Escrow refund mechanism. Released stock does not refund a Zcash payment.
- Claim public values reveal a Payment Tag linkable to the Zcash action. This does not provide the transaction unlinkability of a Private Claim.
- A fail-closed verifier stub permits local instruction testing but cannot fulfill any payment. A real verifier and a known-good proof must be integrated and exercised before deployment.
- The current shared contract closes Reservation accounts and permits their public nonces to be recreated. An old unclaimed payment can then target the new payout. Permanent nonce consumption requires a shared contract revision.
- Permissionless release immediately after expiry plus Finality Depth can race proof generation for a payment mined at expiry. A two-step release with a Claim window requires a shared contract revision. These two issues block deployment of the current contract.
