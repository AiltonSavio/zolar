// @vitest-environment jsdom
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { getBase58Decoder } from '@solana/kit';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadConfig } from './config';
import { useEvents } from './hooks';
import { Rpc } from './rpc';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const config = loadConfig({ VITE_ESCROW_POLL_MS: '60000' });

const claimTx = (signature: string) => ({
  slot: 7,
  blockTime: 1_760_000_000,
  transaction: {
    signatures: [signature],
    message: {
      accountKeys: ['Payer', 'Escrow1', config.escrowProgram],
      instructions: [{ programIdIndex: 2, accounts: [1], data: getBase58Decoder().decode(Uint8Array.of(4)) }],
    },
  },
  meta: { err: null },
});

describe('useEvents', () => {
  it('shows the transactions it could fetch even when another is not available yet', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: { body: string }) => {
        const { method, params } = JSON.parse(init.body) as { method: string; params: unknown[] };
        const result =
          method === 'getSignaturesForAddress'
            ? [{ signature: 'pending', err: null }, { signature: 'landed', err: null }]
            : params[0] === 'landed'
              ? claimTx('landed')
              : null;
        return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result }));
      }),
    );
    const { result } = renderHook(() => useEvents(new Rpc(config.rpcUrl), config));
    await waitFor(() => expect(result.current.data?.map((e) => e.signature)).toEqual(['landed']));
    expect(result.current.data?.[0]?.kind).toBe('private-claim');
  });
});
