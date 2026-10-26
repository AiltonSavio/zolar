# Zolar

A Solana-side light client for Zcash, so Solana programs can act on shielded Zcash payments without a custodian, and the escrow-based swap built on it.

## Language

**Header Relay**:
The verified chain of Zcash block headers held on Solana, extended from the Checkpoint by anyone who submits valid headers.
_Avoid_: Oracle, bridge, light client (when meaning the on-chain state)

**Checkpoint**:
The starting height and recent header history the Header Relay is initialised with once; the system's one trust assumption.
_Avoid_: Genesis, anchor (a Zcash term for note-tree roots)

**Relayer**:
Anyone who submits Zcash headers or Inclusion Proofs to Solana; it holds no authority the program does not check.
_Avoid_: Oracle, operator, guardian

**Inclusion Proof**:
A Merkle path showing that a transaction ID is in the block of a header in the Header Relay.
_Avoid_: SPV proof, receipt

**Finality Depth**:
How many blocks must follow a block before Zolar treats it as final; one value for the whole Header Relay, fixed at the Checkpoint.
_Avoid_: Confirmation depth, confirmations (alone)

**Final Block Tree**:
The append-only record of every block that has reached Finality Depth, each with its transaction Merkle root and height.
_Avoid_: History tree (a ZIP-221 term), accumulator

**Halt**:
The state the Header Relay enters when the heaviest chain reorganises deeper than Finality Depth; it accepts no more headers.
_Avoid_: Pause, freeze

## Payments and swaps

**Payer**:
The party who sends the shielded ZEC.
_Avoid_: Seller, taker, sender

**Payee**:
The party who receives the shielded ZEC and, in a swap, locks the Escrow that pays the Payer.
_Avoid_: Buyer, maker, receiver

**Escrow**:
SPL tokens a Payee locks on Solana for a named Payer address, released to that address once a specific Zcash transaction is final, or refunded once it no longer can be.
_Avoid_: Vault, lock, deposit

**Payment Commitment**:
The hash of a transaction ID and a secret salt stored in an Escrow, so Solana never sees which Zcash transaction it waits for.
_Avoid_: Expected txid, payment hash

**Claim**:
The Payer's request to release an Escrow by proving its committed transaction is final.
_Avoid_: Withdraw, redeem

**Private Claim**:
A Claim made with a zero-knowledge proof, so the transaction ID is never revealed on Solana.
_Avoid_: ZK claim, shielded claim

**Bond**:
Optional tokens the Payer locks alongside an Escrow, returned on a Claim and paid to the Payee if the Payer never completes.
_Avoid_: Collateral, deposit, premium

**Refund**:
The Payee taking an unclaimed Escrow and its Bond back, in two steps: a request once the relay tip is above the expiry height plus Finality Depth (or 24 hours after creation), then the Refund itself after the Claim window.
_Avoid_: Withdrawal, cancel

**Claim window**:
The 15 minutes of Solana time between a refund request and the Refund, in which a Claim still settles, so a Payer whose payment finalises late is not raced.
_Avoid_: Grace period, timeout

## Vending checkout

**Vending Machine**:
The Solana account holding one shop's immutable config and stock.

**Shop Viewing Key**:
The Ironwood/Orchard external incoming viewing key of a dedicated ZIP-32 account, published for the shop. Reveals the shop's incoming payments only; its hash is stored in the Vending Machine.

**Reservation**:
A Solana account the Payer creates before paying; holds one unit of stock for them.

**Payment Proof**:
An SP1 v6.10.0 Groth16 proof that a Relay-final Zcash payment paid the Reservation.

**Payment Tag**:
The paying Ironwood action's nullifier; claims once per Vending Machine.
