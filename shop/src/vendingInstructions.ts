import { getWallets } from '@wallet-standard/app';
import type { WalletAccount } from '@wallet-standard/base';
import { signatureBase58, type VendingConnection } from '@zolar/sdk';
export { PublicKey, createVendingChain } from '@zolar/sdk';

export async function connectVendingWallet(): Promise<VendingConnection> {
  const wallet = getWallets()
    .get()
    .find(
      (wallet) =>
        wallet.chains.includes('solana:devnet') &&
        'standard:connect' in wallet.features &&
        'solana:signAndSendTransaction' in wallet.features,
    );
  if (!wallet) throw new Error('Install a Solana wallet with devnet transaction support, then reload.');
  const connect = wallet.features['standard:connect'] as {
    connect: () => Promise<{ accounts: readonly WalletAccount[] }>;
  };
  const { accounts } = await connect.connect();
  const account = accounts.find(
    (account) => account.chains.includes('solana:devnet') && account.features.includes('solana:signAndSendTransaction'),
  );
  if (!account) throw new Error('Choose a wallet account that supports Solana devnet.');
  return {
    address: account.address,
    async send(transaction) {
      const feature = wallet.features['solana:signAndSendTransaction'] as {
        signAndSendTransaction: (input: {
          account: WalletAccount;
          chain: string;
          transaction: Uint8Array;
        }) => Promise<readonly { signature: Uint8Array }[]>;
      };
      const [result] = await feature.signAndSendTransaction({ account, chain: 'solana:devnet', transaction });
      if (!result) throw new Error('The wallet returned no transaction signature');
      // Wallet Standard returns raw signature bytes; Solana RPC uses base58.
      return signatureBase58(result.signature);
    },
  };
}
