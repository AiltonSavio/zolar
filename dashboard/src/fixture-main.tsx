// Dev-only page (/fixture.html): the real App against mocked RPC answers, to see states the live
// relays must never be in. The data is fake and the page says so.
import { getBase64Decoder, type ReadonlyUint8Array } from '@solana/kit';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { loadConfig } from './config';
import { relayAccount } from './model/fixtures';
import './styles.css';

const config = loadConfig({});
const state = new URLSearchParams(location.search).get('state') ?? 'halted';
const b64 = getBase64Decoder();
const acct = (data: ReadonlyUint8Array) => ({ data: [b64.decode(Uint8Array.from(data)), 'base64'], owner: config.relayProgram });

const running = relayAccount({ tipHeight: 4_517_994, leafCount: 893, finalizedHeight: 4_517_984 });
const halted = relayAccount({
  tipHeight: 4_517_994,
  leafCount: 893,
  finalizedHeight: 4_517_984,
  halted: 1,
  haltHeight: 4_517_994,
  haltHash: Uint8Array.from({ length: 32 }, (_, i) => 255 - i),
});

window.fetch = async (_url, init) => {
  const { method } = JSON.parse(String(init?.body)) as { method: string };
  const result =
    method === 'getMultipleAccounts'
      ? { value: [acct(state === 'halted' ? halted : running), acct(running)] }
      : [];
  return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result }));
};

createRoot(document.getElementById('root')!).render(
  <>
    <p style={{ margin: 0, padding: 6, textAlign: 'center', background: '#333', color: '#fff', fontSize: 12 }}>
      FIXTURE DATA, not the live relays
    </p>
    <App config={config} />
  </>,
);
