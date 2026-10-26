# Vending Machines claim with SP1 Payment Proofs

Ben sets up 100 tickets at 0.5 testnet ZEC (50,000,000 zatoshi) once. Ana
reserves one ticket on Solana devnet before paying, then sends exactly 0.5
testnet ZEC to Ben and separately pays her wallet's transaction fee. A Payment
Proof lets anyone mint her ticket to the payout wallet in her Reservation.
Ben runs nothing after setup; permissionless header relaying and a prover
service still need to run.
Ana pays the Reservation's rent, recovered when it closes, and needs SOL for
transactions. Whoever submits the Claim also funds its permanent Payment Tag
account and any missing associated token account; v1 has no reimbursement pool.

This records the ticket 16 design on October 26, 2026. Implementation and
end-to-end verification are in progress. It is scoped to Zcash NU7 testnet
v6/Ironwood payments and Solana devnet tickets with no monetary value.

## Decision

Use a **Vending Machine** with immutable sale configuration, a **Reservation**
before payment, and an SP1 v6.10.0 Groth16 **Payment Proof**. The proof checks
what the shop received, rather than a transaction chosen before the Payee
locks an Escrow. The **Payment Tag** is the nullifier of the Ironwood action
containing that output. A successful Claim consumes one Reservation and mints
one ticket, regardless of any overpayment.

The wallet survey found no usable released browser-wallet flow that exposes
a pre-broadcast transaction ID to a dApp. Noir signs and broadcasts before
returning the ID; Vizor hands off a payment request without returning it.
Zkool's local service can prepare a payment, but requires the rejected local
service workflow. The MetaMask Snap and hardware signers need a host-built
PCZT and have network or integration constraints. A transaction ID existing
before signing does not mean a dApp can obtain it from the wallet.

Requiring Ben to inspect each PCZT and create each Escrow does not meet the
checkout requirement that he runs nothing after setup. Pay-then-lock was
also rejected: Ana's irreversible payment would depend on Ben subsequently
locking the ticket. The Reservation instead holds stock before she pays,
and the Payment Proof authorizes minting without another Payee action.
It does not make the two chains atomic or provide a ZEC refund.

### Shop Viewing Key and immutable configuration

Ben publishes a **Shop Viewing Key**, the external incoming viewing key
(`dk || ivk`, 64 bytes), from a dedicated ZIP-32 account used only for the
shop. The shop uses an Orchard/Ironwood-only unified address so a wallet
cannot choose a transparent or Sapling receiver. The key exposes incoming
values, memos, and payment timing for the account's external addresses. It
does not grant spending authority or disclose the Payer's viewing key.

The machine pins the price, initial stock, shop diversifier, SHA-256 hash of
the Shop Viewing Key, Header Relay, guest verification-key hash, and
Reservation duration. Stock availability changes through Reserve and
ReleaseReservation; the configuration cannot be updated or withdrawn in v1.
Key rotation or a changed guest requires a new machine. The public key and
address must remain available to the page and prover; a key hash alone is
not enough to recover them.

Program configuration immutability does not remove trust in an upgrade
authority. The devnet programs are unaudited, and this decision makes no
mainnet deployment or immutability guarantee about their executable code.

### Reservation and memo

Ana signs Reserve with a random 32-byte nonce. It decrements available stock,
stores her payout wallet and price, and sets an expiry height from the relay
tip plus the machine's Reservation duration. A failed Reserve means Ana
must not pay. The link requests exactly the price; the Claim accepts an
output worth at least that price and returns no excess ZEC.

The 512-byte memo is ZIP-302 arbitrary data with exactly this encoding:

| Byte range | Meaning |
|---|---|
| `0` | `0xFF`, arbitrary data |
| `1` | `0x01`, version 1 |
| `2..34` | Machine public key, 32 bytes |
| `34..66` | Reservation nonce, 32 bytes |
| `66..512` | Zero padding |

The ZIP-321 `zcash:` link carries the memo as base64url without padding.
Other versions, encodings, or trailing data are rejected. The nonce identifies
the Reservation; the payout comes from that account, not a wallet-supplied
memo address. The machine key binds the payment to one machine. V1 is
devnet-only and has no explicit Solana cluster identifier in the memo.

### Payment Proof and Claim

The guest uses the pinned librustzcash revision
`eb38d7a373661179dbc083a2fd58b3a6c9b25818` to parse the whole v6 transaction
and recompute its transaction ID, including all Ironwood actions and the
consensus branch ID. It proves a SHA-256d transaction Merkle path to the
block's Merkle root, then the relay's Poseidon leaf and 20-level path to a
Final Block Tree root. Tree encodings match the existing Header Relay.

For the selected Ironwood output it trial-decrypts with the Shop Viewing
Key, checks note validity including equality of the recomputed note
commitment `cmx`, and binds the recipient to the configured diversifier
(`g_d = DiversifyHash(d)`). Decryption alone is insufficient: a forged value
must not satisfy a different note commitment. The guest also checks the
exact memo and derives the Payment Tag from the same paying action.

Version 1 public values are a fixed 249-byte record: version, branch ID,
machine, nonce, note value, Payment Tag, Final Block Tree root, payment
height, diversifier, Shop Viewing Key hash, Ironwood pool byte `0x03`, and
60 zero reserved bytes. Integers are little-endian; roots retain the relay's
byte order. The SP1 v6 Solana verifier must bind the guest verification-key
hash and the hash of these exact public-value bytes.

Claim checks the running Header Relay's owner and configured address, the
known tree root, the branch ID, machine, diversifier, key hash, Reservation,
payment height at or below expiry, and value at least the reserved price.
There is one Finality Depth for the relay, as in [ADR 0003](0003-relay-wide-finality-depth-and-halt-on-deep-reorgs.md).
The Payment Tag account is permanent and scoped to the machine; creation
fails on reuse. Anyone may submit a valid proof, but minting always pays
the current Reservation account's payout wallet. The program alone controls
the ticket mint authority. Claim closes the Reservation and returns its rent
to that payout wallet.

ReleaseReservation returns stock and rent once the relay tip is strictly
above expiry plus Finality Depth, or once the relay is Halted and 24 hours
of Solana time have passed since Reservation creation. This is a single
release step, with no Escrow-style 15-minute Claim window. A running relay
that stalls below the height threshold does not satisfy either release
condition. Releasing stock never refunds a Zcash payment.

### Why SP1 here

[ADR 0002](0002-private-claims-use-a-locally-proved-groth16-circuit.md)
rejects SP1 for a Private Claim because its transaction ID and salt must
remain secret from a hosted prover. A vending witness contains public chain
data and a public Shop Viewing Key. There is no secret witness to protect,
so a hosted prover adds no disclosure of payment contents. It can still
associate a request's IP address with a payment and Solana wallet.

This reverses the SP1 rejection for the Payment Proof statement only.
Escrow Private Claims still use their local circom Groth16 circuit and keep
their own setup requirements. SP1 reuses the Rust transaction and note
validation code and its vendor's Groth16 setup, rather than requiring a new
Zolar decryption circuit and ceremony. The zkVM, its wrap, and the ported
Solana verifier become dependencies in the trust model.

Hardware and proving cost remain. V1 plans to prove on a machine with at
least 32 GB RAM or suitable GPU hardware; complete-proof resource
requirements remain unverified. Proving in Ana's browser is not the plan.
Someone must fund and operate the stateless prover and header relaying.
Prover censorship, queues, or downtime can prevent a timely Claim. Services
cannot change the payout of an existing live Reservation; nonce reuse after
closure still needs a protocol fix, as described below.

## Consequences

| Property | Lock-first Escrow | Vending with a Reservation, v1 |
|---|---|---|
| Last unit and fixed price | Tokens locked for the Payer before payment | Stock reserved and price fixed before payment |
| Payee offline | Payee must create each Escrow | Payee does nothing per sale; relay and prover remain necessary |
| Payer's Zcash identity | Hidden with shielded funding | Hidden with shielded-only funding; v1 checks this in the page, not the guest |
| Payment hidden on Solana | Private Claim hides transaction ID | Public key and Payment Tag link the sale to its Zcash action |
| Relay stall or Halt | Solana-time refund returns Escrow tokens to the Payee; it cannot reverse ZEC | Only stock and Reservation rent can be released; a running stall can block even that |
| Payee paid for issued tokens | Depends on proof of work over Finality Depth | Same assumption; automatic minting amplifies deep-reorg abuse |
| Wrong amount, receiver, pool, or memo | Payee checks the PCZT before locking | Payment may be unclaimable; overpayment still buys only one ticket |
| Proving | Local Private Claim, about 47 s and 1.8 GB in recorded runs | Server or suitable proving hardware required; full proof cost and latency unverified |
| Trusted setup | Zolar's per-circuit setup; current key is a development key | SP1's Groth16 setup and verifier release |
| Implementation evidence | Recorded devnet Private Claims | Native decode and SP1 execution spikes; no vending purchase proven end to end |

Shop sales are public. Anyone with the Shop Viewing Key can correlate the
amount, memo, paying action, and Solana payout. This does not carry the
Private Claim promise that Solana cannot identify the Zcash payment. Funding
from a transparent address can expose Ana's address; crossing pools can
expose net value movements. The page must request shielded-only funding,
but v1 does not enforce that in the Payment Proof.

Ana's ZEC is unrecoverable by the program if she uses the wrong amount,
address, pool, or memo, pays too late, or cannot claim during a relay or
prover failure. A voluntary refund needs Ben's cooperation and a return
address. The page must disable Pay for a stalled or Halted relay and state
these limits before wallet handoff. Reservation expiry is not a ZEC refund.

The guest checks no Halo2 transaction proof. It relies on the Header Relay's
proof of work and Finality Depth, not independent full transaction validity.
A sufficient proof-of-work attack can therefore support an invalid payment;
testnet difficulty makes that assumption especially weak. Tickets mint
without an additional settlement delay or non-transferable window. A later
deep reorganisation Halts Claims but does not revoke already minted tickets.
This is accepted for testnet demo tokens only.

Roots rotate through the relay's 128-root history. A proof queued beyond
that history must be made again against a fresh root, and release can win
while a valid payment is still awaiting proof. Permanent Payment Tag rent
grows with sales. There is no sponsored zero-SOL purchase in v1.

Reserve publishes the nonce before payment. There is no nonce commitment,
Reservation bond, per-Payer cap, or forfeiture. Someone can hold all stock
without paying ZEC and recover the rent after release, then repeat for
transaction fees. Reserving stock prevents a sold-out race for a live
Reservation; it does not guarantee shop availability.

Contract review also found an unresolved payment-redirection defect. A
released Reservation closes, but nothing forbids reserving its nonce again.
After Ana pays and her Reservation becomes releasable, Mallory can release
it, reserve the same nonce with Mallory's payout, and claim Ana's still-unused
payment. The proof binds the nonce, not the original payout or Reservation
generation, and only an upper payment-height bound is checked. This must be
resolved before accepting payments; the current interface cannot guarantee
that an unclaimed payment stays bound to the original Payer after release.
No alternative memo or public-values layout is selected by this record.

ZIP-229 and ZIP-230 are Draft. Format changes, guest changes, or SP1 release
changes can require a new guest hash and a new machine. The October 26
spikes measured about 15.9M cycles for decode and decryption and another
8.0M for a synthetic Poseidon path in a separate guest. Their combined cost
is an estimate; neither a complete Payment Proof nor equality with a real
relay root was demonstrated. Wallet NU7 sending and exact binary-memo
preservation also require validation; the survey found Vizor's binary-memo
handoff unsupported. Existing-wallet compatibility is not yet established.

## Considered Options

- **Per-sale Escrow with a PCZT:** preserves the existing Private Claim
  design, but requires Payee action per sale and wallet preparation APIs
  absent from the surveyed usable browser flows.
- **Pay-then-lock:** fits sign-and-broadcast wallets, but trusts Ben to lock
  after receiving an irreversible payment. Rejected for vending checkout.
- **Vending without a Reservation:** permits sold-out races after payment.
  Immutable configuration alone does not reserve the last ticket.
- **A custom circom or gnark decryption circuit:** requires rewriting
  Pallas arithmetic, Sinsemilla, BLAKE2b, and note encryption, with a new
  circuit review and setup. SP1 reuses the pinned Rust implementation.
- **RISC Zero:** a plausible Rust guest and Solana verifier alternative,
  but its proving and wrap path was not validated for this statement.
- **On-chain Poseidon membership:** saves guest cycles but adds a 20-sibling
  path to Claim and may need a proof-buffer upload. V1 keeps it in the guest
  and accepts root-history expiry.
- **Payee or service attestation:** avoids proving work but introduces an
  authority that can mint without a verified payment.

Vending tickets are an application of the relay, not wrapped ZEC. Ben owns
the received ZEC; a ticket has no programmatic ZEC redemption right.
[ADR 0001](0001-swaps-not-a-lock-and-mint-bridge.md) remains unchanged for
cross-chain value transfer and Escrow swaps.
