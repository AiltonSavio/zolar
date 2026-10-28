import { useEffect, useRef, useState } from 'react';
import type { RelayView } from '../model/relay';
import { formatWork, shorten } from '../format';

interface Props {
  label: string;
  address: string;
  view: RelayView | undefined;
  error: string | undefined;
}

/** Bumps `flash` for a moment whenever `value` changes, so a new tip is noticed. */
function useFlash(value: unknown): boolean {
  const first = useRef(true);
  const [flash, setFlash] = useState(false);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const on = setTimeout(() => setFlash(true), 0);
    const off = setTimeout(() => setFlash(false), 1200);
    return () => {
      clearTimeout(on);
      clearTimeout(off);
    };
  }, [value]);
  return flash;
}

export function RelayCard({ label, address, view, error }: Props) {
  const flash = useFlash(view?.tipHeight);
  const halted = view?.halt.halted === true;
  return (
    <section className={`card relay ${halted ? 'is-halted' : ''}`} aria-label={`${label} Header Relay`}>
      <header className="card-head">
        <div>
          <p className="eyebrow">Header Relay</p>
          <h2>{label}</h2>
        </div>
        <span className={`status ${halted ? 'status-halt' : view ? 'status-ok' : 'status-wait'}`}>
          {halted ? 'HALTED' : view ? 'Running' : 'Loading'}
        </span>
      </header>

      {view?.halt.halted && (
        <div className="halt-banner" role="alert">
          <strong>HALTED</strong>
          <span>{view.halt.reason}</span>
          <span className="mono">
            at tip {view.halt.haltHeight.toLocaleString('en-US')}, header {shorten(view.halt.haltHash, 8, 8)}
          </span>
          <span>No more headers are accepted and no Claim settles against this relay.</span>
        </div>
      )}
      {error && <p className="error" role="status">RPC problem: {error}</p>}

      {view && (
        <>
          <div className={`tip ${flash ? 'flash' : ''}`}>
            <span className="label">Tip height</span>
            <span className="tip-height mono" data-testid="tip-height">{view.tipHeight.toLocaleString('en-US')}</span>
            <span className="mono hash" title={view.tipHash}>{shorten(view.tipHash, 10, 10)}</span>
          </div>
          <dl className="grid">
            <div><dt>Chain work</dt><dd className="mono" title={view.chainWork.toString()}>{formatWork(view.chainWork)}</dd></div>
            <div><dt>Finality Depth</dt><dd className="mono">{view.finalityDepth}</dd></div>
            <div><dt>Final Block Tree</dt><dd className="mono">{view.treeSize.toLocaleString('en-US')} blocks</dd></div>
            <div><dt>Final up to height</dt><dd className="mono">{view.finalizedHeight.toLocaleString('en-US')}</dd></div>
            <div className="wide"><dt>Latest tree root</dt><dd className="mono hash" title={view.latestRoot}>{shorten(view.latestRoot, 12, 12)}</dd></div>
          </dl>
          {!view.sealed && <p className="error">This relay is not sealed yet.</p>}
        </>
      )}
      <footer className="mono addr" title={address}>{shorten(address, 6, 6)}</footer>
    </section>
  );
}
