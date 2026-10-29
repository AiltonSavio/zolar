# External wallets keep their prepared payment effects

The checkout integrates the existing Zkool testnet wallet through its released GraphQL API. A local
communications and proof connector holds an account-scoped API capability, never the customer's
Zcash spending keys. Connecting selects the account; a separate authorization permits one payment
with frozen merchant, price, memo, ticket mint, amount and Solana receiving address.

The wallet prepares a PCZT package before broadcast. Only its privacy-redacted PCZT goes to the
Payee. After the connector checks the funded Escrow, Zkool signs the original package. The connector
computes the returned raw transaction's ID and expiry locally, checks both against the committed
payment, saves the exact signed bytes, rechecks the Escrow and expiry, then broadcasts. Recovery
reuses those bytes and never prepares another payment for the same order.

## PCZT wire compatibility

Zkool 6.35.0 uses the Zcash Shielded Assets fork of PCZT v2, which inserts fields even for ordinary
ZEC payments. A version number alone does not identify a compatible wire schema. The adapter fully
decodes that pinned schema, rejects issuance, non-ZEC assets and split-note extensions and ZSA note versions,
and removes only absent extension markers or an explicitly verified native-ZEC asset base in its inspection copy. Standard fields, transaction
effects, ciphertexts and expiry are preserved. Trailing bytes and unsupported encodings are refused.
The original wallet package is retained byte for byte for signing; the normalized copy goes through
the existing privacy redactor before the Payee sees it. The returned raw transaction must match the
inspection copy's exact transaction ID and expiry before broadcast.

The schema is pinned to [the wallet's PCZT fork](https://github.com/zcash-shielded-assets/librustzcash/tree/fb3b4de5c304d2f07b0d8bf96f87f0c8337c68d5/pczt/src).
A fresh-context review verified the Orchard layout. Independent source comparison found Global and
Transparent identical to the existing parser and only getter attributes changed in Sapling.

## Expiry policy

The maximum expiry accepted at Escrow creation is 144 blocks beyond the Header Relay tip. This
admits ordinary external-wallet expiries (Zkool uses 120 blocks under NU7), with an independently
enforced maximum wallet-tip/Relay lag of 24 blocks. The connector refuses unsupported long expiry,
including FROST's 3456-block preparation. It does not rewrite the wallet's transaction effects:
IO-finalized PCZTs already include dummy signatures bound to those effects.

The fixed payout, Payment Commitment, exact payment/Escrow expiry equality, Claim height bounds,
Finality Depth, strict refund eligibility, 15-minute Claim window and 24-hour backstop are unchanged.
No account layout, instruction serialization or circuit input changes. The connector also requires
six remaining live-chain blocks before signing and sending, and 30 minutes before the backstop.

## Consequences

- The Payer's option and merchant inventory reservation can last longer than with the previous
  20-block cap: 144 NU7 testnet blocks are about one hour, plus Finality Depth and the Claim window.
  Merchant order limits remain necessary; this is not a free compatibility change.
- The wallet endpoint is a literal loopback address with redirects disabled. Account-scoped JWTs
  are signing capabilities and stay in the local connector. Admin tokens are refused; no seed,
  passphrase or viewing-key fields are queried.
- Zkool software RPC signing has no native wallet confirmation dialog. The connector's authorization
  page identifies the wallet and account and describes its delegated signing honestly. It is not
  presented as a Zkool application popup or as a new wallet.
- Zkool's GraphQL broadcast API does not reserve spent notes in its wallet database. The connector
  admits one unresolved payment at a time and syncs before another preparation. Concurrent native
  wallet use can still conflict; ambiguous broadcasts retain their original payment identity.
- The web demo requires an existing identified external testnet wallet. The old development CLI
  helper is retained for protocol tests and cannot satisfy the shop's connection requirement.
- An explicitly configured P2P pool may retain a connection across its block scan. Every response
  must match the current requested header hash and transaction Merkle root; failures drop the
  connection and do not advance the scan. This avoids peer handshake limits, but the selected peer
  can observe the full scan and stopping block and may infer the payment block through timing.
  Default DNS discovery continues to choose a random peer for each block.
- This policy is for the testnet/devnet checkout. No Solana mainnet deployment is authorized.

## Read access after a restart

An exact allowed shop origin can discover its latest approved checkout for the configured Zcash
account. Active jobs take priority over receipts, including approved preparation before signing.
The response contains only the job ID and read token, is not cached, and grants no approval or
payment capability. It exposes no approval URL, transaction ID, salt, PCZT or wallet credential.
The page waits for this check before enabling a new order, restores the frozen item/price/payout,
and keeps watching the same job through outages. Reload and status recovery perform GET requests
only. An active checkout cannot be dismissed to permit another payment.

Identity discovery is already authorized by the wallet owner's exact origin allowlist while the
connector runs. The Connect button selects this configured account; it is not a new identity-
disclosure or native wallet-permission boundary. Payment requires separate own-origin approval.

## Review

A fresh-context adversarial review required explicit tip-lag and backstop checks, account-scoped
permissions, unchanged packages, durable signed bytes, and serialized payments. Those objections
were accepted. Broader wallet expiry was accepted as an explicit bounded inventory/option tradeoff,
rather than modifying signatures or claiming ordinary send links preserve escrow-first settlement.

Source: [Zkool GraphQL API](https://github.com/hhanh00/zkool2/blob/59a558fb7383c03b3633f015b33ef120cfbc3bdc/rust/src/graphql/query.rs),
[planner and expiry](https://github.com/hhanh00/zkool2/blob/59a558fb7383c03b3633f015b33ef120cfbc3bdc/rust/src/pay/plan.rs),
[broadcast behavior](https://github.com/hhanh00/zkool2/blob/59a558fb7383c03b3633f015b33ef120cfbc3bdc/rust/src/pay/broadcast.rs).
