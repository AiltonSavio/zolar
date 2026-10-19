import type { EscrowEvent, EventKind } from '../model/claims';
import { formatTokenAmount } from '../model/escrow';
import { ago, shorten } from '../format';

const LABEL: Record<EventKind, { text: string; cls: string; note: string }> = {
  'reveal-claim': { text: 'Reveal Claim', cls: 'chip-reveal', note: 'reveals the Zcash txid on Solana' },
  'private-claim': { text: 'Private Claim', cls: 'chip-private', note: 'txid stays hidden' },
  refund: { text: 'Refund', cls: 'chip-refund', note: 'Escrow closed, tokens back to the Payee' },
  'refund-requested': { text: 'Refund requested', cls: 'chip-warn', note: 'Claim window open' },
  created: { text: 'Escrow created', cls: 'chip-open', note: '' },
  'bond-deposited': { text: 'Bond deposited', cls: 'chip-open', note: '' },
};

interface Props {
  events: EscrowEvent[] | undefined;
  decimals: Record<string, number>;
  nowMs: number;
  cluster: string;
}

export function EventFeed({ events, decimals, nowMs, cluster }: Props) {
  return (
    <section className="card" aria-label="Claims and refunds">
      <header className="card-head">
        <div>
          <p className="eyebrow">Escrow program activity</p>
          <h2>Claims and refunds</h2>
        </div>
      </header>
      {!events && <p className="empty">Loading recent transactions…</p>}
      {events && events.length === 0 && <p className="empty">No Escrow transactions yet.</p>}
      <ul className="feed">
        {events?.map((ev) => {
          const l = LABEL[ev.kind];
          const d = ev.mint ? decimals[ev.mint] : undefined;
          const strong = ev.kind === 'reveal-claim' || ev.kind === 'private-claim' || ev.kind === 'refund';
          return (
            <li key={ev.signature} className={strong ? 'ev ev-strong' : 'ev'}>
              <span className={`chip ${l.cls}`}>{l.text}</span>
              <span className="ev-body">
                {ev.amount !== null && ev.mint && (
                  <span className="mono">{d === undefined ? `${ev.amount} raw` : formatTokenAmount(ev.amount, d)} <span className="muted">{shorten(ev.mint)}</span></span>
                )}
                {l.note && <span className="sub">{l.note}</span>}
              </span>
              <a className="mono" href={`https://explorer.solana.com/tx/${ev.signature}?cluster=${cluster}`} target="_blank" rel="noreferrer">{shorten(ev.signature, 6, 4)}</a>
              <time className="muted">{ev.blockTime ? ago(nowMs, ev.blockTime * 1000) : '–'}</time>
            </li>
          );
        })}
      </ul>
      <p className="note">Reveal Claims publish the Zcash txid on Solana by design. This page never reads or shows it, and a Private Claim leaves no txid anywhere.</p>
    </section>
  );
}
