import { describe, expect, it } from 'vitest';
import { address } from '@solana/kit';
import { getEscrowEncoder } from '@escrow-client';
import { formatTokenAmount, toEscrowView } from './escrow';

const A = address('7zr1j185zT8r6jcysC4LyrdSEJ13Up3xq4TFu6KQqh7M');
const MINT = address('6hcTJNrnzSfL3QNj3uQRsFuC9WHAD48ejMov7Nw7ZyqN');
const RELAY = address('C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS');

function escrowBytes(over: Record<string, unknown> = {}) {
  return getEscrowEncoder().encode({
    version: 1, state: 0, bump: 255, payee: A, payout: A, mint: MINT, vault: A, relay: RELAY,
    commitment: new Uint8Array(32).fill(9), amount: 2_500_000n, bondRequired: 0n, bondDeposited: 0n,
    createdAt: 1_760_000_000n, createdTip: 4_517_100, expiryHeight: 4_517_118, claimMode: 0,
    refundRequestedAt: 0n, reserved: new Uint8Array(55), ...over,
  });
}

describe('toEscrowView', () => {
  it('reads token, amount, expiry and relay', () => {
    const v = toEscrowView('EscrowAddr', escrowBytes());
    expect(v).toMatchObject({
      address: 'EscrowAddr', mint: MINT, amount: 2_500_000n, expiryHeight: 4_517_118, relay: RELAY,
      state: 'open', claimMode: 'reveal-or-private', bond: { kind: 'none' },
    });
  });

  it('labels the claim mode and refund state', () => {
    const v = toEscrowView('E', escrowBytes({ claimMode: 1, state: 1, refundRequestedAt: 1_760_000_500n }));
    expect(v.claimMode).toBe('private-only');
    expect(v.state).toBe('refund-requested');
    expect(v.refundRequestedAt).toBe(1_760_000_500);
  });

  it('tells a required Bond from a deposited one', () => {
    expect(toEscrowView('E', escrowBytes({ bondRequired: 100n })).bond).toEqual({ kind: 'required', amount: 100n });
    expect(toEscrowView('E', escrowBytes({ bondRequired: 100n, bondDeposited: 100n })).bond).toEqual({
      kind: 'deposited', amount: 100n,
    });
  });
});

describe('formatTokenAmount', () => {
  it.each([
    [2_500_000n, 6, '2.5'],
    [1n, 6, '0.000001'],
    [1_000_000_000n, 6, '1,000'],
    [42n, 0, '42'],
  ])('%s with %s decimals is %s', (raw, decimals, expected) => {
    expect(formatTokenAmount(raw, decimals)).toBe(expected);
  });
});
