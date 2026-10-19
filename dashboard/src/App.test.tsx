// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { getBase64Decoder, type ReadonlyUint8Array } from '@solana/kit';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { loadConfig } from './config';
import { relayAccount } from './model/fixtures';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function stubRpc(halted: number) {
  const config = loadConfig({});
  const acct = (data: ReadonlyUint8Array) => ({
    data: [getBase64Decoder().decode(Uint8Array.from(data)), 'base64'],
    owner: config.relayProgram,
  });
  vi.stubGlobal(
    'fetch',
    vi.fn(async (_url: string, init: { body: string }) => {
      const { method } = JSON.parse(init.body) as { method: string };
      const result =
        method === 'getMultipleAccounts'
          ? { value: [acct(relayAccount()), acct(relayAccount({ halted, haltHeight: 4_517_333 }))] }
          : [];
      return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result }));
    }),
  );
  return config;
}

describe('App', () => {
  it('raises a page-wide HALT bar when either relay is halted', async () => {
    render(<App config={stubRpc(1)} />);
    await waitFor(() => expect(screen.getByText(/the Zcash mainnet Header Relay is halted/)).toBeTruthy());
    expect(screen.getAllByRole('alert').length).toBe(2);
  });

  it('shows no alert while both relays run', async () => {
    render(<App config={stubRpc(0)} />);
    await waitFor(() => expect(screen.getAllByTestId('tip-height').length).toBe(2));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
