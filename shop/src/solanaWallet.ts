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
  const account = accounts.find((a) => a.chains.some((c) => c.startsWith('solana:'))) ?? accounts[0];
  if (!account) throw new Error(`${wallet.name} shared no account`);
  return account.address;
}

/** A base58 Solana address, loosely: 32 to 44 base58 characters. */
export function looksLikeAddress(s: string): boolean {
  return /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(s.trim());
}
