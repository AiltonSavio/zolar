import { describe, expect, it } from 'vitest';
import { WalletClient } from './clients.js';

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

it('discovers an existing checkout with GET only and accepts a valid empty result', async () => {
  const calls:string[]=[];
  const wallet=new WalletClient('http://wallet',(async(input,init)=>{
    calls.push(String(input));expect(init?.method).toBeUndefined();
    return Response.json(calls.length===1?null:{job:'saved',token:'read-only'});
  }) as typeof fetch);
  expect(await wallet.resume()).toBeNull();
  expect(await wallet.resume()).toEqual({job:'saved',token:'read-only'});
  expect(calls).toEqual(['http://wallet/resume','http://wallet/resume']);
});

it('rejects null or unrelated job status while allowing an empty resume response', async () => {
  for(const body of [null,{job:'different',stage:'done'}]){
    const wallet=new WalletClient('http://wallet',(async()=>Response.json(body)) as typeof fetch);
    await expect(wallet.job('original','read-token')).rejects.toThrow('invalid checkout status');
  }
});
