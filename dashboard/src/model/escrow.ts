import type { ReadonlyUint8Array } from '@solana/kit';
import { getEscrowDecoder } from '@escrow-client';

export type EscrowState = 'open' | 'refund-requested';
export type ClaimMode = 'reveal-or-private' | 'private-only';
export type BondView =
  | { kind: 'none' }
  | { kind: 'required'; amount: bigint }
  | { kind: 'deposited'; amount: bigint };

export interface EscrowView {
  address: string;
  state: EscrowState;
  mint: string;
  amount: bigint;
  expiryHeight: number;
  createdAt: number;
  refundRequestedAt: number;
  bond: BondView;
  claimMode: ClaimMode;
  relay: string;
}

export function toEscrowView(address: string, data: ReadonlyUint8Array): EscrowView {
  const e = getEscrowDecoder().decode(data);
  const bond: BondView =
    e.bondRequired === 0n
      ? { kind: 'none' }
      : e.bondDeposited >= e.bondRequired
        ? { kind: 'deposited', amount: e.bondDeposited }
        : { kind: 'required', amount: e.bondRequired };
  return {
    address,
    state: e.state === 1 ? 'refund-requested' : 'open',
    mint: e.mint,
    amount: e.amount,
    expiryHeight: e.expiryHeight,
    createdAt: Number(e.createdAt),
    refundRequestedAt: Number(e.refundRequestedAt),
    bond,
    claimMode: e.claimMode === 1 ? 'private-only' : 'reveal-or-private',
    relay: e.relay,
  };
}

export function formatTokenAmount(raw: bigint, decimals: number): string {
  const base = 10n ** BigInt(decimals);
  const whole = (raw / base).toLocaleString('en-US');
  const frac = (raw % base).toString().padStart(decimals, '0').replace(/0+$/, '');
  return frac ? `${whole}.${frac}` : whole;
}
