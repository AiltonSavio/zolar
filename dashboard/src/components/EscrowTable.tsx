import type { EscrowView } from '../model/escrow';
import { formatTokenAmount } from '../model/escrow';
import { shorten } from '../format';

interface Props {
  escrows: EscrowView[] | undefined;
  decimals: Record<string, number>;
  /** Relay address -> its label and current tip height. */
  relays: Record<string, { label: string; tip: number | undefined }>;
  nowSec: number;
  cluster: string;
}

function expiryText(e: EscrowView, tip: number | undefined) {
  if (tip === undefined) return null;
  const left = e.expiryHeight - tip;
  return left > 0 ? `${left} blocks left` : 'past expiry';
}

export function EscrowTable({ escrows, decimals, relays, nowSec, cluster }: Props) {
  return (
    <section className="card" aria-label="Open Escrows">
      <header className="card-head">
        <div>
          <p className="eyebrow">Escrow program</p>
          <h2>Open Escrows</h2>
        </div>
        <span className="count mono">{escrows?.length ?? '–'}</span>
      </header>
      {escrows && escrows.length === 0 && <p className="empty">No open Escrows. A created Escrow shows up here within seconds.</p>}
      {escrows && escrows.length > 0 && (
        <div className="table-wrap">
          <table>
            <thead>
              <tr><th>Escrow</th><th>Token</th><th className="num">Amount</th><th>State</th><th>Expiry (Zcash height)</th><th>Bond</th><th>Claim mode</th></tr>
            </thead>
            <tbody>
              {escrows.map((e) => {
                const d = decimals[e.mint];
                const relay = relays[e.relay];
                const left = expiryText(e, relay?.tip);
                return (
                  <tr key={e.address} className={e.state === 'refund-requested' ? 'row-refund' : ''}>
                    <td className="mono"><a href={`https://explorer.solana.com/address/${e.address}?cluster=${cluster}`} target="_blank" rel="noreferrer">{shorten(e.address)}</a></td>
                    <td className="mono" title={e.mint}>{shorten(e.mint)}</td>
                    <td className="mono num">{d === undefined ? `${e.amount} raw` : formatTokenAmount(e.amount, d)}</td>
                    <td>
                      {e.state === 'open'
                        ? <span className="chip chip-open">Open</span>
                        : <span className="chip chip-refund" title="A Claim still settles until the Refund closes the Escrow">
                            Refund requested{e.refundRequestedAt ? ` · ${Math.max(0, nowSec - e.refundRequestedAt)}s` : ''}
                          </span>}
                    </td>
                    <td className="mono">
                      {e.expiryHeight.toLocaleString('en-US')}
                      <span className="sub">{relay ? `${relay.label}${left ? ` · ${left}` : ''}` : 'unknown relay'}</span>
                    </td>
                    <td>
                      {e.bond.kind === 'none' && <span className="muted">none</span>}
                      {e.bond.kind === 'required' && <span className="chip chip-warn">required, not deposited</span>}
                      {e.bond.kind === 'deposited' && <span className="chip chip-open">deposited</span>}
                    </td>
                    <td>
                      {e.claimMode === 'private-only'
                        ? <span className="chip chip-private">Private only</span>
                        : <span className="chip chip-reveal" title="Either claim type may settle">Reveal or Private</span>}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
