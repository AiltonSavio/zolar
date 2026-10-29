import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ZcashWalletControl, supportedZcashWallet } from './ZcashWalletControl';

afterEach(cleanup);

it('accepts only an identified external testnet wallet and never substitutes the embedded helper', () => {
  for (const status of [null, undefined, { wallet: 'zolar-wallet', network: 'testnet' as const, relay: 'R' },
    { wallet: 'Zkool', network: 'mainnet' as const, relay: 'R', address: 'u1mainnet', account: 1 },
    { wallet: 'Zkool', network: 'testnet' as const, relay: 'R', address: 'utest1account', account: 0 }]) {
    expect(supportedZcashWallet(status)).toBe(false);
  }
  expect(supportedZcashWallet({ wallet: 'Zkool', network: 'testnet', relay: 'R', address: 'utest1account', account: 1 })).toBe(true);
});

it('keeps connection separate from authorizing a payment', async () => {
  const connect = vi.fn(async () => {});
  render(<ZcashWalletControl status={{ wallet: 'Zkool', network: 'testnet', relay: 'R', address: 'utest1account', account: 1 }}
    selection={null} onConnect={connect} onDisconnect={() => {}} />);
  expect(connect).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Connect Zcash wallet' }));
  expect(screen.getByText(/spending keys stay in Zkool/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: /Connect Zkool/ }));
  expect(connect).toHaveBeenCalledOnce();
});
