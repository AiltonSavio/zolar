import {
  fromHex,
  hex,
  finalityProgress,
  paymentLink,
  vendingStatus,
  canRequestRelease,
  releaseSecondsRemaining,
  VendingProverClient,
  zatoshiAmount,
} from '@zolar/sdk';
import type { VendingPayment } from '@zolar/sdk';
import { PublicKey } from './vendingInstructions';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import QRCode from 'qrcode';
import { Brand } from './components/Brand';
import { CalendarIcon, ClockIcon, PinIcon, ShieldIcon, WalletIcon } from './components/Icons';
import { Success } from './components/Success';
import { short, type ShopConfig } from './config';
import type { VendingChain, VendingConnection, VendingProver, VendingSession, VendingSnapshot } from './vendingModel';
import { connectVendingWallet, createVendingChain } from './vendingInstructions';

type Phase = 'idle' | 'reserving' | 'proving' | 'claiming' | 'proof_failed' | 'claim_failed' | 'claim_pending' | 'done';

function VendingCheckout({
  config,
  chain: providedChain,
  prover: providedProver,
  connect = connectVendingWallet,
  pollMs = 5_000,
}: {
  config: ShopConfig;
  chain?: VendingChain;
  prover?: VendingProver;
  connect?: () => Promise<VendingConnection>;
  pollMs?: number;
}) {
  const settings = config.vending!;
  const chain = useMemo(() => providedChain ?? createVendingChain(settings), [providedChain, settings]);
  const prover = useMemo(
    () => providedProver ?? new VendingProverClient(settings.proverUrl),
    [providedProver, settings.proverUrl],
  );
  const storageKey = `zolar-vending:${settings.program}:${settings.machine}`;
  const [saved, setSaved] = useState<{ nonce: string; payout: string; claim?: string } | null>(() => {
    try {
      const value = JSON.parse(localStorage.getItem(storageKey) || 'null');
      return value && /^[a-f0-9]{64}$/.test(value.nonce) && typeof value.payout === 'string' ? value : null;
    } catch {
      return null;
    }
  });
  const save = useCallback(
    (value: typeof saved) => {
      if (value) localStorage.setItem(storageKey, JSON.stringify(value));
      else localStorage.removeItem(storageKey);
      setSaved(value);
    },
    [storageKey],
  );
  const [snapshot, setSnapshot] = useState<VendingSnapshot | null>(null);
  const [wallet, setWallet] = useState<VendingConnection | null>(null);
  const [session, setSession] = useState<VendingSession | null>(null);
  const [payment, setPayment] = useState<VendingPayment | null>(() => {
    try {
      const value = JSON.parse(localStorage.getItem(`${storageKey}:payment`) || 'null');
      return value ? { ...value, valueZat: BigInt(value.valueZat) } : null;
    } catch {
      return null;
    }
  });
  const [phase, setPhase] = useState<Phase>('idle');
  const [error, setError] = useState('');
  const [readError, setReadError] = useState('');
  const [claim, setClaim] = useState('');
  const [qr, setQr] = useState<{ link: string; data: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState('');
  const [releaseBusy, setReleaseBusy] = useState(false);
  const [now, setNow] = useState(Date.now());
  const working = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const value = await chain.read();
        const restored = session ? await chain.restore(session.nonce) : null;
        const found = session ? await prover.findPayment(settings.machine, session.nonce) : null;
        if (live) {
          setSnapshot(value);
          if (restored)
            setSession((previous) =>
              previous &&
              previous.releaseRequested === restored.releaseRequested &&
              previous.releaseRequestedAt === restored.releaseRequestedAt
                ? previous
                : restored,
            );
          if (found) {
            localStorage.setItem(
              `${storageKey}:payment`,
              JSON.stringify({ ...found, valueZat: found.valueZat.toString() }),
            );
            setPayment(found);
          }
          setReadError('');
          setNow(Date.now());
        }
      } catch {
        if (live) {
          setReadError('Cannot check the relay or payment. Do not pay.');
          setNow(Date.now());
        }
      }
      if (live) timer = setTimeout(poll, pollMs);
    }
    void poll();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [chain, prover, session, settings.machine, pollMs, storageKey]);

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, []);

  const status = snapshot
    ? vendingStatus({
        connected: !!wallet,
        available: snapshot.available,
        relay: snapshot.relay,
        reservation: session,
        payment,
        now,
        staleMs: 120_000,
      })
    : null;
  const paid =
    !!payment ||
    !!saved?.claim ||
    ['proving', 'claiming', 'proof_failed', 'claim_failed', 'claim_pending', 'done'].includes(phase);
  const allowed = !paid && !session?.releaseRequested && !readError && status === 'payment' && phase === 'idle';
  const link =
    session && snapshot && allowed
      ? paymentLink(
          settings.address,
          session.priceZat,
          new PublicKey(settings.machine).toBytes(),
          session.nonce,
          new PublicKey(session.payout).toBytes(),
        )
      : '';
  useEffect(() => {
    setCopied(false);
    setCopyError('');
    let live = true;
    if (link)
      QRCode.toDataURL(link, { width: 320, margin: 4, errorCorrectionLevel: 'M' })
        .then((value) => {
          if (live) setQr({ link, data: value });
        })
        .catch(() => {
          if (live) setError('Could not make the QR code. Use the payment link.');
        });
    return () => {
      live = false;
    };
  }, [link]);

  const runProof = useCallback(async () => {
    if (!session || !payment || !wallet || !snapshot || readError || working.current) return;
    working.current = true;
    setError('');
    setPhase('proving');
    let claiming = false;
    try {
      if (payment.payout !== session.payout || payment.valueZat < session.priceZat)
        throw new Error('Payment amount or ticket wallet does not match this reservation. Do not pay again.');
      const proof = await prover.prove(settings.machine, session.nonce, payment.txid);
      const current = await chain.read();
      if (!mounted.current) return;
      if (current.relay.halted) throw new Error('The relay is halted. Try again after the relay recovers.');
      claiming = true;
      setPhase('claiming');
      let submitted: string | undefined;
      let signature: string;
      try {
        signature = await chain.claim(
          {
            ...wallet,
            async send(tx) {
              submitted = await wallet.send(tx);
              save({ nonce: hex(session.nonce), payout: session.payout, claim: submitted });
              return submitted;
            },
          },
          session,
          proof,
        );
      } catch (error) {
        if (submitted) {
          setPhase('claim_pending');
          setError('Claim submitted. Check its status before retrying.');
          return;
        }
        throw error;
      }
      setClaim(signature);
      setPhase('done');
    } catch (e) {
      setError((e as Error).message);
      setPhase(claiming ? 'claim_failed' : 'proof_failed');
    } finally {
      working.current = false;
    }
  }, [chain, prover, settings.machine, session, payment, wallet, snapshot, readError, save]);
  useEffect(() => {
    if (status === 'prove' && phase === 'idle') void runProof();
  }, [status, phase, runProof]);

  async function reserve() {
    if (!wallet || status !== 'ready' || working.current || readError || saved?.claim) return;
    working.current = true;
    setPhase('reserving');
    setError('');
    try {
      if (saved && saved.payout !== wallet.address) throw new Error('Reconnect the wallet used for this reservation.');
      const nonce = saved ? fromHex(saved.nonce) : crypto.getRandomValues(new Uint8Array(32));
      save({ nonce: hex(nonce), payout: wallet.address });
      const reserved = await chain.reserve(wallet, nonce);
      setSession(reserved);
      setPhase('idle');
    } catch (e) {
      setError((e as Error).message);
      setPhase('idle');
    } finally {
      working.current = false;
    }
  }
  async function requestRelease() {
    if (!wallet || !session || working.current) return;
    working.current = true;
    setReleaseBusy(true);
    setError('');
    try {
      setSession(await chain.requestRelease(wallet, session));
      setNow(Date.now());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      working.current = false;
      setReleaseBusy(false);
    }
  }
  async function release() {
    if (!wallet || !session || working.current) return;
    working.current = true;
    setReleaseBusy(true);
    try {
      await chain.release(wallet, session);
      save(null);
      localStorage.removeItem(`${storageKey}:payment`);
      setSession(null);
      setPayment(null);
      setPhase('idle');
      setError('');
    } catch (e) {
      setError((e as Error).message);
    } finally {
      working.current = false;
      setReleaseBusy(false);
    }
  }
  async function recover() {
    if (!saved || !wallet || working.current) return;
    if (saved.payout !== wallet.address) {
      setError('Reconnect the wallet used for this reservation.');
      return;
    }
    working.current = true;
    try {
      const restored = await chain.restore(fromHex(saved.nonce));
      if (restored && restored.payout === wallet.address) {
        setSession(restored);
        setPhase(saved.claim ? 'claim_pending' : 'idle');
        setError('');
      } else if (saved.claim) {
        const status = await chain.transactionStatus(saved.claim);
        if (status === 'confirmed') {
          setClaim(saved.claim);
          setPhase('done');
          setSession({
            nonce: fromHex(saved.nonce),
            payout: saved.payout,
            address: settings.machine,
            priceZat: snapshot?.priceZat ?? 0n,
            expiryHeight: 0,
            createdAt: 0,
            releaseRequested: false,
            releaseRequestedAt: 0,
          });
        } else if (status === 'failed') {
          save({ nonce: saved.nonce, payout: saved.payout });
          setError('Claim failed. Reserve again with the saved nonce to retry.');
          setPhase('idle');
        } else setError('Claim status is not confirmed yet. Keep this reservation saved.');
      } else setError('Reservation not found yet. Retry Reserve with the same nonce.');
    } catch (e) {
      setError((e as Error).message);
    } finally {
      working.current = false;
    }
  }
  async function checkClaim() {
    if (!saved?.claim) return;
    try {
      const result = await chain.transactionStatus(saved.claim);
      if (result === 'confirmed') {
        setClaim(saved.claim);
        setPhase('done');
        setError('');
      } else if (result === 'failed') {
        save({ nonce: saved.nonce, payout: saved.payout });
        setPhase('claim_failed');
        setError('Claim failed on Solana. You can retry it.');
      } else setError('Claim is still awaiting confirmation.');
    } catch (e) {
      setError((e as Error).message);
    }
  }
  const reason =
    readError ||
    (status === 'halted'
      ? 'The relay is halted. Do not pay.'
      : status === 'stalled'
        ? 'The relay has stopped advancing. Do not pay.'
        : '');
  const requestable = !!snapshot && !!session && canRequestRelease(snapshot.relay, session, now / 1000);
  const releaseRemaining = session ? releaseSecondsRemaining(session, now / 1000) : null;
  const countdown =
    releaseRemaining === null
      ? ''
      : `${Math.floor(releaseRemaining / 60)}:${String(releaseRemaining % 60).padStart(2, '0')}`;
  const finality =
    snapshot && session && payment
      ? finalityProgress({
          stage: 'waiting_for_finality',
          escrow: session.address,
          tip: snapshot.relay.tip,
          final_height: snapshot.relay.finalHeight,
          expiry: session.expiryHeight,
          sent_tip: payment.height - 1,
          finality_depth: snapshot.relay.finalityDepth,
        })
      : null;
  const done = phase === 'done';

  return (
    <div className="page">
      <header className="top">
        <Brand />
        <span className="pill">
          <span className="dot" />
          Zcash testnet · Solana devnet<span className="badge">Demo</span>
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
              <h1 className="display title">Shielded Night</h1>
              <p className="mono subtitle">A Zcash party during Breakpoint London</p>
              <ul className="facts">
                <li>
                  <PinIcon />
                  London
                </li>
                <li>
                  <CalendarIcon />
                  16 November 2026
                </li>
                <li>
                  <ClockIcon />
                  21:00
                </li>
              </ul>
              <p className="tagline">A night for people who like their payments private.</p>
              <p className="mono fine">
                Unofficial side event. Not affiliated with Breakpoint or the Solana Foundation.
              </p>
            </div>
            <aside className="details">
              <div>
                <p className="eyebrow">Ticket</p>
                <p className="value">General admission</p>
              </div>
              <div>
                <p className="eyebrow">Price</p>
                <p className="price">{snapshot ? zatoshiAmount(session?.priceZat ?? snapshot.priceZat) : '…'} ZEC</p>
                <p className="mono">(testnet)</p>
              </div>
              <div>
                <p className="eyebrow">Availability</p>
                <p className="value">{snapshot ? `${snapshot.available} of ${snapshot.stock} left` : '…'}</p>
              </div>
              {wallet && (
                <div>
                  <p className="eyebrow">Ticket wallet</p>
                  <p className="mono wallet-line">
                    <WalletIcon />
                    {short(wallet.address)}
                  </p>
                </div>
              )}
            </aside>
          </section>
          <div className="perforation" aria-hidden="true" />
          {done && session && (
            <Success
              vending
              cluster="devnet"
              view={{
                job: '',
                item: 'General admission',
                payout: session.payout,
                price_zats: Number(snapshot?.priceZat ?? 0n),
                stage: 'done',
                escrow: session.address,
                claim,
                txid_found_on_solana: null,
              }}
            />
          )}
          {!done && (
            <section className="stub-bottom checkout" aria-live="polite">
              <div className="get">
                <p className="eyebrow">Checkout</p>
                <h2 className="display">
                  {phase === 'reserving'
                    ? 'Reserving your ticket…'
                    : phase === 'proving'
                      ? 'Making your payment proof…'
                      : phase === 'claiming'
                        ? 'Sending your claim…'
                        : phase === 'claim_pending'
                          ? 'Checking your claim'
                          : phase === 'proof_failed'
                            ? 'Proof failed'
                            : phase === 'claim_failed'
                              ? 'Claim failed'
                              : session?.releaseRequested && !paid
                                ? 'Reservation expired'
                                : status === 'sold_out'
                                  ? 'Sold out'
                                  : status === 'expired'
                                    ? 'Reservation expired'
                                    : status === 'finality'
                                      ? 'Waiting for finality'
                                      : session
                                        ? 'Pay with Zcash'
                                        : 'Get your ticket'}
                </h2>
                {reason && (
                  <p className="mono bad" role="alert">
                    {reason}
                  </p>
                )}
                {!session && (status !== 'sold_out' || !!saved) && !wallet && (
                  <button
                    className="outline"
                    type="button"
                    onClick={async () => {
                      try {
                        setWallet(await connect());
                        setError('');
                      } catch (e) {
                        setError((e as Error).message);
                      }
                    }}
                  >
                    <WalletIcon />
                    Connect Solana wallet
                  </button>
                )}
                {!session && wallet && saved && (
                  <button className="outline" type="button" onClick={recover}>
                    Check reservation
                  </button>
                )}
                {!session && wallet && status !== 'sold_out' && (
                  <button
                    className="primary"
                    type="button"
                    disabled={status !== 'ready' || phase !== 'idle' || !!reason || !!saved?.claim}
                    onClick={reserve}
                  >
                    Reserve ticket
                  </button>
                )}
                {!session && <p className="mono small">You’ll need devnet SOL for rent and transaction fees.</p>}
                {link && (
                  <>
                    <p className="mono">Scan or open this request in your Zcash wallet.</p>
                    {qr?.link === link && <img className="payment-qr" src={qr.data} alt="Zcash payment QR code" />}
                    <p className="mono small payment-address">{settings.address}</p>
                    <p className="mono small">Keep the amount and memo unchanged. Pay once from shielded funds.</p>
                  </>
                )}
                {status === 'finality' && finality && (
                  <>
                    <p className="mono">
                      {finality.done} of {finality.of} blocks on top
                    </p>
                    <span
                      className="meter"
                      role="progressbar"
                      aria-label="Finality"
                      aria-valuenow={finality.done}
                      aria-valuemin={0}
                      aria-valuemax={finality.of}
                    >
                      <span style={{ width: `${(finality.done / finality.of) * 100}%` }} />
                    </span>
                  </>
                )}
                {phase === 'claim_pending' && (
                  <button className="outline" type="button" onClick={checkClaim}>
                    Check claim
                  </button>
                )}
                {phase === 'proving' && (
                  <p className="mono small">The proof service is checking your payment. Keep this page open.</p>
                )}
                {(phase === 'proof_failed' || phase === 'claim_failed') && (
                  <button className="primary" type="button" disabled={!!reason} onClick={runProof}>
                    Retry {phase === 'proof_failed' ? 'proof' : 'claim'}
                  </button>
                )}
                {session && (status === 'expired' || status === 'halted' || session.releaseRequested) && (
                  <>
                    <p className="mono small">
                      Release returns your Solana rent and the ticket to stock. It cannot refund a Zcash payment.
                    </p>
                    {releaseRemaining === null ? (
                      <button
                        className="outline"
                        type="button"
                        disabled={!requestable || releaseBusy}
                        onClick={requestRelease}
                      >
                        Request release
                      </button>
                    ) : (
                      <>
                        <p className="mono small">
                          A Claim can still mint your ticket until the reservation is released.
                        </p>
                        {releaseRemaining > 0 && (
                          <p className="mono small" role="timer">
                            Release in {countdown}
                          </p>
                        )}
                        <button
                          className="outline"
                          type="button"
                          disabled={
                            releaseRemaining > 0 ||
                            releaseBusy ||
                            phase === 'proving' ||
                            phase === 'claiming' ||
                            phase === 'claim_pending'
                          }
                          onClick={release}
                        >
                          Release
                        </button>
                      </>
                    )}
                    {releaseRemaining === null && !requestable && (
                      <p className="mono small">
                        Request release after the expiry plus Finality Depth, or 24 hours into a halt.
                      </p>
                    )}
                  </>
                )}
              </div>
              <div className="pay">
                <p className="eyebrow">Total</p>
                <p className="price">{snapshot ? zatoshiAmount(session?.priceZat ?? snapshot.priceZat) : '…'} ZEC</p>
                <p className="mono">(testnet)</p>
                {link ? (
                  <>
                    <a className="primary payment-open" href={link}>
                      Open in wallet
                    </a>
                    <button
                      className="outline"
                      type="button"
                      onClick={async () => {
                        try {
                          await navigator.clipboard.writeText(link);
                          setCopied(true);
                          setCopyError('');
                        } catch {
                          setCopyError('Could not copy. Select the payment link below.');
                        }
                      }}
                    >
                      {copied ? 'Copied' : 'Copy payment link'}
                    </button>
                    <input
                      className="payment-link mono small"
                      aria-label="Payment link"
                      readOnly
                      value={link}
                      onFocus={(e) => e.currentTarget.select()}
                    />
                  </>
                ) : (
                  !paid && (
                    <button className="primary" type="button" disabled>
                      <ShieldIcon />
                      Pay with Zcash
                    </button>
                  )
                )}
                {!paid && (
                  <>
                    <p className="mono small">Wallet fees are extra. Your wallet shows the fee before you send.</p>
                    <p className="mono small">
                      Use an Ironwood-capable testnet wallet. The shop’s incoming payments are public; your shielded
                      address stays private.
                    </p>
                    {session && <p className="mono small">Pay by Zcash height {session.expiryHeight}.</p>}
                  </>
                )}
                {copyError && link && (
                  <p className="mono bad" role="alert">
                    {copyError}
                  </p>
                )}
              </div>
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

export function VendingApp(props: Parameters<typeof VendingCheckout>[0]) {
  const binding = useMemo(() => {
    try {
      if (!props.config.vending) throw new Error('Vending configuration is missing.');
      return { chain: props.chain ?? createVendingChain(props.config.vending), error: '' };
    } catch (error) {
      return { chain: null, error: (error as Error).message };
    }
  }, [props.chain, props.config.vending]);
  if (!binding.chain)
    return (
      <div className="page">
        <header className="top">
          <Brand />
        </header>
        <main>
          <article className="stub">
            <section className="stub-bottom">
              <h1 className="display">Checkout is not ready</h1>
              <p className="mono bad" role="alert">
                {binding.error}
              </p>
            </section>
          </article>
        </main>
      </div>
    );
  return <VendingCheckout {...props} chain={binding.chain} />;
}
