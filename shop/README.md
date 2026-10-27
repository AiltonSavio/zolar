# Checkout demo

The demo shop uses the [Zolar SDK](../sdk/README.md) to exchange shielded
Zcash testnet ZEC for a ticket token on Solana devnet. The shop needs no
keys; the Payee runs `zolar-merchant`, and the Payer runs `zolar-wallet`
on the same computer as their browser.

## Run locally

Run commands from the repository root. You need Node 22, pnpm 10, the
release CLI binaries, funded test wallets, and the Private Claim proving
artifacts. Follow the SDK's [service setup](../sdk/README.md#run-the-services)
for the catalog, keys, patched devtool, and circuit prerequisites. Keep
wallets and service data outside the repository.

1. Start the merchant in its own terminal, using your prepared file paths:

   ```sh
   cli/target/release/zolar-merchant \
     --bind 127.0.0.1:7979 --name 'Shielded Night' \
     --network testnet --rpc-url https://api.devnet.solana.com \
     --keypair "$HOME/zolar-wallets/payee-sol.json" \
     --ufvk-file "$HOME/zolar-wallets/payee.ufvk" \
     --address-file "$HOME/zolar-wallets/payee.address" \
     --catalog "$HOME/zolar-data/catalog.json" \
     --data-dir "$HOME/zolar-data/merchant" \
     --allow-origin http://localhost:5175
   ```

2. Start the wallet helper in another terminal:

   ```sh
   cli/target/release/zolar-wallet \
     --port 7878 --allow-origin http://localhost:5175 \
     --network testnet --rpc-url https://api.devnet.solana.com \
     --fee-payer "$HOME/zolar-wallets/payer-sol.json" \
     --devtool "$ZOLAR_DEVTOOL" \
     --wallet-dir "$HOME/zolar-wallets/payer" \
     --identity "$HOME/zolar-wallets/payer-key.txt" \
     --prove-script "$PWD/circuits/scripts/prove.sh" \
     --zolar-data "$HOME/zolar-data" \
     --data-dir "$HOME/zolar-data/wallet" \
     --txid-check "$PWD/scripts/check-txid-absent.sh"
   ```

3. Install the page dependencies and configure its public URLs:

   ```sh
   pnpm --dir clients/vending-js install --frozen-lockfile
   pnpm --dir clients/vending-js build
   pnpm --dir sdk install --frozen-lockfile
   pnpm --dir shop install --frozen-lockfile
   cp shop/.env.example shop/.env.local
   pnpm --dir shop dev --host localhost --port 5175 --strictPort
   ```

   Open `http://localhost:5175`. The shop resolves `@zolar/sdk` directly to
   the sibling SDK source, so no separate SDK build is needed for development.
   The exact origin must be allowed by both services. If these ports are
   already in use, reuse the existing demo or choose free ports and update
   the URLs and allowlists together.

The variables in [`.env.example`](.env.example) are public browser settings:

| Variable | Default | Meaning |
|---|---|---|
| `VITE_MERCHANT_URL` | `http://127.0.0.1:7979` | Payee service |
| `VITE_WALLET_URL` | `http://127.0.0.1:7878` | Payer's local helper |
| `VITE_ITEM_ID` | `ga` | Catalog item to display |
| `VITE_CLUSTER` | `devnet` | Solana explorer links |

Keep `VITE_CLUSTER=devnet`; it only controls explorer links, not the services'
RPC or network. Never put keys, viewing keys, identities, or salts in Vite
environment variables. Restart Vite after changing its environment.

Connect a Solana wallet or paste the address that will receive the ticket.
The shop requests only the address, with no Solana signature. Click **Pay
with Zcash**, then review and approve the terms in the wallet's own window.
If the popup is blocked, use the approval link on the page. Keep the helper
running while it pays, waits for Finality Depth, proves locally, and Claims.
The final privacy result is unknown if the optional transaction ID search
could not run; it is not then evidence that the ID was absent.

## Event disclaimer and limits

“Shielded Night” is a fictional, unofficial Zcash party during Breakpoint
London, shown as November 16, 2026 at 21:00. It is not affiliated with
Breakpoint or the Solana Foundation. The devnet token is a demo ticket,
not admission to a real event.

This demo uses Zcash testnet and Solana devnet. The wallet helper is a
stand-in for wallet support. A Private Claim needs about 2 GiB of memory
and about one minute of local proving, with a single-party development
Groth16 key. See the [SDK limits](../sdk/README.md#limits).

## Checks

```sh
pnpm --dir shop lint
pnpm --dir shop test
npx --prefix shop --no-install tsc --noEmit -p shop
pnpm --dir shop build
```

## Vending mode

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
