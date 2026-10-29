import { base58 } from '@scure/base';
import { describe, expect, it } from 'vitest';
import { encodeVendingMemo, paymentLink, zatoshiAmount, vendingStatus, VendingProverClient } from './vending.js';

const machine = Uint8Array.from({ length: 32 }, (_, i) => i);
const payout = new Uint8Array(32).fill(42);
const nonce = Uint8Array.from({ length: 32 }, (_, i) => 255 - i);
const relay = { tip: 110, finalHeight: 100, finalityDepth: 10, halted: false, updatedAt: 1_000 };
const reservation = { releaseRequested: false, releaseRequestedAt: 0, nonce, expiryHeight: 120, createdAt: 0 };

describe('vending wire values', () => {
  it('encodes canonical zv1 text and zero pads the exact ZIP-302 bytes', () => {
    const bytes = encodeVendingMemo(machine, nonce, payout);
    const text = new TextEncoder().encode(
      `zv1:${base58.encode(machine)}:${base58.encode(nonce)}:${base58.encode(payout)}`,
    );
    expect(bytes).toEqual(Uint8Array.from([...text, ...new Uint8Array(512 - text.length)]));
    expect(bytes.length).toBe(512);
    expect(() => encodeVendingMemo(new Uint8Array(31), nonce, payout)).toThrow();
    expect(() => encodeVendingMemo(machine, new Uint8Array(33), payout)).toThrow();
  });
  it('uses integer arithmetic for exact prices, including the u64 range', () => {
    expect(zatoshiAmount(1n)).toBe('0.00000001');
    expect(zatoshiAmount(50_000_000n)).toBe('0.5');
    expect(zatoshiAmount(18_446_744_073_709_551_615n)).toBe('184467440737.09551615');
    expect(() => zatoshiAmount(0n)).toThrow();
    expect(() => zatoshiAmount(-1n)).toThrow();
  });
  it('builds a single-recipient ZIP-321 link with unpadded base64url of all memo bytes', () => {
    const link = paymentLink('utest1abc', 100_001n, machine, nonce, payout);
    const [address, query] = link.split('?');
    expect(address).toBe('zcash:utest1abc');
    const params = new URLSearchParams(query);
    expect(params.get('amount')).toBe('0.00100001');
    const memo = params.get('memo')!;
    expect(memo).not.toMatch(/[+/=]/);
    expect(memo.length).toBeGreaterThan(128);
    expect(Uint8Array.from(atob(memo.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0))).toEqual(
      encodeVendingMemo(machine, nonce, payout).slice(0, encodeVendingMemo(machine, nonce, payout).indexOf(0)),
    );
    expect(() => paymentLink('u1mainnet', 1n, machine, nonce, payout)).toThrow();
  });
});

describe('vending state transitions', () => {
  const input = { available: 1, relay, reservation: null, payment: null, now: 1_001, staleMs: 100 };
  it('connects, reserves, waits for payment, finalises, then proves', () => {
    expect(vendingStatus({ ...input, connected: false })).toBe('connect');
    expect(vendingStatus({ ...input, connected: true })).toBe('ready');
    expect(vendingStatus({ ...input, connected: true, reservation })).toBe('payment');
    expect(
      vendingStatus({
        ...input,
        connected: true,
        reservation,
        payment: { txid: 'a', height: 105, valueZat: 1n, payout: '' },
      }),
    ).toBe('finality');
    expect(
      vendingStatus({
        ...input,
        connected: true,
        reservation,
        payment: { txid: 'a', height: 100, valueZat: 1n, payout: '' },
      }),
    ).toBe('prove');
  });
  it('keeps reserved stock available to its owner when the machine sells out', () => {
    expect(vendingStatus({ ...input, connected: true, available: 0 })).toBe('sold_out');
    expect(vendingStatus({ ...input, connected: true, available: 0, reservation })).toBe('payment');
  });
  it('disables payment when the relay halts, stalls, or a reservation expires', () => {
    expect(vendingStatus({ ...input, connected: true, reservation, relay: { ...relay, halted: true } })).toBe('halted');
    expect(vendingStatus({ ...input, connected: true, reservation, now: 1_101 })).toBe('stalled');
    expect(vendingStatus({ ...input, connected: true, reservation, relay: { ...relay, tip: 121 } })).toBe('expired');
    expect(vendingStatus({ ...input, connected: true, reservation, relay: { ...relay, tip: 119 } })).toBe('payment');
  });
  it('allows an on-time payment to finalise after the payment deadline', () => {
    expect(
      vendingStatus({
        ...input,
        connected: true,
        reservation,
        payment: { txid: 'a', height: 119, valueZat: 1n, payout: '' },
        relay: { ...relay, tip: 130, finalHeight: 120 },
      }),
    ).toBe('prove');
    expect(
      vendingStatus({
        ...input,
        connected: true,
        reservation,
        payment: { txid: 'a', height: 121, valueZat: 1n, payout: '' },
      }),
    ).toBe('expired');
  });
});

describe('stateless prover client', () => {
  it('polls by machine and base58 nonce and accepts a not-yet-seen payment', async () => {
    let seen = '';
    const client = new VendingProverClient('https://prover/', async (input) => {
      seen = String(input);
      return new Response('{"status":"pending"}');
    });
    expect(await client.findPayment('machine', nonce)).toBeNull();
    expect(seen).toBe(`https://prover/find-payment?machine=machine&nonce=${base58.encode(nonce)}`);
  });
});

describe('revision 2 proof jobs', () => {
  it('polls a job through waiting/proving and decodes the 249-byte payout-bearing result', async () => {
    const values = new Uint8Array(249);
    values[0] = 1;
    values[188] = 3;
    values.set(payout, 189);
    let polls = 0;
    const calls: string[] = [];
    const client = new VendingProverClient(
      'http://prover',
      async (input, init) => {
        calls.push(String(input));
        if (init?.method === 'POST') {
          expect(JSON.parse(String(init.body))).toEqual({
            machine: 'M',
            nonce: base58.encode(nonce),
            txid: 'a'.repeat(64),
          });
          return new Response('{"job":"job/1"}');
        }
        polls++;
        return new Response(
          JSON.stringify(
            polls < 3
              ? { status: polls === 1 ? 'waiting_finality' : 'proving' }
              : {
                  status: 'done',
                  proof: '01',
                  public_values: Array.from(values, (b) => b.toString(16).padStart(2, '0')).join(''),
                },
          ),
        );
      },
      async () => {},
    );
    expect((await client.prove('M', nonce, 'a'.repeat(64))).publicValues).toEqual(values);
    expect(calls.slice(1)).toEqual(new Array(3).fill('http://prover/prove/job%2F1'));
  });
  it('throws a failed job so the shop can offer proof retry', async () => {
    const client = new VendingProverClient(
      'http://prover',
      async (_input, init) =>
        new Response(init?.method === 'POST' ? '{"job":"j"}' : '{"status":"failed","error":"busy"}'),
    );
    await expect(client.prove('M', nonce, 'a'.repeat(64))).rejects.toThrow('busy');
  });
  it('rejects malformed JSON instead of treating it as a pending payment', async () => {
    const client = new VendingProverClient('http://prover', async () => new Response('not json'));
    await expect(client.findPayment('M', nonce)).rejects.toThrow();
  });
});
