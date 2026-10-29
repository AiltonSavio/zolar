import { describe, expect, it } from 'vitest';
import { checkout, watchCheckout, finalityProgress, stepIndex, STEPS } from './checkout.js';
import { MerchantClient, WalletClient } from './clients.js';
import type { JobView, OrderTerms, Stage } from './types.js';

const terms: OrderTerms = {
  order_id: 'o1',
  merchant: 'Shielded Night',
  item: { id: 'ga', name: 'General admission', mint: 'Mint', amount: 1, price_zats: 100_000 },
  zcash_address: 'utest1',
  memo: 'zolar order o1 ab',
  payout: 'Payout',
  relay: 'Relay',
  network: 'testnet',
};

function view(stage: Stage): JobView {
  return { job: 'j1', item: 'General admission', price_zats: 100_000, payout: 'Payout', ...stage } as JobView;
}

/** A fake merchant and wallet: records requests, plays the given stages one per poll. */
function fakes(stages: (Stage | Error)[]) {
  const calls: { url: string; init: RequestInit | undefined }[] = [];
  let poll = 0;
  const fetchImpl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, init });
    const json = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
    if (url === 'http://shop/orders') return json(201, terms);
    if (url === 'http://wallet/checkout')
      return json(201, { job: 'j1', token: 'tok', approve_url: 'http://wallet/approve/s' });
    if (url === 'http://wallet/jobs/j1') {
      const s = stages[Math.min(poll++, stages.length - 1)]!;
      if (s instanceof Error) return json(500, { error: s.message });
      return json(200, view(s));
    }
    return json(404, { error: 'not found' });
  }) as typeof fetch;
  return {
    calls,
    merchant: new MerchantClient('http://shop/', fetchImpl),
    wallet: new WalletClient('http://wallet', fetchImpl),
  };
}

const noSleep = async () => {};

describe('checkout', () => {
  it('binds checkout to the explicitly selected external Zcash account without sending credentials', async () => {
    const f = fakes([{ stage: 'rejected' }]);
    await checkout({ merchant: f.merchant, wallet: f.wallet, itemId: 'ga', payout: 'P', sleep: noSleep,
      zcashWallet: { wallet: 'Zkool', network: 'testnet', relay: 'Relay', address: 'utest1selected', account: 7 } });
    const start = f.calls.find((c) => c.url === 'http://wallet/checkout')!;
    expect(JSON.parse(String(start.init?.body))).toEqual({ merchant_url: 'http://shop', order_id: 'o1',
      zcash_address: 'utest1selected', zcash_account: 7 });
    expect(f.calls.find((c) => c.url === 'http://shop/orders')!.init?.body).not.toContain('utest1selected');
  });

  it('creates the order, hands the wallet the merchant and order, and follows stages to done', async () => {
    const done: Stage = { stage: 'done', escrow: 'E', claim: 'C', txid_found_on_solana: false };
    const f = fakes([
      { stage: 'awaiting_approval' },
      { stage: 'awaiting_approval' },
      { stage: 'paying', escrow: 'E' },
      done,
    ]);
    const seen: string[] = [];
    let approval = '';
    const r = await checkout({
      merchant: f.merchant,
      wallet: f.wallet,
      itemId: 'ga',
      payout: 'Payout',
      onApprovalUrl: (u) => (approval = u),
      onStage: (v) => seen.push(v.stage),
      sleep: noSleep,
    });
    expect(r.final.stage).toBe('done');
    expect(r.order).toEqual(terms);
    expect(seen).toEqual(['awaiting_approval', 'paying', 'done']);
    expect(approval).toBe('http://wallet/approve/s');
    const start = f.calls.find((c) => c.url === 'http://wallet/checkout')!;
    expect(JSON.parse(String(start.init?.body))).toEqual({ merchant_url: 'http://shop', order_id: 'o1' });
    const order = f.calls.find((c) => c.url === 'http://shop/orders')!;
    expect(JSON.parse(String(order.init?.body))).toEqual({ item_id: 'ga', payout: 'Payout' });
    const poll = f.calls.find((c) => c.url === 'http://wallet/jobs/j1')!;
    expect(new Headers(poll.init?.headers).get('Authorization')).toBe('Bearer tok');
  });

  it('ends on a rejection or a failure', async () => {
    const f = fakes([{ stage: 'rejected' }]);
    const r = await checkout({ merchant: f.merchant, wallet: f.wallet, itemId: 'ga', payout: 'P', sleep: noSleep });
    expect(r.final.stage).toBe('rejected');
    const g = fakes([{ stage: 'failed', reason: 'the order lapsed' }]);
    const s = await checkout({ merchant: g.merchant, wallet: g.wallet, itemId: 'ga', payout: 'P', sleep: noSleep });
    expect(s.final).toMatchObject({ stage: 'failed', reason: 'the order lapsed' });
  });

  it('rides out a few failed reads of the wallet, then gives up', async () => {
    const f = fakes([new Error('busy'), new Error('busy'), { stage: 'done', escrow: 'E', claim: 'C', txid_found_on_solana: null }]);
    const r = await checkout({ merchant: f.merchant, wallet: f.wallet, itemId: 'ga', payout: 'P', sleep: noSleep });
    expect(r.final.stage).toBe('done');
    const g = fakes([new Error('gone')]);
    await expect(
      checkout({ merchant: g.merchant, wallet: g.wallet, itemId: 'ga', payout: 'P', sleep: noSleep, maxPollErrors: 2 }),
    ).rejects.toThrow('gone');
  });

  it('surfaces the merchant refusing an order', async () => {
    const fetchImpl = (async () =>
      new Response(JSON.stringify({ error: 'this wallet already has a checkout in progress' }), { status: 400 })) as typeof fetch;
    await expect(
      checkout({
        merchant: new MerchantClient('http://shop', fetchImpl),
        wallet: new WalletClient('http://wallet', fetchImpl),
        itemId: 'ga',
        payout: 'P',
        sleep: noSleep,
      }),
    ).rejects.toThrow('already has a checkout');
  });
});

describe('readonly recovery', () => {
  it('keeps the same paid job through a long outage without creating or approving a replacement', async () => {
    const f=fakes([{stage:'paying',escrow:'E'},...Array.from({length:30},()=>new Error('connector offline')),
      {stage:'done',escrow:'E',claim:'C',txid_found_on_solana:false}]);
    const seen:JobView[]=[];
    const final=await watchCheckout({wallet:f.wallet,job:'j1',token:'tok',maxPollErrors:Infinity,sleep:noSleep,onStage:v=>seen.push(v)});
    expect(final.stage).toBe('done');expect(seen.map(v=>v.stage)).toEqual(['paying','recovering','done']);
    expect(seen[1]).toMatchObject({job:'j1',payout:'Payout',price_zats:100000,item:'General admission'});
    expect(f.calls.every(c=>c.url==='http://wallet/jobs/j1'&&!c.init?.method)).toBe(true);
  });
});

describe('progress', () => {
  it('maps every stage to a step', () => {
    expect(stepIndex({ stage: 'awaiting_approval' })).toBe(0);
    expect(stepIndex({ stage: 'checking_escrow', escrow: 'E' })).toBe(1);
    expect(stepIndex({ stage: 'paying', escrow: 'E' })).toBe(2);
    expect(stepIndex({ stage: 'proving', escrow: 'E' })).toBe(4);
    expect(stepIndex({ stage: 'done', escrow: 'E', claim: 'C', txid_found_on_solana: false })).toBe(STEPS.length);
    expect(stepIndex({ stage: 'failed', reason: 'x' })).toBe(-1);
  });

  it('counts blocks towards finality from the tip when the payment went out', () => {
    const w = (tip: number): Stage => ({
      stage: 'waiting_for_finality', escrow: 'E', tip, final_height: tip - 10, expiry: 120, sent_tip: 100, finality_depth: 10,
    });
    expect(finalityProgress(w(0))).toBeNull();
    expect(finalityProgress(w(100))).toEqual({ done: 0, of: 10, tip: 100 });
    expect(finalityProgress(w(107))).toEqual({ done: 6, of: 10, tip: 107 });
    expect(finalityProgress(w(130))).toEqual({ done: 10, of: 10, tip: 130 });
    expect(finalityProgress({ stage: 'paying', escrow: 'E' })).toBeNull();
  });

  it('keeps watching a recovering checkout without pretending a step is complete', async () => {
    const recovering = { stage: 'recovering' } as Stage;
    expect(stepIndex(recovering)).toBe(-1);
    const f = fakes([recovering, { stage: 'done', escrow: 'E', claim: 'C', txid_found_on_solana: false }]);
    const seen: string[] = [];
    const result = await checkout({ merchant: f.merchant, wallet: f.wallet, itemId: 'ga', payout: 'P',
      sleep: noSleep, onStage: (v) => seen.push(v.stage) });
    expect(result.final.stage).toBe('done');
    expect(seen).toEqual(['recovering', 'done']);
  });
});
