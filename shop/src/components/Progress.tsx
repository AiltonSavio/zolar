import { finalityProgress, STEPS, stepIndex, type JobView } from '@zolar/sdk';
import { explorer, short, zec, type ShopConfig } from '../config';
import { CheckIcon } from './Icons';

/** The checkout stages as a stepper, with the live detail of the current one. */
export function Progress({ view, cluster }: { view: JobView; cluster: ShopConfig['cluster'] }) {
  if (view.stage === 'recovering') {
    return (
      <section className="stub-bottom progress" role="status">
        <p className="eyebrow">Checkout in progress</p>
        <h2 className="display">Reconnecting to your checkout</h2>
        <p>Your Zkool connection is restoring the saved checkout and its current status. Keep it running.</p>
      </section>
    );
  }
  const current = stepIndex(view);
  const finality = finalityProgress(view);
  const escrow = 'escrow' in view ? view.escrow : null;
  const detail = (key: string): React.ReactNode => {
    if (key === 'approve' && current === 0) return 'Approve in your Zkool connection window.';
    if (key === 'lock' && escrow)
      return (
        <a href={explorer(cluster, 'address', escrow)} target="_blank" rel="noreferrer">
          Escrow {short(escrow)}
        </a>
      );
    if (key === 'final' && current === 3) {
      if (!finality) return 'Waiting for the payment to be mined';
      const left = Math.max(1, Math.ceil(((finality.of - finality.done + 1) * 25) / 60));
      return (
        <>
          {finality.done} of {finality.of} blocks on top · about {left} min left
          <span className="meter" aria-hidden="true">
            <span style={{ transform: `scaleX(${finality.done / finality.of})` }} />
          </span>
        </>
      );
    }
    if (key === 'prove' && current === 4) return 'About a minute, on your computer.';
    return null;
  };
  return (
    <section className="stub-bottom progress" aria-live="polite">
      <div className="bottom-head">
        <div>
          <p className="eyebrow">Checkout in progress</p>
          <h2 className="display">{view.stage === 'awaiting_approval' ? 'Ready when you are' : 'Processing your ticket'}</h2>
          <p className="mono muted">{view.stage === 'awaiting_approval' ? 'Review the payment and approve in your Zkool connection to continue.' : 'Your shielded Zcash payment is being verified on Solana.'}</p>
        </div>
        <div className="total">
          <p className="eyebrow">Order total</p>
          <p className="price">{zec(view.price_zats)} ZEC</p>
          <p className="mono">(testnet)</p>
        </div>
      </div>
      <ol className="stepper">
        {STEPS.map((step, i) => {
          const state = i < current ? 'done' : i === current ? 'now' : 'later';
          const d = detail(step.key);
          return (
            <li key={step.key} className={`step step-${state}`} aria-current={state === 'now' ? 'step' : undefined}>
              <span className="step-dot">{state === 'done' ? <CheckIcon /> : i + 1}</span>
              <span className="step-label">{step.label}</span>
              {d && <span className="step-detail">{d}</span>}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
