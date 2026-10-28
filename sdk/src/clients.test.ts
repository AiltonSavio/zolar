import { describe, expect, it } from 'vitest';
import { WalletClient } from './clients';

describe('WalletClient', () => {
  it('reports no wallet when nothing answers or the site is not allowed', async () => {
    const down = (async () => {
      throw new TypeError('connection refused');
    }) as typeof fetch;
    expect(await new WalletClient(undefined, down).status()).toBeNull();
    const refused = (async () => new Response(JSON.stringify({ error: 'not found' }), { status: 404 })) as typeof fetch;
    expect(await new WalletClient(undefined, refused).status()).toBeNull();
    const up = (async () =>
      new Response(JSON.stringify({ wallet: 'zolar-wallet', network: 'testnet', relay: 'R' }))) as typeof fetch;
    expect(await new WalletClient(undefined, up).status()).toEqual({ wallet: 'zolar-wallet', network: 'testnet', relay: 'R' });
  });
});
