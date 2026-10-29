import {
  checkout,
  watchCheckout,
  isTerminal,
  MerchantClient,
  openApprovalWindow,
  WalletClient,
  type CatalogEntry,
  type JobView,
  type OrderTerms,
  type WalletStatus,
} from '@zolar/sdk';
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react';
import { Brand } from './components/Brand';
import { CalendarIcon, ClockIcon, PinIcon, ShieldIcon, WalletIcon } from './components/Icons';
import { Progress } from './components/Progress';
import { Success } from './components/Success';
import { type ShopConfig, short, zec } from './config';
import { looksLikeAddress, useSolanaConnection } from './solanaWallet';
import { WalletControl } from './components/WalletControl';
import { PaymentDialog } from './components/PaymentDialog';
import { supportedZcashWallet, ZcashWalletControl } from './components/ZcashWalletControl';

const VendingApp = lazy(() => import('./VendingApp').then((module) => ({ default: module.VendingApp })));

const EVENT = 'Shielded Night';

type Phase =
  | { kind: 'shopping' }
  | { kind: 'running'; view: JobView | null; approvalUrl: string | null; order: OrderTerms | null; payout: string }
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
  const connection = useSolanaConnection();
  const checkoutController = useRef<AbortController | null>(null);
  const approvalWindow = useRef<Window | null>(null);
  const reviewButton = useRef<HTMLButtonElement>(null);
  const [paymentOpen, setPaymentOpen] = useState(false);
  const [walletOpened, setWalletOpened] = useState(false);
  const [popupBlocked, setPopupBlocked] = useState(false);

  const [entry, setEntry] = useState<CatalogEntry | null>(null);
  const [walletStatus, setWalletStatus] = useState<WalletStatus | null | undefined>(undefined);
  const [zcashWallet, setZcashWallet] = useState<WalletStatus | null>(null);
  const payout = connection.selection?.address ?? '';
  const [phase, setPhase] = useState<Phase>({ kind: 'shopping' });
  const [error, setError] = useState<string | null>(null);
  const [resumeChecked, setResumeChecked] = useState(false);
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

  useEffect(() => {
    if (zcashWallet && (!supportedZcashWallet(walletStatus) || walletStatus.address !== zcashWallet.address
      || walletStatus.account !== zcashWallet.account)) setZcashWallet(null);
  }, [walletStatus, zcashWallet]);

  useEffect(() => {
    let live = true;
    const controller = new AbortController();
    setResumeChecked(false);
    const restore = async () => {
      while (live && !controller.signal.aborted) {
        try {
          const saved = await wallet.resume();
          if (!live || checkoutController.current) return;
          if (!saved) { setResumeChecked(true); return; }
          const initial = await wallet.job(saved.job, saved.token);
          if (!live || checkoutController.current) return;
          let dismissed: string | null = null;
          try { dismissed = sessionStorage.getItem(`zolar-dismissed:${wallet.url}`); } catch { dismissed = null; }
          if (isTerminal(initial) && dismissed === saved.job) { setResumeChecked(true); return; }
          checkoutController.current = controller;
          setError(null); setPaymentOpen(false); setResumeChecked(true);
          setPhase({ kind: 'running', view: initial, approvalUrl: null, order: null, payout: initial.payout });
          const final = await watchCheckout({ wallet, ...saved, initialView: initial,
            signal: controller.signal, maxPollErrors: Infinity,
            onStage: (view) => { if (!controller.signal.aborted) setPhase({ kind: 'running', view, approvalUrl: null, order: null, payout: initial.payout }); },
          });
          if (live && !controller.signal.aborted) { setPhase({ kind: 'finished', view: final }); setStockVersion((v) => v + 1); }
          if (checkoutController.current === controller) checkoutController.current = null;
          return;
        } catch {
          // Discovery remains pending until the same wallet answers; it cannot create an order.
          if (!live || controller.signal.aborted) return;
          await new Promise<void>((resolve) => setTimeout(resolve, 2_000));
        }
      }
    };
    void restore();
    return () => { live = false; controller.abort(); if (checkoutController.current === controller) checkoutController.current = null; };
  }, [wallet]);

  function dismissReceipt(job: string) {
    try { sessionStorage.setItem(`zolar-dismissed:${wallet.url}`, job); } catch { /* Dismissal still applies to this page when storage is unavailable. */ }
    setPhase({ kind: 'shopping' });
  }

  async function connectZcash() {
    const status = await wallet.status();
    setWalletStatus(status);
    if (!supportedZcashWallet(status)) throw new Error('The Zkool testnet wallet is unavailable.');
    setZcashWallet(status);
  }

  useEffect(() => {
    const keyboard = () => { document.documentElement.dataset.input = 'keyboard'; };
    const pointer = () => { document.documentElement.dataset.input = 'pointer'; };
    document.addEventListener('keydown', keyboard, true);
    document.addEventListener('pointerdown', pointer, true);
    return () => {
      document.removeEventListener('keydown', keyboard, true);
      document.removeEventListener('pointerdown', pointer, true);
      checkoutController.current?.abort();
    };
  }, []);

  const item = entry?.item;
  const savedView = phase.kind === 'finished' ? phase.view : phase.kind === 'running' ? phase.view : null;
  const itemName = savedView?.item ?? (phase.kind === 'running' ? phase.order?.item.name : undefined) ?? item?.name ?? 'General admission';
  const price = savedView ? `${zec(savedView.price_zats)} ZEC` : item ? `${zec(item.price_zats)} ZEC` : '…';
  const payoutOk = looksLikeAddress(payout);
  const canPay = !!item && !!zcashWallet && payoutOk && resumeChecked && phase.kind === 'shopping';
  const done = phase.kind === 'finished' && phase.view.stage === 'done' ? phase.view : null;

  async function pay() {
    if (!item || !canPay || checkoutController.current) return;
    const controller = new AbortController();
    checkoutController.current = controller;
    setError(null);
    setWalletOpened(false);
    setPopupBlocked(false);
    approvalWindow.current = null;
    setPaymentOpen(true);
    setPhase({ kind: 'running', view: null, approvalUrl: null, order: null, payout });
    try {
      const result = await checkout({
        merchant, wallet, maxPollErrors: Infinity, itemId: item.id, payout: payout.trim(), zcashWallet: zcashWallet ?? undefined, signal: controller.signal,
        // The first click opens the in-page review. The customer opens the wallet explicitly.
        onApprovalUrl: (url) => setPhase((p) => p.kind === 'running' ? { ...p, approvalUrl: url } : p),
        onOrder: (order) => setPhase((p) => p.kind === 'running' ? { ...p, order } : p),
        onStage: (view) => {
          if (controller.signal.aborted) return;
          setPhase((p) => p.kind === 'running' ? { ...p, view } : p);
          if (view.stage !== 'awaiting_approval') setPaymentOpen(false);
        },
      });
      if (controller.signal.aborted) return;
      setPhase({ kind: 'finished', view: result.final });
    } catch (e) {
      if (controller.signal.aborted) return;
      setError((e as Error).message);
      setPhase({ kind: 'shopping' });
    } finally {
      if (!controller.signal.aborted) { setPaymentOpen(false); setStockVersion((v) => v + 1); }
      if (checkoutController.current === controller) checkoutController.current = null;
    }
  }

  function openWalletApproval() {
    if (phase.kind !== 'running' || !phase.approvalUrl) return;
    try {
      if (approvalWindow.current && !approvalWindow.current.closed) {
        approvalWindow.current.focus();
        return;
      }
      const popup = openApprovalWindow();
      if (!popup) { setPopupBlocked(true); return; }
      popup.opener = null;
      popup.location.href = phase.approvalUrl;
      popup.focus();
      approvalWindow.current = popup;
      setPopupBlocked(false);
      setWalletOpened(true);
    } catch { setPopupBlocked(true); }
  }

  function closePayment() {
    setPaymentOpen(false);
    requestAnimationFrame(() => reviewButton.current?.focus());
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
                <p className="value">{itemName}</p>
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
                  {phase.kind === 'running' && (
                    <div>
                      <p className="eyebrow">Ticket wallet</p>
                      <p className="mono wallet-line"><WalletIcon /> {short(phase.payout)}</p>
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
                <WalletControl connection={connection} />
                <ZcashWalletControl status={walletStatus} selection={zcashWallet} onConnect={connectZcash} onDisconnect={() => setZcashWallet(null)} />
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
                    Connect an existing Zcash testnet wallet to pay.
                  </p>
                )}
                {zcashWallet && <p className="mono small">Zkool connected (testnet).</p>}
                {!resumeChecked && <p className="mono small">Checking for an existing checkout…</p>}
                <p className="mono small">Your ticket arrives in your Solana wallet. Solana never learns which Zcash payment paid for it.</p>
              </div>
            </section>
          )}

          {phase.kind === 'running' && (
            <>
              {phase.view ? <Progress view={phase.view} cluster={config.cluster} /> : <section className="stub-bottom"><p className="eyebrow">Checkout</p><h2 className="display">Starting checkout…</h2></section>}
              {(!phase.view || phase.view.stage === 'awaiting_approval') && <div className="review-payment"><button ref={reviewButton} type="button" className="outline" onClick={() => setPaymentOpen(true)}><ShieldIcon />Review payment</button><p className="mono small muted">Authorize your Zkool payment to continue. Closing the dialog keeps this checkout open.</p></div>}
              <PaymentDialog open={paymentOpen} onClose={closePayment} order={phase.order} item={itemName} price={item?.price_zats ?? 0} payout={phase.payout} network={walletStatus?.network ?? 'testnet'} approvalUrl={phase.approvalUrl} onOpenWallet={openWalletApproval} opened={walletOpened} blocked={popupBlocked} />
            </>
          )}

          {done && <><Success view={done} cluster={config.cluster} /><section className="stub-bottom"><button type="button" className="outline" onClick={() => dismissReceipt(done.job)}>Buy another ticket</button></section></>}

          {phase.kind === 'finished' && !done && (
            <section className="stub-bottom">
              <p className="eyebrow">Checkout</p>
              <h2 className="display">{phase.view.stage === 'rejected' ? 'Payment rejected' : 'Checkout failed'}</h2>
              <p className="mono">
                {phase.view.stage === 'failed'
                  ? phase.view.reason
                  : 'Nothing was paid. The shop releases the ticket after the order lapses.'}
              </p>
              <button type="button" className="outline" onClick={() => dismissReceipt(phase.view.job)}>
                Back to the shop
              </button>
            </section>
          )}
        </article>


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
