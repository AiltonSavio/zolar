import { useState } from 'react';
import type { WalletStatus } from '@zolar/sdk';
import { Dialog } from './Dialog';
import { ShieldIcon, ChevronIcon } from './Icons';
import { short } from '../config';

export function supportedZcashWallet(status: WalletStatus | null | undefined): status is WalletStatus & { address: string; account: number } {
  return status?.wallet === 'Zkool' && status.network === 'testnet'
    && typeof status.address === 'string' && status.address.startsWith('utest1')
    && Number.isInteger(status.account) && (status.account ?? 0) > 0;
}

export function ZcashWalletControl({ status, selection, onConnect, onDisconnect }: {
  status: WalletStatus | null | undefined; selection: WalletStatus | null;
  onConnect: () => Promise<void>; onDisconnect: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return <>
    <button type="button" className={`wallet-trigger ${selection ? 'wallet-connected' : ''}`}
      aria-haspopup="dialog" aria-label={selection ? 'Manage Zcash wallet' : 'Connect Zcash wallet'} onClick={() => setOpen(true)}>
      <span className="wallet-avatar"><ShieldIcon /></span>
      <span className="wallet-trigger-text"><strong>{selection?.address ? short(selection.address) : 'Connect Zcash wallet'}</strong>
        <span className="mono small">{selection ? `${selection.wallet} · Zcash testnet` : 'Choose the wallet that pays'}</span></span><ChevronIcon />
    </button>
    <p className="mono small muted wallet-help">ZEC is signed by your connected Zcash wallet.</p>
    <Dialog open={open} onClose={() => setOpen(false)} title="Connect Zcash wallet" eyebrow="Payment wallet">
      {selection && <section className="selected-wallet"><strong>{selection.wallet}</strong><p className="mono full-address">{selection.address}</p>
        <button type="button" className="text-button" onClick={() => { onDisconnect(); setOpen(false); }}>Disconnect Zcash wallet</button></section>}
      {supportedZcashWallet(status) ? <>
        <p className="mono small">Connect to your existing Zkool testnet wallet service. Its spending keys stay in Zkool.</p>
        <button type="button" className="wallet-option" disabled={busy} onClick={async () => {
          setBusy(true); setError(null);
          try { await onConnect(); setOpen(false); } catch (e) { setError((e as Error).message); }
          finally { setBusy(false); }
        }}><ShieldIcon /><span><strong>{busy ? 'Connecting…' : 'Connect Zkool'}</strong><span className="mono small muted">Account {status.account} · {short(status.address)}</span></span><ChevronIcon /></button>
      </> : <p className="mono small">No supported Zcash testnet wallet is connected. Run the local connector with your existing Zkool wallet service to continue.</p>}
      {error && <p role="alert" className="mono small bad">{error}</p>}
      <p className="modal-footnote mono">This shop can discover the configured account while the connection is running. Payment needs separate authorization. Never enter a seed phrase into this shop.</p>
    </Dialog>
  </>;
}
