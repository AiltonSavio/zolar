import {
  checkout,
  MerchantClient,
  openApprovalWindow,
  WalletClient,
  type CatalogEntry,
  type JobView,
  type WalletStatus,
} from '@zolar/sdk';
import { useEffect, useMemo, useState } from 'react';
import { Progress } from './components/Progress';
import { Success } from './components/Success';
import { type ShopConfig, short, zec } from './config';
import { connectAddress, looksLikeAddress, useSolanaWallets } from './solanaWallet';

const EVENT = 'Shielded Night';

type Phase =
  | { kind: 'shopping' }
  | { kind: 'running'; view: JobView | null; approvalUrl: string | null }
  | { kind: 'finished'; view: JobView };

export function App({ config }: { config: ShopConfig }) {
  const merchant = useMemo(() => new MerchantClient(config.merchantUrl), [config.merchantUrl]);
  const wallet = useMemo(() => new WalletClient(config.walletUrl), [config.walletUrl]);
  const solanaWallets = useSolanaWallets();

  const [entry, setEntry] = useState<CatalogEntry | null>(null);
  const [walletStatus, setWalletStatus] = useState<WalletStatus | null | undefined>(undefined);
  const [payout, setPayout] = useState('');
  const [phase, setPhase] = useState<Phase>({ kind: 'shopping' });
  const [error, setError] = useState<string | null>(null);
  // Bumped when a checkout ends, so the stock shown is read again.
  const [stockVersion, setStockVersion] = useState(0);

  useEffect(() => {
    merchant
      .items()
      .then((items) => setEntry(items.find((i) => i.item.id === config.itemId) ?? null))
      .catch((e: Error) => setError(`The shop is not reachable: ${e.message}`));
  }, [merchant, config.itemId, stockVersion]);

  useEffect(() => {
    let live = true;
    const look = () => wallet.status().then((s) => live && setWalletStatus(s));
    look();
    const t = setInterval(look, 5_000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [wallet]);

  const item = entry?.item;
  const canPay = !!item && !!walletStatus && looksLikeAddress(payout) && phase.kind === 'shopping';

  async function pay() {
    if (!item) return;
    setError(null);
    // Opened inside the click, or the browser blocks the wallet's window.
    const approvalWindow = openApprovalWindow();
    setPhase({ kind: 'running', view: null, approvalUrl: null });
    try {
      const result = await checkout({
        merchant,
        wallet,
        itemId: item.id,
        payout: payout.trim(),
        approvalWindow,
        onApprovalUrl: (url) => setPhase((p) => (p.kind === 'running' ? { ...p, approvalUrl: url } : p)),
        onStage: (view) => setPhase((p) => (p.kind === 'running' ? { ...p, view } : p)),
      });
      setPhase({ kind: 'finished', view: result.final });
    } catch (e) {
      approvalWindow?.close();
      setError((e as Error).message);
      setPhase({ kind: 'shopping' });
    } finally {
      setStockVersion((v) => v + 1);
    }
  }

  async function connect(index: number) {
    const w = solanaWallets[index];
    if (!w) return;
    try {
      setPayout(await connectAddress(w));
    } catch (e) {
      setError((e as Error).message);
    }
  }

  return (
    <div className="page">
      <header className="top">
        <span className="brand">Zolar</span>
        <span className="net">Zcash testnet · Solana {config.cluster}</span>
      </header>

      <main className="layout">
        <section className="event">
          <p className="eyebrow">Event</p>
          <h1>{EVENT}</h1>
          <p className="subtitle">A Zcash party during Breakpoint London</p>
          <p className="facts">London · 16 November 2026 · 21:00</p>
          <p className="tagline">A night for people who like their payments private.</p>
          <div className="ticket-row">
            <span>{item?.name ?? 'General admission'}</span>
            <span>{item ? `${zec(item.price_zats)} ZEC (testnet)` : '…'}</span>
            <span>{entry?.left != null ? `${entry.left} left` : ''}</span>
          </div>
          <p className="seller">Seller: {EVENT} (demo shop on Solana {config.cluster})</p>
          <p className="fine">Unofficial side event. Not affiliated with Breakpoint or the Solana Foundation.</p>
        </section>

        <aside className="checkout">
          {phase.kind === 'shopping' && (
            <section className="panel">
              <h2>Checkout</h2>
              <div className="field">
                <span className="field-label">1. Your Solana wallet</span>
                <div className="wallets">
                  {solanaWallets.map((w, i) => (
                    <button key={w.name} type="button" className="secondary" onClick={() => connect(i)}>
                      Connect {w.name}
                    </button>
                  ))}
                </div>
                <label className="field-label small" htmlFor="payout">
                  {solanaWallets.length ? 'or paste an address' : 'Paste your Solana address'}
                </label>
                <input
                  id="payout"
                  value={payout}
                  onChange={(e) => setPayout(e.target.value)}
                  placeholder="Solana address"
                  spellCheck={false}
                  autoComplete="off"
                />
                {payout && !looksLikeAddress(payout) && <span className="hint bad">That is not a Solana address.</span>}
                {looksLikeAddress(payout) && <span className="hint">Ticket goes to {short(payout.trim())}</span>}
              </div>
              <div className="field">
                <span className="field-label">2. Pay with Zcash</span>
                <button type="button" className="primary" disabled={!canPay} onClick={pay}>
                  Pay {item ? `${zec(item.price_zats)} ZEC` : ''} with Zcash
                </button>
                {walletStatus === null && (
                  <span className="hint bad">
                    Zolar Wallet is not running on this computer (or does not allow this site). Start it, then try again.
                  </span>
                )}
                {walletStatus && <span className="hint">Zolar Wallet found ({walletStatus.network}).</span>}
              </div>
              <p className="note">Your ticket arrives in this wallet. Solana never learns which Zcash payment paid for it.</p>
            </section>
          )}

          {phase.kind === 'running' && (
            <>
              {phase.view ? (
                <Progress view={phase.view} cluster={config.cluster} />
              ) : (
                <section className="panel">
                  <h2>Starting checkout…</h2>
                </section>
              )}
              {phase.approvalUrl && (
                <p className="note">
                  Your browser blocked the wallet’s window.{' '}
                  <a href={phase.approvalUrl} target="_blank" rel="noreferrer">
                    Open Zolar Wallet to approve
                  </a>
                </p>
              )}
            </>
          )}

          {phase.kind === 'finished' && phase.view.stage === 'done' && (
            <Success view={phase.view} cluster={config.cluster} event={EVENT} />
          )}
          {phase.kind === 'finished' && phase.view.stage !== 'done' && (
            <section className="panel">
              <h2>{phase.view.stage === 'rejected' ? 'Payment rejected' : 'Checkout failed'}</h2>
              <p>
                {phase.view.stage === 'failed'
                  ? phase.view.reason
                  : 'Nothing was paid. The shop releases the ticket after the order lapses.'}
              </p>
              <button type="button" className="secondary" onClick={() => setPhase({ kind: 'shopping' })}>
                Back to the shop
              </button>
            </section>
          )}

          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}
        </aside>
      </main>

      <footer className="foot">Powered by Zolar — private Zcash checkout on Solana</footer>
    </div>
  );
}
