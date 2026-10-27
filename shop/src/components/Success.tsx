import type { JobView } from '@zolar/sdk';
import { explorer, type ShopConfig } from '../config';
import { LinkIcon, ShieldIcon } from './Icons';

type Done = Extract<JobView, { stage: 'done' }>;

/** The privacy result of a finished checkout, with links to check it. */
export function Success({
  view,
  cluster,
  vending = false,
}: {
  vending?: boolean;
  view: Done;
  cluster: ShopConfig['cluster'];
}) {
  const privacy = vending
    ? 'Ticket minted to your wallet.'
    : view.txid_found_on_solana === false
      ? 'Private Claim verified on Solana. The Zcash transaction ID appears in none of this Escrow’s Solana transactions.'
      : view.txid_found_on_solana === true
        ? 'Warning: the Zcash transaction ID was found on Solana.'
        : 'The privacy check did not run.';
  return (
    <section className="stub-bottom privacy-panel">
      <span className={`privacy-badge ${view.txid_found_on_solana === true ? 'bad' : ''}`}>
        <ShieldIcon />
      </span>
      <div>
        <p className="eyebrow">{vending ? 'Your ticket' : 'Privacy'}</p>
        <p className={`privacy ${view.txid_found_on_solana === true ? 'bad' : ''}`}>{privacy}</p>
        <p className="links mono">
          <a href={explorer(cluster, 'tx', view.claim)} target="_blank" rel="noreferrer">
            <LinkIcon /> View the claim on Solana Explorer
          </a>
          <a href={explorer(cluster, 'address', view.escrow)} target="_blank" rel="noreferrer">
            <LinkIcon /> {vending ? 'View the reservation address' : 'View the Escrow'}
          </a>
        </p>
      </div>
    </section>
  );
}
