import { finalityProgress, STEPS, stepIndex, type JobView } from '@zolar/sdk';
import { explorer, short, type ShopConfig } from '../config';

export function Progress({ view, cluster }: { view: JobView; cluster: ShopConfig['cluster'] }) {
  const current = stepIndex(view);
  const finality = finalityProgress(view);
  const escrow = 'escrow' in view ? view.escrow : null;
  return (
    <section className="panel" aria-live="polite">
      <h2>Getting your ticket</h2>
      <ol className="steps">
        {STEPS.map((step, i) => {
          const state = i < current ? 'done' : i === current ? 'now' : 'later';
          return (
            <li key={step.key} className={`step step-${state}`}>
              <span className="dot" aria-hidden="true" />
              <span className="label">{step.label}</span>
              {step.key === 'approve' && state === 'now' && (
                <span className="detail">Check the Zolar Wallet window and approve the payment.</span>
              )}
              {step.key === 'lock' && escrow && (
                <span className="detail">
                  Escrow{' '}
                  <a href={explorer(cluster, 'address', escrow)} target="_blank" rel="noreferrer">
                    {short(escrow)}
                  </a>
                </span>
              )}
              {step.key === 'final' && state === 'now' && (
                <span className="detail">
                  {finality
                    ? `${finality.done} of ${finality.of} blocks on top · about ${Math.max(1, Math.ceil(((finality.of - finality.done + 1) * 25) / 60))} min left`
                    : 'Waiting for the payment to be mined'}
                </span>
              )}
              {step.key === 'prove' && state === 'now' && <span className="detail">About a minute, on your computer.</span>}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
