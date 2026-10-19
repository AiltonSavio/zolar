export interface RelayConfig {
  label: string;
  address: string;
}

export interface Config {
  rpcUrl: string;
  cluster: string;
  relayProgram: string;
  escrowProgram: string;
  relays: RelayConfig[];
  relayPollMs: number;
  escrowPollMs: number;
}

type Env = Record<string, string | undefined>;

export function loadConfig(env: Env = import.meta.env as Env): Config {
  const num = (v: string | undefined, d: number) => (v && Number(v) > 0 ? Number(v) : d);
  return {
    rpcUrl: env.VITE_RPC_URL || 'https://api.devnet.solana.com',
    cluster: env.VITE_CLUSTER || 'devnet',
    relayProgram: env.VITE_RELAY_PROGRAM || '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D',
    escrowProgram: env.VITE_ESCROW_PROGRAM || 'HMew6N4iWjmWMbJBoK6WfQRucBt8ZfWpMadd9mpMtbML',
    relays: [
      { label: 'Zcash testnet', address: env.VITE_TESTNET_RELAY || 'C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS' },
      { label: 'Zcash mainnet', address: env.VITE_MAINNET_RELAY || '2m1YC6pGTBpoKDHL5NQgDQDTewE7bYs4m6TdxkMG4Yhs' },
    ],
    relayPollMs: num(env.VITE_RELAY_POLL_MS, 5000),
    escrowPollMs: num(env.VITE_ESCROW_POLL_MS, 10000),
  };
}
