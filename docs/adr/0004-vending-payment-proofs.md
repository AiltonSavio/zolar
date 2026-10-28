# Vending program integration

The governing decision is [ADR 0005: Vending Machines claim with SP1 Payment Proofs](0005-vending-machines-claim-with-sp1-payment-proofs.md).
Its revision 2 memo, public values, Shop Viewing Key and two-step release rules
also govern the program. The program uses the real `sp1-solana-v6` verifier and
`vending-types` shared with the guest. No stub verifier is shipped.

These layouts replace an undeployed revision; no live Vending accounts require
migration. A future deployed layout or verifier change requires a separate
migration decision. A positive vending Claim still needs a real Payment Proof;
verifier fixture acceptance alone does not establish ticket minting.
