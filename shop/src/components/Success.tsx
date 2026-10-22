import type { JobView } from '@zolar/sdk';
import { explorer, short, type ShopConfig } from '../config';

type Done = Extract<JobView, { stage: 'done' }>;

export function Success({ view, cluster, event }: { view: Done; cluster: ShopConfig['cluster']; event: string }) {
  const privacy =
    view.txid_found_on_solana === false
      ? 'Private Claim verified on Solana. The Zcash transaction ID appears in none of this Escrow’s Solana transactions.'
      : view.txid_found_on_solana === true
        ? 'Warning: the Zcash transaction ID was found on Solana.'
        : 'The privacy check did not run.';
  return (
    <section className="panel success">
      <h2>You’re in.</h2>
      <div className="ticket">
        <div className="ticket-event">{event}</div>
        <div className="ticket-kind">{view.item}</div>
        <div className="ticket-to">In wallet {short(view.payout)}</div>
      </div>
      <p className={view.txid_found_on_solana === true ? 'privacy bad' : 'privacy'}>{privacy}</p>
      <p className="links">
        <a href={explorer(cluster, 'tx', view.claim)} target="_blank" rel="noreferrer">
          View the claim on Solana Explorer
        </a>
        {' · '}
        <a href={explorer(cluster, 'address', view.escrow)} target="_blank" rel="noreferrer">
          View the Escrow
        </a>
      </p>
    </section>
  );
}
