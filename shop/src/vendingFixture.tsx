import { createRoot } from 'react-dom/client';
import { createVendingChain, VendingProverClient } from '@zolar/sdk';
import { fixtureAddress, fixtureMachine, fixtureProgram, mockVendingTransport } from '../../sdk/src/fixtures/vending';
import { VendingApp } from './VendingApp';
import './styles.css';

if (import.meta.env.DEV) {
  const state = new URLSearchParams(location.search).get('state') || 'payment';
  localStorage.clear();
  const mock = mockVendingTransport(state);
  const settings = {
    machine: fixtureMachine,
    program: fixtureProgram,
    address: fixtureAddress,
    rpcUrl: 'http://fixture.invalid/rpc',
    abiUrl: 'http://fixture.invalid/abi.json',
    proverUrl: 'http://fixture.invalid',
  };
  const chain = createVendingChain(settings, mock.fetchImpl);
  createRoot(document.getElementById('root')!).render(
    <VendingApp
      config={{ mode: 'vending', cluster: 'devnet', itemId: 'ga', merchantUrl: '', walletUrl: '', vending: settings }}
      chain={chain}
      prover={
        new VendingProverClient(
          settings.proverUrl,
          mock.fetchImpl,
          () => new Promise((resolve) => setTimeout(resolve, 1_000)),
        )
      }
      connect={async () => mock.wallet}
      pollMs={250}
    />,
  );
}
