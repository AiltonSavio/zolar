import type { MerchantClient, WalletClient } from './clients';
import { ZolarError } from './http';
import { isTerminal, type JobView, type OrderTerms, type Stage } from './types';

export interface CheckoutOptions {
  merchant: MerchantClient;
  wallet: WalletClient;
  itemId: string;
  /** The customer's Solana wallet: the item goes here. */
  payout: string;
  /**
   * A window opened synchronously in the click handler (browsers block popups opened later); the
   * wallet's approval page is loaded into it. See {@link openApprovalWindow}.
   */
  approvalWindow?: Window | null;
  /** Called with the approval URL when no window could be used, so the page can show a link. */
  onApprovalUrl?: (url: string) => void;
  onOrder?: (terms: OrderTerms) => void;
  /** Called whenever the stage changes. */
  onStage?: (view: JobView) => void;
  pollMs?: number;
  signal?: AbortSignal;
  /** Read errors in a row the wallet may have before the checkout gives up (the wallet keeps going). */
  maxPollErrors?: number;
  /** For tests. */
  sleep?: (ms: number) => Promise<void>;
}

export interface CheckoutResult {
  order: OrderTerms;
  job: string;
  final: JobView;
}

/** Opens an empty popup for the wallet's approval page. Call it directly in the click handler. */
export function openApprovalWindow(): Window | null {
  if (typeof window === 'undefined') return null;
  return window.open('about:blank', 'zolar-approve', 'popup,width=460,height=760');
}

const defaultSleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * Runs a checkout: creates the order at the merchant, asks the customer's wallet to pay it, shows the
 * wallet's approval page, then follows the wallet's stages until the item is claimed, the customer
 * rejects, or the checkout fails. The wallet does the payment, the Escrow check and the proof; this
 * page never sees the Zcash transaction ID or the salt.
 */
export async function checkout(o: CheckoutOptions): Promise<CheckoutResult> {
  const sleep = o.sleep ?? defaultSleep;
  const pollMs = o.pollMs ?? 2_000;
  const maxErrors = o.maxPollErrors ?? 15;

  const order = await o.merchant.createOrder(o.itemId, o.payout);
  o.onOrder?.(order);
  let started;
  try {
    started = await o.wallet.startCheckout(o.merchant.url, order.order_id);
  } catch (e) {
    o.approvalWindow?.close();
    throw e;
  }
  if (o.approvalWindow && !o.approvalWindow.closed) {
    o.approvalWindow.location.href = started.approve_url;
  } else {
    o.onApprovalUrl?.(started.approve_url);
  }

  let lastKey = '';
  let errors = 0;
  for (;;) {
    if (o.signal?.aborted) throw new ZolarError('checkout stopped watching (the wallet keeps going)', null);
    let view: JobView;
    try {
      view = await o.wallet.job(started.job, started.token);
      errors = 0;
    } catch (e) {
      errors += 1;
      if (errors > maxErrors) throw e;
      await sleep(pollMs);
      continue;
    }
    const key = JSON.stringify(view);
    if (key !== lastKey) {
      lastKey = key;
      o.onStage?.(view);
    }
    if (isTerminal(view)) return { order, job: started.job, final: view };
    await sleep(pollMs);
  }
}

/** The steps a checkout page shows, in order. */
export const STEPS = [
  { key: 'approve', label: 'Approve in Zolar Wallet' },
  { key: 'lock', label: 'Ticket locked on Solana' },
  { key: 'pay', label: 'Payment sent on Zcash' },
  { key: 'final', label: 'Zcash block final on Solana' },
  { key: 'prove', label: 'Proof made on your computer' },
  { key: 'claim', label: 'Ticket released to your wallet' },
] as const;

export type StepKey = (typeof STEPS)[number]['key'];

/** Index into {@link STEPS} of the step in progress (STEPS.length when done). */
export function stepIndex(stage: Stage): number {
  switch (stage.stage) {
    case 'awaiting_approval':
    case 'rejected':
      return 0;
    case 'building_payment':
    case 'locking_item':
    case 'checking_escrow':
      return 1;
    case 'paying':
      return 2;
    case 'waiting_for_finality':
      return 3;
    case 'proving':
      return 4;
    case 'claimed':
      return 5;
    case 'done':
      return STEPS.length;
    case 'failed':
      return -1;
  }
}

/**
 * How far the payment is towards finality while the wallet waits: blocks on top of the first block it
 * can be in, out of the Finality Depth. Null before the wallet has read the relay.
 */
export function finalityProgress(stage: Stage): { done: number; of: number; tip: number } | null {
  if (stage.stage !== 'waiting_for_finality' || stage.tip === 0 || stage.sent_tip === 0) return null;
  const of = stage.finality_depth;
  const done = Math.min(of, Math.max(0, stage.tip - (stage.sent_tip + 1)));
  return { done, of, tip: stage.tip };
}
