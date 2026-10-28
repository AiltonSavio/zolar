import type { Config } from './config';
import { RelayCard } from './components/RelayCard';
import { EscrowTable } from './components/EscrowTable';
import { EventFeed } from './components/EventFeed';
import { useEscrows, useEvents, useMintDecimals, useNow, useRelays, useRpc } from './hooks';
import { shorten } from './format';

export function App({ config }: { config: Config }) {
  const rpc = useRpc(config);
  const relays = useRelays(rpc, config);
  const escrows = useEscrows(rpc, config);
  const events = useEvents(rpc, config);
  const now = useNow();
  const decimals = useMintDecimals(rpc, [
    ...(escrows.data ?? []).map((e) => e.mint),
    ...(events.data ?? []).flatMap((e) => (e.mint ? [e.mint] : [])),
  ]);

  const readings = relays.data;
  const haltedLabels = config.relays
    .filter((_, i) => {
      const r = readings?.[i];
      return r && 'view' in r && r.view.halt.halted;
    })
    .map((r) => r.label);

  const relayInfo = Object.fromEntries(
    config.relays.map((r, i) => {
      const reading = readings?.[i];
      return [r.address, { label: r.label, tip: reading && 'view' in reading ? reading.view.tipHeight : undefined }];
    }),
  );
  const rpcError = relays.error ?? escrows.error ?? events.error;

  return (
    <>
      {haltedLabels.length > 0 && (
        <div className="halt-bar" role="alert">
          HALT: the {haltedLabels.join(' and ')} Header Relay {haltedLabels.length > 1 ? 'are' : 'is'} halted
        </div>
      )}
      <main>
        <header className="top">
          <div>
            <h1>Zolar</h1>
            <p className="tagline">Zcash light client on Solana. Escrows that pay out when a shielded payment is final.</p>
          </div>
          <div className="live" title={new URL(config.rpcUrl).origin}>
            <span className={`pulse ${rpcError ? 'pulse-bad' : ''}`} />
            <span>{rpcError ? 'RPC trouble, retrying' : 'Live'} · Solana {config.cluster}</span>
            <span className="muted mono">{shorten(new URL(config.rpcUrl).host, 14, 6)}</span>
          </div>
        </header>

        <div className="relays">
          {config.relays.map((r, i) => {
            const reading = readings?.[i];
            return (
              <RelayCard
                key={r.address}
                label={r.label}
                address={r.address}
                view={reading && 'view' in reading ? reading.view : undefined}
                error={reading && 'error' in reading ? reading.error : relays.error}
              />
            );
          })}
        </div>

        <EscrowTable
          escrows={escrows.data}
          decimals={decimals}
          relays={relayInfo}
          nowSec={Math.floor(now / 1000)}
          cluster={config.cluster}
        />
        <EventFeed events={events.data} decimals={decimals} nowMs={now} cluster={config.cluster} />

        <footer className="foot">
          Read-only. Testnet Finality Depth is a labelled demo value, not mainnet-safe. Escrow program {shorten(config.escrowProgram, 6, 6)}, Header Relay program {shorten(config.relayProgram, 6, 6)}.
        </footer>
      </main>
    </>
  );
}
