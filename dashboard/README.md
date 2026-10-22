# Zolar dashboard

Read-only web page: both Header Relays (tip, chain work, Finality Depth, Final Block Tree size and latest root, Halt), open Escrows, and Escrow program transactions as they land (Reveal Claims, Private Claims, refunds). It decodes accounts with the generated clients in `clients/js` and `clients/escrow-js` and polls the RPC; it holds no keys and sends nothing.

```sh
pnpm --dir dashboard install
pnpm --dir dashboard dev --port 5174        # on this machine: devserve --port 5174 -- pnpm --dir dashboard dev --port 5174
pnpm --dir dashboard lint && pnpm --dir dashboard test && pnpm --dir dashboard build
```

Configuration is environment variables (see `.env.example`; defaults are the devnet deployment): `VITE_RPC_URL`, `VITE_RELAY_PROGRAM`, `VITE_ESCROW_PROGRAM`, `VITE_TESTNET_RELAY`, `VITE_MAINNET_RELAY`, `VITE_RELAY_POLL_MS`, `VITE_ESCROW_POLL_MS`. The public devnet RPC rate-limits, so the defaults poll politely (5 s for the relays, 10 s for Escrows and transactions) and back off on 429. `VITE_RPC_URL` is built into the page, so anyone who loads it can read it: use a public endpoint or a key restricted to this site, never a private API key.

Privacy: a Reveal Claim carries the Zcash txid in its instruction data. The dashboard reads only the first byte of each Escrow instruction (its tag) and never decodes or shows the payload, so nothing here reveals or derives what a Private Claim hides. Reveal Claims are labelled as revealing.

Helpers:
- `/fixture.html` (dev server only) renders the same page against mocked RPC answers, `?state=halted` for a Halted relay or `?state=running`.
- `pnpm demo:escrow -- --keypair <payee key file> --payout <wallet>` creates a devnet Escrow with a random commitment (never claimable) so there is something to watch; take it back with `zolar-payee refund --escrow <address> --wait`.
