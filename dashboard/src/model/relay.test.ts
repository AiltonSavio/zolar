import { describe, expect, it } from 'vitest';
import { toRelayView } from './relay';

import { relayAccount } from './fixtures';

describe('toRelayView', () => {
  it('reads tip, work, finality depth and tree size from account bytes', () => {
    const v = toRelayView(relayAccount());
    expect(v.tipHeight).toBe(4_517_300);
    expect(v.chainWork).toBe(123_456_789_012_345_678_901n);
    expect(v.finalityDepth).toBe(10);
    expect(v.treeSize).toBe(199);
    expect(v.network).toBe('testnet');
    expect(v.halt).toEqual({ halted: false });
  });

  it('shows the tip hash in explorer order (reversed internal bytes)', () => {
    const v = toRelayView(relayAccount());
    expect(v.tipHash.startsWith('1f1e1d1c')).toBe(true);
    expect(v.tipHash.endsWith('03020100')).toBe(true);
  });

  it('latest root is roots[leafCount % 128]', () => {
    const v = toRelayView(relayAccount()); // 199 % 128 = 71 -> fill 72 = 0x48
    expect(v.latestRoot).toBe('48'.repeat(32));
  });

  it('reports a deep-reorg Halt with its height and hash', () => {
    const v = toRelayView(
      relayAccount({ halted: 1, haltHeight: 4_517_333, haltHash: Uint8Array.from({ length: 32 }, (_, i) => i) }),
    );
    expect(v.halt).toEqual({
      halted: true,
      reason: 'Deep reorg: the heaviest chain reorganised past a final block',
      haltHeight: 4_517_333,
      haltHash: expect.stringMatching(/^1f1e/),
    });
  });

  it('reports a full Final Block Tree as a Halt', () => {
    const v = toRelayView(relayAccount({ halted: 2 }));
    expect(v.halt.halted && v.halt.reason).toMatch(/tree is full/i);
  });

  it('treats a relay that is not sealed as pending', () => {
    expect(toRelayView(relayAccount({ state: 0 })).sealed).toBe(false);
  });
});
