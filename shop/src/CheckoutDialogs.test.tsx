import { getWallets } from '@wallet-standard/app';
import type { Wallet, WalletAccount } from '@wallet-standard/base';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from './App';

vi.mock('@zolar/sdk', async (importOriginal) => {
  const sdk = await importOriginal<typeof import('@zolar/sdk')>();
  return { ...sdk, checkout: (options: Parameters<typeof sdk.checkout>[0]) => sdk.checkout({ ...options, pollMs: 5 }) };
});

const address = '8CSGksK45fuvW94GMX1Y9Pex4Hu2LoJhGhJTATi215rh';
const otherAddress = 'VausZjA4TJ8vovwd6eAFwHUQsgpX5LoFQDJEv24rfVW';
const config = { merchantUrl: 'http://shop', walletUrl: 'http://wallet', itemId: 'ga', cluster: 'devnet' as const };
const unregistrations: (() => void)[] = [];
const account = (a: string): WalletAccount => ({ address: a, publicKey: new Uint8Array(32), chains: ['solana:devnet'], features: [] });

function registerWallet(name: string, failure = false) {
  const listeners = new Set<(change: { accounts: readonly WalletAccount[] }) => void>();
  const connect = vi.fn(async () => {
    if (failure) throw new Error('Connection declined');
    return { accounts: [account(address)] };
  });
  const disconnect = vi.fn(async () => {});
  const wallet: Wallet = { version: '1.0.0', name, icon: 'data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciLz4=', chains: ['solana:devnet'], accounts: [], features: {
    'standard:connect': { version: '1.0.0', connect },
    'standard:disconnect': { version: '1.0.0', disconnect },
    'standard:events': { version: '1.0.0', on: (_event: string, listener: (change: { accounts: readonly WalletAccount[] }) => void) => { listeners.add(listener); return () => listeners.delete(listener); } },
  } };
  unregistrations.push(getWallets().register(wallet));
  return { connect, disconnect, change: (accounts: readonly WalletAccount[]) => act(() => listeners.forEach((fn) => fn({ accounts }))) };
}

function network() {
  let stage = 'awaiting_approval';
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
      if (url === 'http://wallet/resume') return Response.json(null);
    if (url === 'http://shop/items') return Response.json([{ item: { id: 'ga', name: 'General admission', mint: address, amount: 1, price_zats: 100000 }, left: 97 }]);
    if (url === 'http://wallet/status') return Response.json({ wallet: 'Zkool', network: 'testnet', relay: 'Relay', address: 'utest1external', account: 1 });
    if (url === 'http://shop/orders' && init?.method !== 'POST') throw Error('order needs POST');
    if (url === 'http://shop/orders') return Response.json({ order_id: 'order', merchant: 'Shielded Night', item: { id: 'ga', name: 'General admission', mint: address, amount: 1, price_zats: 100000 }, payout: address, network: 'testnet', zcash_address: 'utest1', memo: 'challenge', relay: 'Relay' });
    if (url === 'http://wallet/checkout') return Response.json({ job: 'job', token: 'token', approve_url: 'http://wallet/approve/secret' });
    if (url === 'http://wallet/jobs/job') return Response.json({ job: 'job', item: 'General admission', price_zats: 100000, payout: address, stage });
    throw Error('unexpected request: ' + url);
  });
  vi.stubGlobal('fetch', fetch);
  return { fetch, reject: () => { stage = 'rejected'; } };
}

beforeEach(() => {
  // jsdom does not implement the native dialog top layer. Real browser checks cover focus/inertness.
  vi.spyOn(HTMLDialogElement.prototype, 'showModal').mockImplementation(function (this: HTMLDialogElement) { this.setAttribute('open', ''); });
  vi.spyOn(HTMLDialogElement.prototype, 'close').mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute('open'); });
});
afterEach(() => { cleanup(); unregistrations.splice(0).forEach((fn) => fn()); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

async function connect(name = 'Test Wallet') {
  fireEvent.click(screen.getByRole('button', { name: /Connect wallet/i }));
  const dialog = screen.getByRole('dialog', { name: 'Connect your wallet' });
  fireEvent.click(within(dialog).getByRole('button', { name: new RegExp(name) }));
  await screen.findByRole('button', { name: new RegExp('Manage wallet ' + address) });
}

it('shows one trigger, puts installed wallets in a dialog, and tracks account changes and disconnects', async () => {
  const wallet = registerWallet('Test Wallet'); registerWallet('Another Wallet'); network(); render(<App config={config} />);
  expect(screen.queryByRole('button', { name: /Test Wallet/ })).toBeNull();
  await connect();
  expect(wallet.connect).toHaveBeenCalledOnce();
  await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Connect your wallet' })).toBeNull());
  wallet.change([account(otherAddress)]);
  const trigger = await screen.findByRole('button', { name: 'Manage wallet ' + otherAddress });
  fireEvent.click(trigger);
  fireEvent.click(screen.getByRole('button', { name: 'Disconnect wallet' }));
  await screen.findByRole('button', { name: /Connect wallet/i });
  expect(wallet.disconnect).toHaveBeenCalledOnce();
});

it('keeps connection errors in the picker and permits retry', async () => {
  registerWallet('Declining Wallet', true); network(); render(<App config={config} />);
  fireEvent.click(screen.getByRole('button', { name: /Connect wallet/i }));
  fireEvent.click(screen.getByRole('button', { name: /Declining Wallet/ }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'Connection declined');
  expect(screen.getByRole('dialog', { name: 'Connect your wallet' })).toBeTruthy();
});

it('lets someone without an extension choose a receiving address inside the picker', async () => {
  network(); render(<App config={config} />);
  fireEvent.click(screen.getByRole('button', { name: /Connect wallet/i }));
  fireEvent.click(screen.getByRole('button', { name: /Use a Solana address/ }));
  fireEvent.change(screen.getByLabelText('Solana address'), { target: { value: 'invalid' } });
  expect((screen.getByRole('button', { name: 'Use this address' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByLabelText('Solana address'), { target: { value: address } });
  fireEvent.click(screen.getByRole('button', { name: 'Use this address' }));
  expect(screen.getByRole('button', { name: 'Manage wallet ' + address })).toBeTruthy();
});

it('opens an in-page payment review first, hands off only on a click, and handles blocked popups', async () => {
  registerWallet('Test Wallet'); const requests = network(); const open = vi.fn(() => null); vi.stubGlobal('open', open);
  render(<App config={config} />); await connect(); fireEvent.click(screen.getByRole('button', { name: 'Connect Zcash wallet' })); fireEvent.click(await screen.findByRole('button', { name: /Connect Zkool/ })); await screen.findByText('Zkool connected (testnet).');
  fireEvent.click(screen.getByRole('button', { name: 'Pay with Zcash' }));
  const dialog = await screen.findByRole('dialog', { name: 'Pay with Zcash' });
  expect(open).not.toHaveBeenCalled();
  expect(within(dialog).getByText(address)).toBeTruthy();
  const handoff = await within(dialog).findByRole('button', { name: 'Authorize Zkool payment' });
  await waitFor(() => expect((handoff as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(handoff);
  expect(open).toHaveBeenCalledOnce();
  expect(await within(dialog).findByText(/Authorization window blocked/)).toBeTruthy();
  expect(within(dialog).getByRole('link', { name: 'Open payment authorization' }).getAttribute('href')).toBe('http://wallet/approve/secret');
  expect(document.querySelector('iframe')).toBeNull();
  requests.reject();
  expect(await screen.findByRole('heading', { name: 'Payment rejected' })).toBeTruthy();
  expect(requests.fetch.mock.calls.filter(([url]) => String(url) === 'http://shop/orders')).toHaveLength(1);
});


it('keeps the approved receiving address frozen if the connected account changes during checkout', async () => {
  const wallet = registerWallet('Test Wallet'); const requests = network();
  render(<App config={config} />); await connect(); fireEvent.click(screen.getByRole('button', { name: 'Connect Zcash wallet' })); fireEvent.click(await screen.findByRole('button', { name: /Connect Zkool/ })); await screen.findByText('Zkool connected (testnet).');
  fireEvent.click(screen.getByRole('button', { name: 'Pay with Zcash' }));
  const dialog = await screen.findByRole('dialog', { name: 'Pay with Zcash' });
  await within(dialog).findByText('Ready for your approval');
  wallet.change([account(otherAddress)]);
  expect(within(dialog).getByText(address)).toBeTruthy();
  expect(within(dialog).queryByText(otherAddress)).toBeNull();
  const order = requests.fetch.mock.calls.find(([url]) => String(url) === 'http://shop/orders');
  expect(JSON.parse(String(order?.[1]?.body)).payout).toBe(address);
  requests.reject();
  await screen.findByRole('heading', { name: 'Payment rejected' });
});
