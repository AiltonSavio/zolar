import { base58 } from '@scure/base';
import { base, request, type Fetch } from './http';

export interface VendingRelay {
  tip: number;
  finalHeight: number;
  finalityDepth: number;
  halted: boolean;
  /** Local time when this tip was first observed, not the most recent successful poll. */
  updatedAt: number;
}
export interface VendingReservation {
  nonce: Uint8Array;
  expiryHeight: number;
  createdAt: number;
  releaseRequested: boolean;
  releaseRequestedAt: number;
}
export interface VendingPayment {
  txid: string;
  height: number;
  valueZat: bigint;
  payout: string;
}
export type VendingStatus =
  | 'connect'
  | 'ready'
  | 'sold_out'
  | 'payment'
  | 'finality'
  | 'prove'
  | 'expired'
  | 'halted'
  | 'stalled';

export function vendingStatus(input: {
  connected: boolean;
  available: number;
  relay: VendingRelay;
  reservation: VendingReservation | null;
  payment: VendingPayment | null;
  now: number;
  staleMs: number;
}): VendingStatus {
  const { connected, available, relay, reservation, payment, now, staleMs } = input;
  if (relay.halted) return 'halted';
  if (reservation && (payment ? payment.height > reservation.expiryHeight : relay.tip >= reservation.expiryHeight))
    return 'expired';
  if (now - relay.updatedAt > staleMs) return 'stalled';
  if (!reservation) return available === 0 ? 'sold_out' : connected ? 'ready' : 'connect';
  if (!payment) return 'payment';
  return relay.finalHeight >= payment.height ? 'prove' : 'finality';
}

export function encodeVendingMemo(machine: Uint8Array, nonce: Uint8Array, payout: Uint8Array): Uint8Array {
  if (machine.length !== 32 || nonce.length !== 32 || payout.length !== 32)
    throw new Error('Machine, nonce and payout must each be 32 bytes');
  const memo = new Uint8Array(512);
  memo.set(new TextEncoder().encode(`zv1:${base58.encode(machine)}:${base58.encode(nonce)}:${base58.encode(payout)}`));
  return memo;
}
export function zatoshiAmount(price: bigint): string {
  if (price <= 0n || price > 0xffff_ffff_ffff_ffffn) throw new Error('Price must be a positive u64');
  const fractional = (price % 100_000_000n).toString().padStart(8, '0').replace(/0+$/, '');
  return `${price / 100_000_000n}${fractional ? `.${fractional}` : ''}`;
}
export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}
export function fromHex(value: string): Uint8Array {
  if (!/^(?:[a-fA-F0-9]{2})+$/.test(value)) throw new Error('Invalid hexadecimal bytes');
  return Uint8Array.from(value.match(/../g)!, (s) => parseInt(s, 16));
}
/** Address must already be validated as the machine's testnet, single-receiver UA. */
export function paymentLink(
  address: string,
  price: bigint,
  machine: Uint8Array,
  nonce: Uint8Array,
  payout: Uint8Array,
): string {
  if (!/^utest1[0-9a-z]+$/.test(address)) throw new Error('A testnet unified address is required');
  const memo = btoa(
    String.fromCharCode(
      ...encodeVendingMemo(machine, nonce, payout).slice(0, encodeVendingMemo(machine, nonce, payout).indexOf(0)),
    ),
  )
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '');
  return `zcash:${address}?amount=${zatoshiAmount(price)}&memo=${memo}`;
}

export interface VendingProof {
  proof: Uint8Array;
  publicValues: Uint8Array;
}
/** HTTP transport only: never receives wallet secrets or signs transactions. */
export class VendingProverClient {
  readonly url: string;
  constructor(
    url: string,
    private readonly fetchImpl: Fetch = globalThis.fetch.bind(globalThis),
    private readonly sleep: (ms: number) => Promise<void> = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  ) {
    this.url = base(url);
  }
  async findPayment(machine: string, nonce: Uint8Array): Promise<VendingPayment | null> {
    const params = new URLSearchParams({ machine, nonce: base58.encode(nonce) });
    const result = await request<{
      status: string;
      txid?: string;
      height?: number;
      value_zat?: number | string;
      payout?: string;
    }>(this.fetchImpl, `${this.url}/find-payment?${params}`);
    if (result.status === 'pending') return null;
    if (
      result.status !== 'found' ||
      !result.txid ||
      !/^[0-9a-fA-F]{64}$/.test(result.txid) ||
      !Number.isSafeInteger(result.height) ||
      result.height! <= 0 ||
      result.height! > 0xffff_ffff ||
      !result.payout ||
      base58.decode(result.payout).length !== 32 ||
      result.value_zat === undefined ||
      (typeof result.value_zat === 'number' && !Number.isSafeInteger(result.value_zat))
    )
      throw new Error('Invalid payment response');
    const valueZat = BigInt(result.value_zat);
    if (valueZat < 0n || valueZat > 0xffff_ffff_ffff_ffffn) throw new Error('Invalid payment amount');
    return { txid: result.txid, height: result.height!, valueZat, payout: result.payout };
  }
  async prove(machine: string, nonce: Uint8Array, txid: string): Promise<VendingProof> {
    const started = await request<{ job: string }>(this.fetchImpl, `${this.url}/prove`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ machine, nonce: base58.encode(nonce), txid }),
    });
    if (typeof started.job !== 'string' || !started.job) throw new Error('Invalid proof job');
    for (let attempt = 0; attempt < 360; attempt++) {
      const result = await request<{ status: string; error?: string; proof?: string; public_values?: string }>(
        this.fetchImpl,
        `${this.url}/prove/${encodeURIComponent(started.job)}`,
      );
      if (result.status === 'failed') throw new Error(result.error || 'Payment proof failed');
      if (result.status === 'done') {
        const proof = fromHex(result.proof ?? '');
        const publicValues = fromHex(result.public_values ?? '');
        if (
          publicValues.length !== 249 ||
          publicValues[0] !== 1 ||
          publicValues[188] !== 3 ||
          publicValues.slice(221).some((b) => b !== 0)
        )
          throw new Error('Invalid payment proof public values');
        return { proof, publicValues };
      }
      if (result.status !== 'proving' && result.status !== 'waiting_finality')
        throw new Error('Invalid proof job status');
      await this.sleep(5_000);
    }
    throw new Error('Proof service timed out. Retry the proof; do not pay again.');
  }
}
