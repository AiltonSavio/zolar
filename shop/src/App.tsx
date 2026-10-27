import {
  checkout,
  MerchantClient,
  openApprovalWindow,
  WalletClient,
  type CatalogEntry,
  type JobView,
  type WalletStatus,
} from '@zolar/sdk';
import { lazy, Suspense, useEffect, useMemo, useState } from 'react';
import { Brand } from './components/Brand';
import { CalendarIcon, ClockIcon, PinIcon, ShieldIcon, WalletIcon } from './components/Icons';
import { Progress } from './components/Progress';
import { Success } from './components/Success';
import { type ShopConfig, short, zec } from './config';
import { connectAddress, looksLikeAddress, useSolanaWallets } from './solanaWallet';

const VendingApp = lazy(() => import('./VendingApp').then((module) => ({ default: module.VendingApp })));

const EVENT = 'Shielded Night';

type Phase =
  | { kind: 'shopping' }
  | { kind: 'running'; view: JobView | null; approvalUrl: string | null }
  | { kind: 'finished'; view: JobView };

export function App({ config }: { config: ShopConfig }) {
  return config.mode === 'vending' ? (
    <Suspense fallback={<p className="mono notice">Loading checkout…</p>}>
      <VendingApp config={config} />
    </Suspense>
  ) : (
    <CheckoutApp config={config} />
  );
}

function CheckoutApp({ config }: { config: ShopConfig }) {
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
  const price = item ? `${zec(item.price_zats)} ZEC` : '…';
  const payoutOk = looksLikeAddress(payout);
  const canPay = !!item && !!walletStatus && payoutOk && phase.kind === 'shopping';
  const done = phase.kind === 'finished' && phase.view.stage === 'done' ? phase.view : null;

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
        <Brand />
        <span className="pill">
          <span className="dot" aria-hidden="true" />
          Zcash testnet · Solana {config.cluster}
          <span className="badge">Demo</span>
        </span>
      </header>

      <main>
        {done && (
          <div className="hero">
            <h1 className="display">You’re in.</h1>
            <p className="mono">A night for people who like their payments private.</p>
          </div>
        )}

        <article className="stub">
          <section className="stub-top">
            <img
              className="art"
              src="/london-night.webp"
              alt="Tower Bridge and the Thames at night, in black and yellow"
            />
            <div className="event">
              <p className="eyebrow">Event</p>
              {done ? <h2 className="display title">{EVENT}</h2> : <h1 className="display title">{EVENT}</h1>}
              <p className="mono subtitle">A Zcash party during Breakpoint London</p>
              <ul className="facts">
                <li>
                  <PinIcon /> London
                </li>
                <li>
                  <CalendarIcon /> 16 November 2026
                </li>
                <li>
                  <ClockIcon /> 21:00
                </li>
              </ul>
              {!done && <p className="tagline">A night for people who like their payments private.</p>}
              <p className="mono fine">
                Unofficial side event. Not affiliated with Breakpoint or the Solana Foundation.
              </p>
            </div>
            <aside className="details">
              <div>
                <p className="eyebrow">Ticket</p>
                <p className="value">{item?.name ?? 'General admission'}</p>
              </div>
              {done ? (
                <div className="wallet-line">
                  <WalletIcon />
                  <div>
                    <p className="value">Ticket in wallet</p>
                    <p className="mono">{short(done.payout)}</p>
                  </div>
                </div>
              ) : (
                <>
                  <div>
                    <p className="eyebrow">Price</p>
                    <p className="price">{price}</p>
                    <p className="mono">(testnet)</p>
                  </div>
                  {phase.kind === 'running' && payoutOk && (
                    <div>
                      <p className="eyebrow">Ticket wallet</p>
                      <p className="mono wallet-line">
                        <WalletIcon /> {short(payout.trim())}
                      </p>
                    </div>
                  )}
                  <div>
                    <p className="eyebrow">Availability</p>
                    <p className="value">{entry?.left != null ? `${entry.left} of 100 left` : '…'}</p>
                  </div>
                  <div>
                    <p className="eyebrow">Seller</p>
                    <p className="value">{EVENT}</p>
                    <p className="mono small">(demo shop on Solana {config.cluster})</p>
                  </div>
                </>
              )}
            </aside>
          </section>

          <div className="perforation" aria-hidden="true" />

          {phase.kind === 'shopping' && (
            <section className="stub-bottom checkout">
              <div className="get">
                <p className="eyebrow">Checkout</p>
                <h2 className="display">Get your ticket</h2>
                {solanaWallets.length > 0 && (
                  <div className="wallets">
                    {solanaWallets.map((w, i) => (
                      <button key={w.name} type="button" className="outline" onClick={() => connect(i)}>
                        <WalletIcon /> Connect {w.name}
                      </button>
                    ))}
                  </div>
                )}
                <label className="field">
                  <span className="mono small">
                    {solanaWallets.length ? 'or paste a Solana address' : 'Your Solana address (the ticket goes here)'}
                  </span>
                  <span className="input">
                    <WalletIcon />
                    <input
                      value={payout}
                      onChange={(e) => setPayout(e.target.value)}
                      placeholder="Solana address"
                      aria-label="Solana address"
                      spellCheck={false}
                      autoComplete="off"
                    />
                    {payoutOk && <span className="mono ok">● {short(payout.trim())}</span>}
                  </span>
                </label>
                {payout && !payoutOk && <p className="mono small bad">That is not a Solana address.</p>}
              </div>
              <div className="pay">
                <p className="eyebrow">Total</p>
                <p className="price">{price}</p>
                <p className="mono">(testnet)</p>
                <button type="button" className="primary" disabled={!canPay} onClick={pay}>
                  <ShieldIcon /> Pay with Zcash
                </button>
                {walletStatus === null && (
                  <p className="mono small bad">
                    Zolar Wallet is not running on this computer (or does not allow this site). Start it, then try
                    again.
                  </p>
                )}
                {walletStatus && <p className="mono small">Zolar Wallet found ({walletStatus.network}).</p>}
                <p className="mono small">
                  Your ticket arrives in this wallet. Solana never learns which Zcash payment paid for it.
                </p>
              </div>
            </section>
          )}

          {phase.kind === 'running' &&
            (phase.view ? (
              <Progress view={phase.view} cluster={config.cluster} />
            ) : (
              <section className="stub-bottom">
                <p className="eyebrow">Checkout</p>
                <h2 className="display">Starting checkout…</h2>
              </section>
            ))}

          {done && <Success view={done} cluster={config.cluster} />}

          {phase.kind === 'finished' && !done && (
            <section className="stub-bottom">
              <p className="eyebrow">Checkout</p>
              <h2 className="display">{phase.view.stage === 'rejected' ? 'Payment rejected' : 'Checkout failed'}</h2>
              <p className="mono">
                {phase.view.stage === 'failed'
                  ? phase.view.reason
                  : 'Nothing was paid. The shop releases the ticket after the order lapses.'}
              </p>
              <button type="button" className="outline" onClick={() => setPhase({ kind: 'shopping' })}>
                Back to the shop
              </button>
            </section>
          )}
        </article>

        {phase.kind === 'running' && phase.approvalUrl && (
          <p className="mono notice">
            Your browser blocked the wallet’s window.{' '}
            <a href={phase.approvalUrl} target="_blank" rel="noreferrer">
              Open Zolar Wallet to approve
            </a>
          </p>
        )}
        {error && (
          <p className="mono notice bad" role="alert">
            {error}
          </p>
        )}
      </main>

      <footer className="foot">
        <Brand size="small" />
        <p className="mono small">Powered by Zolar — private Zcash checkout on Solana.</p>
      </footer>
    </div>
  );
}
