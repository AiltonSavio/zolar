// The checkout protocol of zolar-merchant and zolar-wallet (cli/src/checkout.rs). Keep in step.

/** Something the merchant sells. `amount` is in the mint's smallest unit. */
export interface Item {
  id: string;
  name: string;
  mint: string;
  amount: number;
  price_zats: number;
}

export interface CatalogEntry {
  item: Item;
  /** Items left in the merchant's account, if it could be read. */
  left: number | null;
}

/** An order as the merchant offers it: everything the customer approves in the wallet. */
export interface OrderTerms {
  order_id: string;
  merchant: string;
  item: Item;
  /** The merchant's Zcash unified address. */
  zcash_address: string;
  /** The challenge the payment's memo carries; new for every order. */
  memo: string;
  /** The Solana wallet the item goes to. */
  payout: string;
  /** The Header Relay the Escrow trusts. */
  relay: string;
  network: 'testnet' | 'mainnet';
}

export type OrderStatus = 'open' | 'escrowed' | 'claimed' | 'refunded' | 'expired';

export interface OrderView {
  order_id: string;
  status: OrderStatus;
  escrow: string | null;
}

export interface CheckoutStarted {
  job: string;
  /** Shows the wallet that a status request comes from the page that started the job. */
  token: string;
  /** The wallet's own approval page; open it for the customer. */
  approve_url: string;
}

/** Where a checkout stands in the customer's wallet. */
export type Stage =
  | { stage: 'awaiting_approval' }
  | { stage: 'rejected' }
  | { stage: 'building_payment' }
  | { stage: 'locking_item' }
  | { stage: 'checking_escrow'; escrow: string }
  | { stage: 'paying'; escrow: string }
  | {
      stage: 'waiting_for_finality';
      escrow: string;
      /** Relay tip and final height now (0 until first read). */
      tip: number;
      final_height: number;
      /** The payment must be mined by this Zcash height. */
      expiry: number;
      /** Relay tip when the payment went out: it is mined after it and final `finality_depth` blocks later. */
      sent_tip: number;
      finality_depth: number;
    }
  | { stage: 'proving'; escrow: string }
  | { stage: 'claimed'; escrow: string; claim: string }
  | {
      stage: 'done';
      escrow: string;
      claim: string;
      /** Whether the wallet's search found the Zcash transaction ID on Solana; null if not run. */
      txid_found_on_solana: boolean | null;
    }
  | { stage: 'failed'; reason: string };

export type StageName = Stage['stage'];

export type JobView = Stage & {
  job: string;
  item: string;
  price_zats: number;
  payout: string;
};

export interface WalletStatus {
  wallet: string;
  network: 'testnet' | 'mainnet';
  relay: string;
}

export const TERMINAL_STAGES: readonly StageName[] = ['rejected', 'done', 'failed'];

export function isTerminal(stage: Stage): boolean {
  return TERMINAL_STAGES.includes(stage.stage);
}
