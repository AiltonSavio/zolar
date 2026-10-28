import { base, type Fetch, request } from './http';
import type { CatalogEntry, CheckoutStarted, JobView, OrderTerms, OrderView, WalletStatus } from './types';

/** The shop's merchant service (zolar-merchant). */
export class MerchantClient {
  readonly url: string;
  constructor(
    url: string,
    private readonly fetchImpl: Fetch = globalThis.fetch.bind(globalThis),
  ) {
    this.url = base(url);
  }

  items(): Promise<CatalogEntry[]> {
    return request(this.fetchImpl, `${this.url}/items`);
  }

  /** Creates an order delivering `itemId` to the Solana wallet `payout`. */
  createOrder(itemId: string, payout: string): Promise<OrderTerms> {
    return request(this.fetchImpl, `${this.url}/orders`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ item_id: itemId, payout }),
    });
  }

  order(orderId: string): Promise<OrderView> {
    return request(this.fetchImpl, `${this.url}/orders/${encodeURIComponent(orderId)}`);
  }
}

export const DEFAULT_WALLET_URL = 'http://127.0.0.1:7878';

/** The customer's Zolar wallet on their own machine (zolar-wallet). */
export class WalletClient {
  readonly url: string;
  constructor(
    url: string = DEFAULT_WALLET_URL,
    private readonly fetchImpl: Fetch = globalThis.fetch.bind(globalThis),
  ) {
    this.url = base(url);
  }

  /** The wallet's network and relay, or null if no wallet answers (not running, or this site not allowed). */
  async status(): Promise<WalletStatus | null> {
    try {
      return await request<WalletStatus>(this.fetchImpl, `${this.url}/status`, { timeoutMs: 3_000 });
    } catch {
      return null;
    }
  }

  /** Asks the wallet to pay an order. Nothing is paid until the customer approves at `approve_url`. */
  startCheckout(merchantUrl: string, orderId: string): Promise<CheckoutStarted> {
    return request(this.fetchImpl, `${this.url}/checkout`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ merchant_url: base(merchantUrl), order_id: orderId }),
    });
  }

  job(job: string, token: string): Promise<JobView> {
    return request(this.fetchImpl, `${this.url}/jobs/${encodeURIComponent(job)}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
  }
}
