# Checkout demo

Ben configures 100 tickets at 0.001 TAZ once. Ana reserves on Solana devnet,
pays from an existing Zcash-testnet wallet, and uses a Payment Proof to Claim
one ticket. Ben has no per-sale step. The relay and keyless prover must run.

A real payment and SP1 execution are recorded; a Groth16 vending proof and
successful ticket mint remain unverified. See the vending program's
[current evidence](../programs/vending/README.md#live-demo-evidence-2026-10-27).

## Vending checkout

Set `VITE_CHECKOUT_MODE=vending`; the default `checkout` mode keeps the
existing merchant/wallet flow. Configure `VITE_VENDING_MACHINE`,
`VITE_VENDING_PROGRAM`, `VITE_VENDING_ADDRESS` (testnet Ironwood-only UA),
`VITE_VENDING_RPC`, `VITE_PROVER_URL`, and `VITE_VENDING_ABI_URL`.
The last URL must serve the authoritative machine-setup artifact described
in the [SDK deployment binding](../sdk/README.md#deployment-binding).
The page disables payment until it observes the testnet relay advance;
120 seconds without progress disables it again.

Connect a devnet Solana wallet, Reserve, then scan or open the ZIP-321
request in an Ironwood-capable testnet wallet. Keep the exact amount and
memo; pay once from shielded funds. Fees are extra. The page watches for
the payment, reuses the existing finality calculation, requests a proof,
and asks the Solana wallet to submit Claim. The success panel describes
the minted ticket without an Escrow transaction-ID privacy claim.

Nonce, detected payment, and submitted Claim signature are saved locally
for recovery. After reload, reconnect the same wallet and use **Check
reservation**. A pending Claim must be checked before another submission.
Do not clear browser storage during a purchase. Proof failures can be
retried without another Zcash payment. Release returns Solana rent and
stock; it cannot refund Zcash. **Request release** first becomes available
after expiry plus Finality Depth (or the Halt backstop). It starts a 15-minute
Claim window, shown as a countdown. **Release** becomes available when that
window ends. A detected payment can still Claim during the window. The page
refreshes stored release requests, including requests made by another signer.

### Start the vending page

From the repository root, install the client and page dependencies, then
load the public devnet configuration without changing your local env file:

```sh
pnpm --dir clients/vending-js install --frozen-lockfile
pnpm --dir clients/vending-js build
pnpm --dir sdk install --frozen-lockfile
pnpm --dir shop install --frozen-lockfile
set -a
. shop/.env.vending.example
set +a
devserve --name zolar-vending-shop --port 5188 --mem 500M --force -- \
  pnpm --dir shop dev --host 127.0.0.1 --port 5188 --strictPort
```

Open `http://127.0.0.1:5188`. The example targets the deployed demo Machine.
For a new shop, use its own public configuration and authoritative binding.
Follow [prover preparation](../programs/vending/README.md#prepare-a-proving-host) and forward
its loopback API on port 8088 before making a purchase. Execute-only mode
cannot mint a ticket. The Solana wallet signs Reserve and Claim and pays
Solana fees/rent; the Zcash wallet signs and pays ZEC plus its wallet fee.
Keep the Reservation and browser storage until settlement is verified.

Shop sales are public through the dedicated Shop Viewing Key. Vending does
not provide Escrow Private Claim transaction-ID privacy or a ZEC refund.
Never put a Payer's wallet keys, seed, or viewing key into browser settings.

## Escrow checkout with an external Zcash wallet

The customer connects a Solana wallet to receive a ticket and an **existing Zkool testnet wallet**
to pay shielded ZEC. Zkool owns the Zcash keys and signs the actual transaction. `zolar-connect`
is a local communications/proof service; it has no seed phrase, Zcash wallet database or spending
key. The shop refuses the old development wallet helper as a payment connection.

Supported wallet API: official [Zkool 6.35.0](https://github.com/hhanh00/zkool2/releases/tag/zkool-v6.35.0),
`zkool_graphql`, API version 1.0. This is a local wallet-service connection, not a browser extension
or Vizor integration. Software API signing has no wallet-native confirmation popup: the local
connector explicitly asks permission for the agreed payment before invoking the wallet signer.

### Prerequisites

- A funded existing Zkool **testnet** account with spendable shielded ZEC.
- Its authenticated GraphQL service on a literal loopback address, such as
  `http://127.0.0.1:8000/graphql`, started with `--coin 1` and a current NU7 testnet
  lightwalletd such as `https://testnet.zec.rocks:443`.
- An ES256 JWT issued by that wallet service's owner with `{sub: ACCOUNT_ID, write: true, exp: ...}`.
  The connector refuses admin tokens (`sub: 0`). Keep this delegated spending capability private;
  never put it in a `VITE_` variable, URL, browser storage, screenshots or source control.
- The existing funded merchant, ticket inventory, Solana devnet fee key, Private Claim proving
  artifacts and local prover from the root README. The fee key only pays Solana fees.
- A devnet Escrow deployment matching this build's 144-block expiry policy. The older deployment
  rejects the wallet proposal before any payment. [ADR 0006](../docs/adr/0006-external-wallets-keep-their-prepared-payment-effects.md)
  records the longer inventory reservation and required safety checks.

The wallet service's own setup uses its existing database and encryption configuration. Supply its
password to Zkool, never the shop or connector. For example (replace paths with your configuration):

```sh
zkool_graphql --coin 1 --db-path "$ZKOOL_DATABASE" \
  --db-password-file "$ZKOOL_PASSWORD_FILE" \
  --jwt-public-key-file "$ZKOOL_JWT_PUBLIC_KEY" \
  --lwd-url https://testnet.zec.rocks:443 --port 8000
```

Do not run two wallet service instances against the same database. Do not use a mainnet wallet or
an unauthenticated API for the demo. Never import your Vizor seed into Zolar.

### Run the checkout

Build `zolar-connect` and `zolar-merchant` from `cli/Cargo.toml`, using the repository's documented memory cap. Install `circuits/` dependencies (`pnpm --dir circuits install --frozen-lockfile`)
and build the local development prover (`cargo build --release -p dev-groth16`, with the same memory cap)
when using the supplied development proving key. The proving scripts need both before payment. Start the existing merchant as documented in the [SDK README](../sdk/README.md), with
its origin allowlist matching the shop. Then run the connector:

```sh
# ZOLAR_ZKOOL_TOKEN is supplied privately through the environment.
cli/target/release/zolar-connect \
  --allow-origin http://localhost:5175 \
  --zkool-url http://127.0.0.1:8000/graphql --zkool-account "$ZKOOL_ACCOUNT" \
  --fee-payer "$SOLANA_DEVNET_FEE_KEYPAIR" \
  --prove-script "$PWD/circuits/scripts/prove.sh" \
  --zolar-data "$ZOLAR_DATA" --data-dir "$ZOLAR_CONNECT_DATA" \
  --txid-check "$PWD/scripts/check-txid-absent.sh"

pnpm --dir shop install --frozen-lockfile
pnpm --dir shop dev --host localhost --port 5175 --strictPort
```

Optional `--zcash-peer host:port` arguments replace DNS discovery with a responsive testnet peer pool.
Headers and blocks are still checked against the trusted Relay; supplying a peer does not bypass verification.
Explicit peers retain a connection during the block scan to avoid repeated-handshake limits. A selected
peer can observe the scan and its last requested block, which may identify the payment block through
timing. Use several independently operated peers for availability. Default DNS discovery continues
to choose random peers per block.

Public browser variables in `.env.local`: `VITE_MERCHANT_URL`, `VITE_WALLET_URL` (the connector URL,
default `http://127.0.0.1:7878`), `VITE_ITEM_ID`, and `VITE_CLUSTER=devnet`. No credentials belong here.

1. **Connect wallet:** select your Solana wallet, or paste its receiving address. This requests no
   Solana signature or payment.
2. **Connect Zcash wallet:** explicitly select the identified Zkool testnet account. The configured origin can already discover its account identity; Connect selects it for checkout.
   Selection does not authorize a payment. A mainnet wallet, missing identity or embedded helper is refused.
3. **Pay with Zcash:** review the price, ticket and receiving address. **Authorize Zkool payment**
   opens the local connection's authorization page, which identifies the account and requesting shop.
4. Approve or reject. Only approval lets the connector ask Zkool to prepare the payment, send the
   merchant a redacted copy, independently check the funded Escrow, then request a real signature.
5. The returned signed transaction ID and expiry are verified locally before its exact bytes are
   saved and broadcast. The connector waits for finality, proves locally, and claims the ticket.

Keep the connector and Zkool running. The page only sees connection identity and checkout stages;
it never receives the private PCZT, signed transaction, commitment salt or wallet JWT. A declined or
expired authorization sends nothing. After broadcast uncertainty, restart with the same connector
data directory: recovery reuses the exact signed payment, never creates a replacement. The page restores the same
approved job after an outage or reload and shows its saved item, price and receiving address.

### Current limits

- One unresolved checkout per connected wallet. Sync before preparing another payment. Zkool's
  GraphQL broadcast API does not reserve spent notes as its native send flow does; concurrent
  spending from another client can conflict.
- Ordinary software-wallet v6 shielded payments only. FROST's long expiry, transparent-funded
  payments, issuance and unknown package formats are refused. Other account types need verification.
- The Header Relay must remain within 24 blocks of the wallet's live tip. Signing and sending
  require six remaining blocks and at least 30 minutes before the Escrow's refund backstop.
- Release availability, unit tests and a successful connection do not establish a paid checkout.
  Acceptance requires a real external-wallet signature, settled Private Claim and explicit txid-
  absence check. Record those signatures before calling the demo ready.

## Event disclaimer and limits

“Shielded Night” is a fictional, unofficial Zcash party during Breakpoint
London, shown as November 16, 2026 at 21:00. It is not affiliated with
Breakpoint or the Solana Foundation. The devnet token is a demo ticket,
not admission to a real event.

This demo uses Zcash testnet and Solana devnet, with the existing Zkool wallet
performing the Zcash signing. A Private Claim needs about 2 GiB of memory
and about one minute of local proving, with a single-party development
Groth16 key. See the [SDK limits](../sdk/README.md#limits).

## Checks

```sh
pnpm --dir shop lint
pnpm --dir shop test
npx --prefix shop --no-install tsc --noEmit -p shop
pnpm --dir shop build
```

### Browser fixtures

The dev-only `/fixture.html?state=payment` harness uses mocked Solana RPC,
a mock Wallet Standard transport, and the revision-2 prover API. It sends
no transactions and produces no real proofs. Other states are `connect`,
`reserve`, `finality`, `proving`, `claiming`, `success`, `sold-out`, `expired`,
`halted`, `stalled`, `proof-failed`, `claim-failed`, `release-window`, and
`release-ready`.

```sh
devserve --name zolar-vending-shop --port 5188 --mem 500M --force -- \
  pnpm --dir shop dev --host 127.0.0.1 --port 5188 --strictPort
python3 shop/scripts/check-vending-browser.py --url http://127.0.0.1:5188
```

The script uses one agent-browser session, checks every state at 1440 and
390 pixels, writes screenshots under `.zolar-work/screenshots/`, and closes
the session. Payment screenshots are taken before any clipboard interaction;
they must have no copy error. Unit tests verify clipboard success, denial and
recovery. Once payment is detected, payment controls and pre-payment notes
are hidden through finality, proof, Claim and failure states.
The fixture page is excluded from the production Vite entry points.
