import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { VendingApp } from './VendingApp';
import type { VendingChain, VendingConnection, VendingProver, VendingSnapshot } from './vendingModel';

const address = '11111111111111111111111111111111';
const config = {
  merchantUrl: '',
  walletUrl: '',
  itemId: 'ga',
  cluster: 'devnet' as const,
  mode: 'vending' as const,
  vending: {
    address: 'utest1abc',
    machine: address,
    rpcUrl: 'http://rpc',
    proverUrl: 'http://prover',
    program: address,
    abiUrl: '/abi.json',
  },
};
const connection: VendingConnection = { address, send: vi.fn() };
function setup(
  options: {
    available?: number;
    halted?: boolean;
    stalled?: boolean;
    expired?: boolean;
    payment?: boolean;
    proofFail?: boolean;
    releaseRequestedAt?: number;
  } = {},
) {
  const snapshot: VendingSnapshot = {
    machine: address,
    priceZat: 100_000n,
    available: options.available ?? 42,
    stock: 100,
    relay: {
      tip: options.expired ? 150 : 110,
      finalHeight: options.payment ? 110 : 100,
      finalityDepth: 10,
      halted: options.halted ?? false,
      updatedAt: options.stalled ? 0 : Date.now(),
    },
  };
  const chain: VendingChain = {
    transactionStatus: vi.fn(async () => 'confirmed' as const),
    restore: vi.fn(async () => null),
    read: vi.fn(async () => snapshot),
    reserve: vi.fn(async (_wallet, nonce) => ({
      nonce,
      address,
      payout: address,
      priceZat: 100_000n,
      expiryHeight: 120,
      createdAt: Date.now() / 1000,
      releaseRequested: options.releaseRequestedAt !== undefined,
      releaseRequestedAt: options.releaseRequestedAt ?? 0,
    })),
    requestRelease: vi.fn(async (_wallet, session) => ({
      ...session,
      releaseRequested: true,
      releaseRequestedAt: Math.floor(Date.now() / 1000),
    })),
    release: vi.fn(async () => {}),
    claim: vi.fn(async () => 'claim-signature'),
  };
  const prover: VendingProver = {
    findPayment: vi.fn(async () =>
      options.payment ? { txid: 'a'.repeat(64), height: 105, valueZat: 100_000n, payout: address } : null,
    ),
    prove: vi.fn(async () => {
      if (options.proofFail) throw new Error('Proof unavailable');
      return { proof: new Uint8Array(1), publicValues: new Uint8Array(249) };
    }),
  };
  render(<VendingApp config={config} chain={chain} prover={prover} connect={async () => connection} pollMs={20} />);
  return { chain, prover, snapshot };
}
afterEach(() => {
  cleanup();
  localStorage.clear();
});
async function reserve() {
  fireEvent.click(await screen.findByRole('button', { name: 'Connect Solana wallet' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Reserve ticket' }));
}

describe('vending checkout', () => {
  it('connects before reserving and shows a link and QR for the exact reserved amount', async () => {
    const { chain } = setup();
    await reserve();
    const link = await screen.findByRole('link', { name: 'Open in wallet' });
    expect(link.getAttribute('href')).toMatch(/^zcash:utest1abc\?amount=0.001&memo=enYx/);
    expect(await screen.findByAltText('Zcash payment QR code')).toBeTruthy();
    expect(chain.reserve).toHaveBeenCalledOnce();
    expect(screen.getByText(/Wallet fees are extra/)).toBeTruthy();
  });
  it('shows sold out without enabling reservation', async () => {
    setup({ available: 0 });
    expect(await screen.findByText('Sold out')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Reserve ticket' })).toBeNull();
  });
  it.each(['halted', 'stalled'] as const)('disables payment when the relay is %s', async (state) => {
    setup({ [state]: true });
    expect(
      await screen.findByText(
        state === 'halted' ? 'The relay is halted. Do not pay.' : 'The relay has stopped advancing. Do not pay.',
      ),
    ).toBeTruthy();
    expect(screen.queryByRole('link', { name: 'Open in wallet' })).toBeNull();
    expect((screen.getByRole('button', { name: 'Pay with Zcash' }) as HTMLButtonElement).disabled).toBe(true);
  });
  it.each([undefined, 100, 1000])(
    'hides payment controls and notes on expired reservations (release age %s)',
    async (age) => {
      setup({ expired: true, releaseRequestedAt: age === undefined ? undefined : Math.floor(Date.now() / 1000) - age });
      await reserve();
      expect(await screen.findByText('Reservation expired')).toBeTruthy();
      expect(screen.queryByRole('button', { name: 'Pay with Zcash' })).toBeNull();
      expect(screen.queryByRole('link', { name: 'Open in wallet' })).toBeNull();
      expect(screen.queryByText(/Wallet fees are extra/)).toBeNull();
      expect(screen.queryByText(/Ironwood-capable/)).toBeNull();
      expect(screen.queryByText(/Pay by Zcash height/)).toBeNull();
      expect(screen.getByRole('button', { name: age === undefined ? 'Request release' : 'Release' })).toBeTruthy();
    },
  );
  it('requests release first and shows the Claim window countdown', async () => {
    const { chain } = setup({ expired: true });
    await reserve();
    expect(await screen.findByText('Reservation expired')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Release' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Request release' }));
    expect(await screen.findByText(/Release in 15:00/)).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Release' }) as HTMLButtonElement).disabled).toBe(true);
    expect(chain.requestRelease).toHaveBeenCalledOnce();
    expect(chain.release).not.toHaveBeenCalled();
    expect(screen.getByText(/A Claim can still mint your ticket/)).toBeTruthy();
  });
  it('enables Release at the end of the Claim window', async () => {
    const { chain } = setup({ expired: true, releaseRequestedAt: Math.floor(Date.now() / 1000) - 900 });
    await reserve();
    fireEvent.click(await screen.findByRole('button', { name: 'Release' }));
    await waitFor(() => expect(chain.release).toHaveBeenCalledOnce());
  });
  it('still Claims a payment while release is requested', async () => {
    const { chain } = setup({ payment: true, releaseRequestedAt: Math.floor(Date.now() / 1000) - 100 });
    await reserve();
    expect(await screen.findByText('You’re in.')).toBeTruthy();
    expect(chain.claim).toHaveBeenCalledOnce();
    expect(chain.release).not.toHaveBeenCalled();
  });
  it('keeps a failed proof retryable without asking for a second payment', async () => {
    const { prover } = setup({ payment: true, proofFail: true });
    await reserve();
    expect(await screen.findByText('Proof failed')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Retry proof' }));
    await waitFor(() => expect(prover.prove).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole('link', { name: 'Open in wallet' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Pay with Zcash' })).toBeNull();
    expect(screen.queryByText(/Wallet fees are extra/)).toBeNull();
    expect(screen.queryByText(/Ironwood-capable/)).toBeNull();
    expect(screen.queryByText(/Pay by Zcash height/)).toBeNull();
  });
  it('submits Claim and reuses the success screen without an Escrow privacy claim', async () => {
    const { chain } = setup({ payment: true });
    await reserve();
    expect(await screen.findByText('You’re in.')).toBeTruthy();
    expect(chain.claim).toHaveBeenCalledOnce();
    expect(screen.getByText('Ticket minted to your wallet.')).toBeTruthy();
    expect(screen.queryByText(/transaction ID appears in none/)).toBeNull();
  });
});

describe('payment recovery', () => {
  it('does not offer another payment when a detected payment temporarily disappears from the scanner', async () => {
    const { prover } = setup({ payment: false });
    vi.mocked(prover.findPayment).mockResolvedValueOnce({
      txid: 'a'.repeat(64),
      height: 105,
      valueZat: 100_000n,
      payout: address,
    });
    await reserve();
    expect(await screen.findByText('Waiting for finality')).toBeTruthy();
    await new Promise((resolve) => setTimeout(resolve, 80));
    expect(screen.queryByRole('link', { name: 'Open in wallet' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Pay with Zcash' })).toBeNull();
    expect(screen.queryByText(/Wallet fees are extra/)).toBeNull();
  });
  it.each(['halted', 'stalled', 'expired'] as const)(
    'hides payment controls after a detected payment when %s',
    async (failure) => {
      const { snapshot, prover } = setup();
      await reserve();
      await screen.findByRole('link', { name: 'Open in wallet' });
      vi.mocked(prover.findPayment).mockResolvedValue({
        txid: 'a'.repeat(64),
        height: failure === 'expired' ? 121 : 105,
        valueZat: 100_000n,
        payout: address,
      });
      if (failure === 'halted') snapshot.relay.halted = true;
      if (failure === 'stalled') snapshot.relay.updatedAt = 0;
      await waitFor(() =>
        expect(localStorage.getItem(`zolar-vending:${address}:${address}:payment`)).toContain('valueZat'),
      );
      expect(screen.queryByRole('button', { name: 'Pay with Zcash' })).toBeNull();
      expect(screen.queryByText(/Wallet fees are extra/)).toBeNull();
      expect(screen.queryByText(/Ironwood-capable/)).toBeNull();
      expect(screen.queryByText(/Pay by Zcash height/)).toBeNull();
    },
  );
  it('refreshes a release requested by another signer and restores its countdown', async () => {
    const { chain } = setup({ expired: true });
    await reserve();
    await screen.findByRole('button', { name: 'Request release' });
    const session = await vi.mocked(chain.reserve).mock.results[0]!.value;
    vi.mocked(chain.restore).mockResolvedValue({
      ...session,
      releaseRequested: true,
      releaseRequestedAt: Math.floor(Date.now() / 1000) - 60,
    });
    expect(await screen.findByText(/Release in 14:/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Request release' })).toBeNull();
  });
  it('persists the nonce before submitting Reserve, retaining it when confirmation fails', async () => {
    const { chain } = setup();
    vi.mocked(chain.reserve).mockImplementation(async () => {
      expect(localStorage.getItem(`zolar-vending:${address}:${address}`)).toContain('nonce');
      throw new Error('Confirmation unavailable');
    });
    await reserve();
    expect(await screen.findByText('Confirmation unavailable')).toBeTruthy();
    expect(localStorage.getItem(`zolar-vending:${address}:${address}`)).toContain('nonce');
    expect(screen.getByRole('button', { name: 'Check reservation' })).toBeTruthy();
  });
});

describe('copy request', () => {
  it('shows a copy error only after a failed copy and clears it on success', async () => {
    const writeText = vi.fn().mockRejectedValueOnce(new Error('denied')).mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    setup();
    await reserve();
    const button = await screen.findByRole('button', { name: 'Copy payment link' });
    expect(screen.queryByText(/Could not copy/)).toBeNull();
    fireEvent.click(button);
    expect(await screen.findByText(/Could not copy/)).toBeTruthy();
    fireEvent.click(button);
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
    expect(screen.queryByText(/Could not copy/)).toBeNull();
    Reflect.deleteProperty(navigator, 'clipboard');
  });
  it('copies the same link that opens the wallet', async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    setup();
    await reserve();
    const link = await screen.findByRole('link', { name: 'Open in wallet' });
    expect(screen.queryByText(/Could not copy/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Copy payment link' }));
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
    expect(writeText).toHaveBeenCalledWith(link.getAttribute('href'));
    Reflect.deleteProperty(navigator, 'clipboard');
  });
});

describe('saved Claim reconciliation', () => {
  it('preserves a pending signature and prevents another Reserve after reload', async () => {
    const key = `zolar-vending:${address}:${address}`;
    localStorage.setItem(key, JSON.stringify({ nonce: 'ab'.repeat(32), payout: address, claim: 'pending-signature' }));
    const { chain } = setup();
    vi.mocked(chain.transactionStatus).mockResolvedValue('pending');
    fireEvent.click(await screen.findByRole('button', { name: 'Connect Solana wallet' }));
    expect(((await screen.findByRole('button', { name: 'Reserve ticket' })) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: 'Check reservation' }));
    expect(await screen.findByText('Claim status is not confirmed yet. Keep this reservation saved.')).toBeTruthy();
    expect(localStorage.getItem(key)).toContain('pending-signature');
    expect(chain.reserve).not.toHaveBeenCalled();
  });
  it('retains the nonce and unlocks re-reservation only after a definite failed Claim', async () => {
    const key = `zolar-vending:${address}:${address}`;
    localStorage.setItem(key, JSON.stringify({ nonce: 'ab'.repeat(32), payout: address, claim: 'failed-signature' }));
    const { chain } = setup();
    vi.mocked(chain.transactionStatus).mockResolvedValue('failed');
    fireEvent.click(await screen.findByRole('button', { name: 'Connect Solana wallet' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Check reservation' }));
    expect(await screen.findByText('Claim failed. Reserve again with the saved nonce to retry.')).toBeTruthy();
    expect(JSON.parse(localStorage.getItem(key)!)).toEqual({ nonce: 'ab'.repeat(32), payout: address });
    expect((screen.getByRole('button', { name: 'Reserve ticket' }) as HTMLButtonElement).disabled).toBe(false);
  });
});
