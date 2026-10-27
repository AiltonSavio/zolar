export interface VendingConfig {
  machine: string;
  program: string;
  address: string;
  rpcUrl: string;
  proverUrl: string;
  abiUrl: string;
}
export interface ShopConfig {
  mode?: 'checkout' | 'vending';
  vending?: VendingConfig;
  merchantUrl: string;
  walletUrl: string;
  itemId: string;
  cluster: 'devnet' | 'mainnet-beta';
}

export function readConfig(env: Record<string, string | undefined> = import.meta.env): ShopConfig {
  return {
    mode: env.VITE_CHECKOUT_MODE === 'vending' ? 'vending' : 'checkout',
    vending:
      env.VITE_CHECKOUT_MODE === 'vending'
        ? {
            abiUrl: env.VITE_VENDING_ABI_URL || '/vending-abi.json',
            machine: env.VITE_VENDING_MACHINE || '',
            program: env.VITE_VENDING_PROGRAM || 'J9paKuXLpRr4rmWVp8vBZ6ZMwQhrTSk6w3Rmyv5JHFW5',
            address: env.VITE_VENDING_ADDRESS || '',
            rpcUrl: env.VITE_VENDING_RPC || 'https://api.devnet.solana.com',
            proverUrl: env.VITE_PROVER_URL || 'http://127.0.0.1:8080',
          }
        : undefined,
    merchantUrl: env.VITE_MERCHANT_URL || 'http://127.0.0.1:7979',
    walletUrl: env.VITE_WALLET_URL || 'http://127.0.0.1:7878',
    itemId: env.VITE_ITEM_ID || 'ga',
    cluster: env.VITE_CLUSTER === 'mainnet-beta' ? 'mainnet-beta' : 'devnet',
  };
}

export function explorer(cluster: ShopConfig['cluster'], kind: 'tx' | 'address', id: string): string {
  const q = cluster === 'devnet' ? '?cluster=devnet' : '';
  return `https://explorer.solana.com/${kind}/${id}${q}`;
}

export function zec(zats: number): string {
  return (zats / 1e8).toLocaleString('en-US', { maximumFractionDigits: 8 });
}

export function short(address: string, head = 4, tail = 4): string {
  return address.length <= head + tail + 1 ? address : `${address.slice(0, head)}…${address.slice(-tail)}`;
}
