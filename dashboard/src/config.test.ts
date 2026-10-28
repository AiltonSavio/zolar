import { describe, expect, it } from 'vitest';
import { loadConfig } from './config';

describe('loadConfig', () => {
  it('takes addresses and RPC URL from the environment', () => {
    const c = loadConfig({
      VITE_RPC_URL: 'http://localhost:8899',
      VITE_TESTNET_RELAY: 'T',
      VITE_MAINNET_RELAY: 'M',
      VITE_ESCROW_PROGRAM: 'E',
      VITE_RELAY_POLL_MS: '2000',
    });
    expect(c.rpcUrl).toBe('http://localhost:8899');
    expect(c.relays.map((r) => r.address)).toEqual(['T', 'M']);
    expect(c.escrowProgram).toBe('E');
    expect(c.relayPollMs).toBe(2000);
  });

  it('defaults to the devnet deployment', () => {
    const c = loadConfig({});
    expect(c.relays.map((r) => r.address)).toEqual([
      'C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS',
      '2m1YC6pGTBpoKDHL5NQgDQDTewE7bYs4m6TdxkMG4Yhs',
    ]);
    expect(c.escrowProgram).toBe('HMew6N4iWjmWMbJBoK6WfQRucBt8ZfWpMadd9mpMtbML');
  });
});
