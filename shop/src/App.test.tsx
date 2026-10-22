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
      if (url === 'http://shop/items')
        return new Response(
          JSON.stringify([{ item: { id: 'ga', name: 'General admission', mint: 'M', amount: 1, price_zats: 100000 }, left: 42 }]),
        );
      if (url === 'http://wallet/status' && walletUp)
        return new Response(JSON.stringify({ wallet: 'zolar-wallet', network: 'testnet', relay: 'R' }));
      throw new TypeError('connection refused');
    }),
  );
}

describe('shop', () => {
  it('shows the price and says when no Zolar Wallet runs', async () => {
    stubFetch(false);
    render(<App config={config} />);
    expect(await screen.findByText('0.001 ZEC (testnet)')).toBeTruthy();
    expect(await screen.findByText(/Zolar Wallet is not running/)).toBeTruthy();
    expect((screen.getByRole('button', { name: /Pay 0.001 ZEC/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('reads the stock again when a checkout ends', async () => {
    let itemReads = 0;
    vi.stubGlobal('open', vi.fn(() => null));
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === 'http://shop/items') {
          itemReads += 1;
          return new Response(
            JSON.stringify([{ item: { id: 'ga', name: 'General admission', mint: 'M', amount: 1, price_zats: 100000 }, left: 42 - itemReads }]),
          );
        }
        if (url === 'http://wallet/status') return new Response(JSON.stringify({ wallet: 'zolar-wallet', network: 'testnet', relay: 'R' }));
        if (url === 'http://shop/orders') return new Response(JSON.stringify({ error: 'sold out' }), { status: 400 });
        throw new TypeError('connection refused');
      }),
    );
    render(<App config={config} />);
    await screen.findByText(/Zolar Wallet found/);
    fireEvent.change(screen.getByLabelText(/Solana address/), { target: { value: '8CSGksK45fuvW94GMX1Y9Pex4Hu2LoJhGhJTATi215rh' } });
    fireEvent.click(screen.getByRole('button', { name: /Pay 0.001 ZEC/ }));
    expect(await screen.findByText('sold out')).toBeTruthy();
    await waitFor(() => expect(itemReads).toBe(2));
    expect(await screen.findByText('40 left')).toBeTruthy();
  });

  it('finds a running wallet', async () => {
    stubFetch(true);
    render(<App config={config} />);
    expect(await screen.findByText(/Zolar Wallet found \(testnet\)/)).toBeTruthy();
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
    expect(screen.getByText('FSpj…VYDf')).toBeTruthy();
  });

  it('states the privacy check result honestly', () => {
    const done = { ...base, stage: 'done' as const, escrow: 'E', claim: 'C' };
    render(<Success cluster="devnet" event="Shielded Night" view={{ ...done, txid_found_on_solana: false }} />);
    expect(screen.getByText(/appears in none/)).toBeTruthy();
    cleanup();
    render(<Success cluster="devnet" event="Shielded Night" view={{ ...done, txid_found_on_solana: null }} />);
    expect(screen.getByText(/did not run/)).toBeTruthy();
  });
});
