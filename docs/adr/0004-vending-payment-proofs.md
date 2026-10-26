# Vending uses SP1 for a public payment statement

A Vending Machine mints a ticket for a payment into a dedicated shop account on Zcash testnet. Its Payment Proof verifies Ironwood output decryption, the fixed `zv1:<machine>:<nonce>:<payout>` text memo, transaction inclusion and Final Block Tree membership. The Shop Viewing Key and the witness are public, so a hosted prover does not disclose the secret transaction ID and salt protected by an Escrow Private Claim. ADR 0002 continues to govern Escrow Private Claims.

The Vending program pins immutable price, stock, diversifier, full 64-byte Shop Viewing Key and its syscall-computed SHA-256 hash, Relay and guest verification-key hash. The mint has zero decimals, a PDA mint authority and no freeze authority. V1 tickets are immediately transferable; there is no non-transferable settlement window. The Payee's chosen verification-key hash must identify the agreed vending guest.

## Consequences

- This is testnet-only fulfillment, not wrapped ZEC or an Escrow refund mechanism. Released stock does not refund a Zcash payment.
- Claim public values reveal a Payment Tag linkable to the Zcash action. This does not provide the transaction unlinkability of a Private Claim.
- A fail-closed verifier stub permits local instruction testing but cannot fulfill any payment. A real verifier and a known-good proof must be integrated and exercised before deployment.
- The memo and public values bind the payout to the Reservation. A released nonce can be reserved again, but an old payment can only fulfill a Reservation for its original payout; permanent nonce consumption is unnecessary. Payment Tags still prevent claiming a payment twice.
- Release uses two steps. RequestRelease records Solana time once the relay tip exceeds expiry plus Finality Depth, or the Relay is Halted and the Reservation is at least 24 hours old. ReleaseReservation returns stock and rent after a 15-minute Claim window. Claims remain eligible until the Reservation closes. A later Halt does not restart that window.
- The 24-hour backstop requires a Halt: a running Relay stalled below the height threshold can leave stock reserved indefinitely. Neither release path can refund ZEC.
- These layouts replace an undeployed revision: no live Vending accounts require migration. A future deployed layout or verifier change requires a separate migration decision.
