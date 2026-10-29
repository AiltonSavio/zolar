import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { Progress } from './components/Progress';
import { Success } from './components/Success';

const config = { merchantUrl: 'http://shop', walletUrl: 'http://wallet', itemId: 'ga', cluster: 'devnet' as const };

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function stubFetch(walletUp: boolean) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === 'http://wallet/resume') return Response.json(null);
      if (url === 'http://shop/items')
        return new Response(
          JSON.stringify([{ item: { id: 'ga', name: 'General admission', mint: 'M', amount: 1, price_zats: 100000 }, left: 42 }]),
        );
      if (url === 'http://wallet/status' && walletUp)
        return new Response(JSON.stringify({ wallet: 'Zkool', network: 'testnet', relay: 'R', address: 'utest1external', account: 1 }));
      throw new TypeError('connection refused');
    }),
  );
}

async function connectZcash() {
  fireEvent.click(screen.getByRole('button', { name: 'Connect Zcash wallet' }));
  fireEvent.click(await screen.findByRole('button', { name: /Connect Zkool/ }));
  await screen.findByText('Zkool connected (testnet).');
}

describe('shop', () => {
  it('requires an existing Zcash wallet before payment', async () => {
    stubFetch(false);
    render(<App config={config} />);
    expect((await screen.findAllByText('0.001 ZEC')).length).toBe(2);
    expect(await screen.findByText(/Connect an existing Zcash testnet wallet/)).toBeTruthy();
    expect((screen.getByRole('button', { name: /Pay with Zcash/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('reads the stock again when a checkout ends', async () => {
    let itemReads = 0;
    vi.stubGlobal('open', vi.fn(() => null));
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
      if (url === 'http://wallet/resume') return Response.json(null);
        if (url === 'http://shop/items') {
          itemReads += 1;
          return new Response(
            JSON.stringify([{ item: { id: 'ga', name: 'General admission', mint: 'M', amount: 1, price_zats: 100000 }, left: 42 - itemReads }]),
          );
        }
        if (url === 'http://wallet/status') return new Response(JSON.stringify({ wallet: 'Zkool', network: 'testnet', relay: 'R', address: 'utest1external', account: 1 }));
        if (url === 'http://shop/orders') return new Response(JSON.stringify({ error: 'sold out' }), { status: 400 });
        throw new TypeError('connection refused');
      }),
    );
    render(<App config={config} />);
    await connectZcash();
    fireEvent.click(screen.getByRole('button', { name: 'Connect wallet' }));
    fireEvent.click(screen.getByRole('button', { name: 'Use a Solana address' }));
    fireEvent.change(screen.getByLabelText('Solana address'), { target: { value: '8CSGksK45fuvW94GMX1Y9Pex4Hu2LoJhGhJTATi215rh' } });
    fireEvent.click(screen.getByRole('button', { name: 'Use this address' }));
    fireEvent.click(screen.getByRole('button', { name: /Pay with Zcash/ }));
    expect(await screen.findByText('sold out')).toBeTruthy();
    await waitFor(() => expect(itemReads).toBe(2));
    expect(await screen.findByText('40 of 100 left')).toBeTruthy();
  });

  it('finds a running wallet', async () => {
    stubFetch(true);
    render(<App config={config} />);
    await connectZcash();
    expect(await screen.findByText('Zkool connected (testnet).')).toBeTruthy();
  });
});

describe('stages', () => {
  const base = { job: 'j', item: 'General admission', price_zats: 100000, payout: '8CSGksK45fuvW94GMX1Y9Pex4Hu2LoJhGhJTATi215rh' };

  it('counts blocks while waiting for finality', () => {
    render(
      <Progress
        cluster="devnet"
        view={{ ...base, stage: 'waiting_for_finality', escrow: 'FSpjvGFbd6QQBJh6apUyE9AqmCrEJYagKZigdn4VYDf', tip: 107, final_height: 97, expiry: 120, sent_tip: 100, finality_depth: 10 }}
      />,
    );
    expect(screen.getByText(/6 of 10 blocks on top/)).toBeTruthy();
    expect(screen.getByText('Escrow FSpj…VYDf')).toBeTruthy();
  });

  it('states the privacy check result honestly', () => {
    const done = { ...base, stage: 'done' as const, escrow: 'E', claim: 'C' };
    render(<Success cluster="devnet" view={{ ...done, txid_found_on_solana: false }} />);
    expect(screen.getByText(/appears in none/)).toBeTruthy();
    cleanup();
    render(<Success cluster="devnet" view={{ ...done, txid_found_on_solana: null }} />);
    expect(screen.getByText(/did not run/)).toBeTruthy();
  });

  it('keeps a recovering checkout open without claiming a payment step is complete', () => {
    render(<Progress cluster="devnet" view={{ ...base, stage:'recovering' }} />);
    expect(screen.getByRole('status').textContent).toContain('restoring the saved checkout');
    expect(screen.getByText('Reconnecting to your checkout')).toBeTruthy();
    expect(screen.queryByRole('list')).toBeNull();
  });
});

it('restores a paid receipt with its frozen item price and receiving address using read requests only', async () => {
  sessionStorage.clear();
  const calls:{url:string;method:string}[]=[];
  vi.stubGlobal('fetch',vi.fn(async(input:RequestInfo|URL,init?:RequestInit)=>{
    const url=String(input);calls.push({url,method:init?.method??'GET'});
    if(url==='http://shop/items')return Response.json([{item:{id:'ga',name:'Changed catalog item',mint:'M',amount:1,price_zats:9900000},left:40}]);
    if(url==='http://wallet/status')return Response.json({wallet:'Zkool',network:'testnet',relay:'R',address:'utest1external',account:1});
    if(url==='http://wallet/resume')return Response.json({job:'restored',token:'read-only'});
    if(url==='http://wallet/jobs/restored')return Response.json({job:'restored',item:'Original paid ticket',price_zats:100000,
      payout:'VausZjA4TJ8vovwd6eAFwHUQsgpX5LoFQDJEv24rfVW',stage:'done',escrow:'E',claim:'C',txid_found_on_solana:false});
    throw Error('unexpected request '+url);
  }));
  render(<App config={config}/>);
  expect(await screen.findByText('Ticket in wallet')).toBeTruthy();
  expect(screen.getByText('Original paid ticket')).toBeTruthy();
  expect(screen.queryByText('Changed catalog item')).toBeNull();
  expect(screen.getByText('Vaus…rfVW')).toBeTruthy();
  expect(calls.every(c=>c.method==='GET')).toBe(true);
  fireEvent.click(screen.getByRole('button',{name:'Buy another ticket'}));
  expect(await screen.findByText('Get your ticket')).toBeTruthy();
  expect(sessionStorage.getItem('zolar-dismissed:http://wallet')).toBe('restored');
});
