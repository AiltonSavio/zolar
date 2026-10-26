# Build on Zolar

`@zolar/sdk` connects a shop page to `zolar-merchant` (the Payee's service)
and `zolar-wallet` (the Payer's local helper). The Payer pays shielded Zcash;
a Private Claim releases the Payee's SPL tokens to the chosen Solana address.
See the [demo shop](../shop/README.md) for a working page.

## Run the services

Use Zcash testnet and Solana devnet. Follow the root README's
[wallet and funding steps](../README.md#wallets-and-funds-once) first. The
Payee needs a funded Solana keypair, token inventory in its associated token
account, a Zcash unified address file, and a unified full viewing key file.
The Payer needs a funded Zcash wallet, its identity file, the patched
`zcash-devtool`, and a Solana fee keypair. Keep keys and wallet data outside
the repository.

Build the CLI binaries on a machine with enough memory:

```sh
cargo build --release --manifest-path cli/Cargo.toml
```

Create a catalog JSON array outside the repository. For example, replace the
mint placeholder with your devnet token mint:

```json
[
  {
    "id": "ga",
    "name": "General admission",
    "mint": "YOUR_DEVNET_TOKEN_MINT",
    "amount": 1,
    "price_zats": 100000
  }
]
```

`amount` is in the mint's smallest units; `price_zats` is in zatoshis. Use a
mint without a freeze authority: the wallet helper refuses one with it.
Run these commands from the repository root, in separate terminals. The
paths below are examples; use your own prepared files.

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

The merchant creates private-only Escrows. Its worker marks Claims and
refunds abandoned Escrows after the protocol's refund conditions are met.
Orders and payment proposals are stored under `--data-dir`; keep it private.

The Payer also needs the depth-8 proving artifacts and local prover from the
root README's [Private Claim instructions](../README.md#the-swap). Point
`--zolar-data` at the directory containing `build/d8/`. Pass `--prove-script`
explicitly when using a binary built on another machine, because its default
contains the build machine's source path.

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

Both services default to the devnet relay for the chosen Zcash network;
`--relay <address>` overrides it and must agree on both sides. Repeat
`--allow-origin` for additional exact shop origins. `localhost` and
`127.0.0.1` are different origins.

Merchant controls: `--max-open-orders` (10), `--max-orders-per-payout` (1),
`--order-ttl-secs` (600), and `--sweep-secs` (30). Wallet controls:
`--max-price-zats` (1000000) and `--mem-max` (2500M). The optional
`--txid-check` runs the transaction ID search after a Claim. Without it, or
if the search cannot complete, `txid_found_on_solana` is `null`; only `false`
means the search ran and found nothing. Run either binary with `--help` for
all flags and environment variables.

## Integrate a page

The package lives in this repository. The demo resolves `@zolar/sdk` to
`sdk/src/index.ts` in its Vite and TypeScript configs; use that setup for a
local integration. `pnpm --dir sdk build` emits JavaScript and types in
`sdk/dist/` for a package consumer.

Add a button, a payout address field, a status area, and a fallback link:

```html
<input id="payout" aria-label="Solana payout address" />
<button id="pay">Pay with Zcash</button>
<p id="status" role="status"></p>
<a id="approval" target="_blank" rel="noreferrer" hidden>Approve in wallet</a>
```

```ts
import {
  checkout, MerchantClient, openApprovalWindow, WalletClient,
} from '@zolar/sdk';

const merchant = new MerchantClient('http://127.0.0.1:7979');
const wallet = new WalletClient(); // http://127.0.0.1:7878
const pay = document.querySelector<HTMLButtonElement>('#pay')!;
const payout = document.querySelector<HTMLInputElement>('#payout')!;
const status = document.querySelector<HTMLElement>('#status')!;
const approval = document.querySelector<HTMLAnchorElement>('#approval')!;

pay.addEventListener('click', async () => {
  // Open before any await, directly inside the click handler.
  const approvalWindow = openApprovalWindow();
  pay.disabled = true;
  approval.hidden = true;
  try {
    const result = await checkout({
      merchant, wallet, itemId: 'ga', payout: payout.value.trim(),
      approvalWindow,
      onApprovalUrl(url) {
        approval.href = url; // Fallback when the popup is blocked.
        approval.hidden = false;
      },
      onStage(view) { status.textContent = view.stage; },
    });
    const end = result.final;
    status.textContent = end.stage === 'done'
      ? `Claim: ${end.claim}`
      : end.stage === 'failed' ? end.reason : 'Payment rejected';
  } catch (error) {
    approvalWindow?.close();
    status.textContent = String(error);
  } finally {
    pay.disabled = false;
  }
});
```

`MerchantClient.items()` reads the catalog and available inventory;
`order(orderId)` reads an order's status. `WalletClient.status()` returns
the wallet's network and relay, or `null` if unavailable or disallowed.
`checkout()` creates an order, starts the wallet job, opens approval, and
polls stages. `onOrder` receives the terms. Request errors throw; terminal
`rejected`, `failed`, and `done` states return in `result.final`.

## Show progress

Use `STEPS` for the six display labels and `stepIndex(view)` for the current
index. Earlier indices are complete; `STEPS.length` means done, and `-1`
means failed. Handle rejection and failure separately from the stepper.

| Wallet stage | Display step |
|---|---|
| `awaiting_approval` | Approve |
| `building_payment`, `locking_item`, `checking_escrow` | Lock |
| `paying` | Pay |
| `waiting_for_finality` | Finality |
| `proving` | Local proof |
| `claimed` | Claim submitted |
| `done` | Complete |
| `rejected`, `failed` | Terminal outcome |

`finalityProgress(view)` returns `{ done, of, tip }` while waiting, or `null`
before relay data is available. It measures blocks after the first block
the payment could be in, not a lookup of the actual payment's block.
`onStage` receives changes to the full job view, including relay heights.
The default polling interval is two seconds. An `AbortSignal` stops the
page watching; the wallet keeps working. Keep the wallet running after
payment so it can finish the Claim. Jobs are persisted for restart recovery.

## HTTP boundary and security

The merchant serves `GET /items`, `POST /orders`, `GET /orders/{id}`,
`GET /orders/{id}/terms`, and `POST /orders/{id}/escrow`. Only the wallet
sends the payment proposal (base64 redacted PCZT) to the last endpoint and
receives the commitment salt.

The wallet listens only on `127.0.0.1`, checks the Host, and allows exact
shop Origins. The page uses `GET /status`, `POST /checkout`, and
`GET /jobs/{id}` (with the returned bearer token). Approval is shown at
`GET /approve/{secret}` and submitted by `POST` on the wallet's own origin;
the approval covers a digest of the terms and expires after ten minutes.
The wallet checks the Escrow before paying. The shop page never receives
the Zcash transaction ID or salt. The Payee gets the redacted PCZT, which
removes the Payer's viewing key, spent-note values, change data, and witness.
See the root README's [privacy table](../README.md#what-each-party-learns)
for what the Payee and network services still learn.

## Limits

- The local helper stands in for Zcash wallet support; this is not a
  checkout supported by ordinary Zcash wallets today.
- A Private Claim needs about 2 GiB of memory and about one minute of local
  proving. Its payment block can contain at most 256 transactions.
- This checkout is a Zcash testnet / Solana devnet demo, with test tokens.
- Groth16 uses a single-party development key. Whoever generated it could
  forge Claims. A multi-party ceremony has not been completed.

## Checks

```sh
pnpm --dir sdk install --frozen-lockfile
pnpm --dir sdk typecheck
pnpm --dir sdk test
pnpm --dir sdk build
```

## Vending checkout (testnet / devnet)

Ana connects her Solana wallet and reserves one ticket at 100,000 zatoshi.
The SDK builds a ZIP-321 request for exactly `0.001` testnet ZEC; wallet
fees are extra. After payment reaches the Header Relay's Final Block Tree,
the stateless prover creates a Payment Proof and Ana signs the Claim that
mints her ticket. She needs devnet SOL for rent and Solana fees. The Payee
runs no per-sale merchant service in this mode.

`encodeVendingMemo(machineBytes, nonceBytes, payoutBytes)` returns the
512-byte ZIP-302 memo: canonical ASCII
`zv1:<machine-base58>:<nonce-base58>:<payout-base58>`, followed by zeros.
All three inputs are exactly 32 bytes. `paymentLink(address, priceBigInt,
machineBytes, nonceBytes, payoutBytes)` puts the UTF-8 text (without ZIP-302
padding) in unpadded base64url and formats the amount using integer
arithmetic. `ironwoodReceiver(address)` checks the testnet single-receiver
UA checksum, F4Jumble encoding, padding, and sole type-3 receiver. Ironwood
uses that receiver's address format.

`VendingProverClient.findPayment(machine, nonce)` calls
`GET /find-payment?machine=<base58>&nonce=<base58>`. It accepts `pending`
or `found` with txid, block height, value, and payout.
`prove(machine, nonce, txid)` posts `/prove`, then polls `/prove/<job>`
through `waiting_finality` and `proving` to `done` or `failed`.
Proof/public-values strings are hexadecimal. The public values are 249
bytes, with payout at 189..221 and zero reserved bytes at 221..249.
The scanner's `value_zat` must be an exact JSON integer (or a decimal
string); unsafe JavaScript numeric integers are rejected.

### Deployment binding

All handwritten instruction encoding and account decoding lives in
`src/vendingInstructions.ts`. Replace it with the generated Codama client
when available. `createVendingChain(deployment)` reads the Vending Machine
and Header Relay directly from RPC, constructs Reserve/Claim/Release,
checks the devnet genesis hash, and waits for confirmed transaction status.
`VendingConnection` is `{ address: string, send(transaction: Uint8Array):
Promise<string> }`; `send` signs and broadcasts serialized Solana transaction
bytes and returns a base58 signature. The shop binds it to Wallet Standard's
`solana:signAndSendTransaction` on `solana:devnet`.

The shared interface has no authoritative instruction selectors, ordered
account metas, or Machine/Reservation offsets. The SDK therefore requires
a public **deployment-owned `VendingAbi` JSON artifact** at `abiUrl` instead
of guessing these values. Its TypeScript schema is exported. It contains:

- `version: 1`, `machineAddress`, `programAddress`.
- `shopReceiver`: the exact 43-byte receiver; `shopViewingKeyHash`: 32 bytes.
- `selectors.reserve/claim/release`: byte prefixes; `claimFields`: the exact
  order of `proof`, `public_values`, and optional `proof_len_u32` and
  `public_values_len_u32` fields (lengths are little-endian).
- `machine` and `reservation`: account length, discriminator/version byte
  prefix, and byte offsets for every named field. Price is u64 LE; stock,
  available, and expiry are u32 LE; creation time is i64 LE Unix seconds.
- `accounts.reserve/claim/release`: ordered `{ name, writable, signer }`
  entries. The exported type enumerates every available account role.

Machine setup must produce and audit the receiver/viewing-key association
as one artifact. The browser verifies the entire configured receiver
against the pin, SHA-256 of the on-chain 64-byte Shop Viewing Key against
both the machine and the pin, and the diversifier against the receiver.
It does **not** derive Pallas public keys in JavaScript. Missing bindings,
wrong identities, invalid offsets, wrong account owners, pending relays,
and mainnet relays fail closed. The fixture ABI is illustrative test data,
never a deployment artifact. No deployed program/ABI conformance is claimed
by these fixtures.

`VendingChain` exposes `read`, `reserve`, `restore`, `release`, `claim`,
and `transactionStatus`. The shop saves a nonce before Reserve and retains
Claim signatures across ambiguous confirmations; reuse the same nonce for
recovery. Release requires `tip > expiry + Finality Depth`, or a halted
relay and 24 hours of Solana time on-chain. Client time only controls the
button; the program enforces the rule.

Vending publishes the shop's incoming payment information. It does not
have the Escrow Private Claim's txid-hiding guarantee. Proof creation needs
a working external prover; the browser never proves. These testnet tickets
are transferable immediately, with no extra settlement window, and are
not mainnet-safe.

Address/request encoding references: [ZIP 316](https://zips.z.cash/zip-0316)
and [ZIP 321](https://zips.z.cash/zip-0321).
