import { getWallets } from '@wallet-standard/app';
import type { Wallet, WalletAccount } from '@wallet-standard/base';
import { useEffect, useState } from 'react';

// The shop only needs the customer's address: the ticket is sent there, nothing is signed.
type ConnectFeature = { connect: (input?: { silent?: boolean }) => Promise<{ accounts: readonly WalletAccount[] }> };

export function isSolanaWallet(w: Wallet): boolean {
  return 'standard:connect' in w.features && w.chains.some((c) => c.startsWith('solana:'));
}

export function useSolanaWallets(): Wallet[] {
  const [wallets, setWallets] = useState<Wallet[]>(() => getWallets().get().filter(isSolanaWallet));
  useEffect(() => {
    const api = getWallets();
    const refresh = () => setWallets(api.get().filter(isSolanaWallet));
    const offRegister = api.on('register', refresh);
    const offUnregister = api.on('unregister', refresh);
    return () => {
      offRegister();
      offUnregister();
    };
  }, []);
  return wallets;
}

export async function connectAddress(wallet: Wallet): Promise<string> {
  const feature = wallet.features['standard:connect'] as ConnectFeature | undefined;
  if (!feature) throw new Error(`${wallet.name} cannot connect`);
  const { accounts } = await feature.connect();
  const account = accounts.find((a) => a.chains.some((c) => c.startsWith('solana:')) && looksLikeAddress(a.address));
  if (!account) throw new Error(`${wallet.name} shared no Solana account`);
  return account.address;
}

/** A base58 Solana address, loosely: 32 to 44 base58 characters. */
export function looksLikeAddress(s: string): boolean {
  return /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(s.trim());
}

type ChangeFeature = { on: (event: 'change', listener: (change: { accounts?: readonly WalletAccount[] }) => void) => () => void };
type DisconnectFeature = { disconnect: () => Promise<void> };
export type WalletSelection = { address: string; wallet?: Wallet };

/** Connection state belongs to the page, so the receiving wallet stays selected across checkout. */
export function useSolanaConnection() {
  const wallets = useSolanaWallets();
  const [selection, setSelection] = useState<WalletSelection | null>(null);
  const [connecting, setConnecting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const wallet = selection?.wallet;
    if (!wallet) return;
    const events = wallet.features['standard:events'] as ChangeFeature | undefined;
    const offChange = events?.on('change', ({ accounts }) => {
      if (!accounts) return;
      const available = accounts.filter((a) => a.chains.some((c) => c.startsWith('solana:')) && looksLikeAddress(a.address));
      const account = available.find((a) => a.address === selection.address) ?? available[0];
      setSelection(account ? { address: account.address, wallet } : null);
    });
    const offUnregister = getWallets().on('unregister', (...removed) => {
      if (removed.includes(wallet)) setSelection(null);
    });
    return () => { offChange?.(); offUnregister(); };
  }, [selection?.wallet, selection?.address]);
  async function connect(wallet: Wallet) {
    setConnecting(wallet.name); setError(null);
    try {
      const address = await connectAddress(wallet);
      if (!getWallets().get().includes(wallet)) throw new Error('This wallet is no longer available. Choose another wallet.');
      setSelection({ wallet, address });
      return true;
    } catch (e) { setError((e as Error).message); return false; }
    finally { setConnecting(null); }
  }
  async function disconnect() {
    setError(null);
    try {
      const feature = selection?.wallet?.features['standard:disconnect'] as DisconnectFeature | undefined;
      await feature?.disconnect(); setSelection(null); return true;
    } catch (e) { setError((e as Error).message); return false; }
  }
  function useAddress(address: string) {
    if (!looksLikeAddress(address)) return false;
    setError(null); setSelection({ address: address.trim() }); return true;
  }
  return { wallets, selection, connecting, error, clearError: () => setError(null), connect, disconnect, useAddress };
}
