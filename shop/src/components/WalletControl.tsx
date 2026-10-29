import { useState } from 'react';
import { looksLikeAddress, type useSolanaConnection } from '../solanaWallet';
import { short } from '../config';
import { Dialog } from './Dialog';
import { ArrowIcon, ChevronIcon, WalletIcon } from './Icons';

export function WalletControl({ connection }: { connection: ReturnType<typeof useSolanaConnection> }) {
  const [open, setOpen] = useState(false);
  const [addressMode, setAddressMode] = useState(false);
  const [draft, setDraft] = useState('');
  const { selection, wallets, connecting, error } = connection;
  function show() { connection.clearError(); setAddressMode(false); setDraft(selection?.address ?? ''); setOpen(true); }
  return (
    <>
      <button type="button" className={`wallet-trigger ${selection ? 'wallet-connected' : ''}`} aria-haspopup="dialog" aria-label={selection ? `Manage wallet ${selection.address}` : 'Connect wallet'} aria-busy={!!connecting} onClick={show}>
        <span className="wallet-avatar">{selection?.wallet ? <img src={selection.wallet.icon} alt="" /> : <WalletIcon />}</span>
        <span className="wallet-trigger-text"><strong>{selection ? short(selection.address) : connecting ? 'Connecting…' : 'Connect wallet'}</strong><span className="mono small">{selection ? selection.wallet?.name ?? 'Receiving address' : 'Choose your Solana wallet'}</span></span>
        {selection && <span className="connected-dot" aria-label={selection.wallet ? 'Connected' : 'Address selected'} />}
        <ChevronIcon />
      </button>
      <p className="mono small muted wallet-help">Your ticket goes here. Connecting only shares your address.</p>
      <Dialog open={open} onClose={() => setOpen(false)} title={addressMode ? 'Use a Solana address' : 'Connect your wallet'} eyebrow="Ticket wallet">
        {addressMode ? (
          <form className="address-form" onSubmit={(e) => { e.preventDefault(); if (connection.useAddress(draft)) setOpen(false); }}>
            <p className="mono small muted">Paste the address that should receive your ticket.</p>
            <label className="field"><span className="mono small">Solana address</span><span className="input"><WalletIcon /><input aria-label="Solana address" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Your receiving address" autoComplete="off" spellCheck={false} /></span></label>
            {draft && !looksLikeAddress(draft) && <p className="mono small bad">That is not a Solana address.</p>}
            <button className="primary" type="submit" disabled={!!connecting || !looksLikeAddress(draft)}>Use this address <ArrowIcon /></button>
            <button className="text-button" type="button" onClick={() => setAddressMode(false)}>Back to wallets</button>
          </form>
        ) : (
          <>
            {selection && <section className="selected-wallet"><span className="eyebrow">{selection.wallet ? 'Connected' : 'Receiving address'}</span><p className="mono full-address">{selection.address}</p><button className="text-button" type="button" disabled={!!connecting} onClick={async () => { if (await connection.disconnect()) setOpen(false); }}>Disconnect wallet</button></section>}
            <p className="mono small muted">{wallets.length ? selection ? 'Switch to another wallet below.' : 'Choose an installed wallet to receive your ticket.' : 'No Solana wallet detected. Open your wallet extension or use a receiving address.'}</p>
            <div className="wallet-options">
              {wallets.map((wallet) => <button className="wallet-option" type="button" key={wallet.name} disabled={!!connecting} onClick={async () => { if (await connection.connect(wallet)) setOpen(false); }}>
                <img className="wallet-option-icon" src={wallet.icon} alt="" />
                <span><strong>{wallet.name}</strong><span className="mono small muted">{connecting === wallet.name ? 'Waiting for your wallet…' : wallet === selection?.wallet ? 'Connected' : 'Detected'}</span></span><ChevronIcon />
              </button>)}
            </div>
            <button className="address-option" type="button" disabled={!!connecting} onClick={() => setAddressMode(true)}><WalletIcon /><span>Use a Solana address</span><ArrowIcon /></button>
          </>
        )}
        {error && <p role="alert" className="mono small bad modal-error">{error}</p>}
        <p className="modal-footnote mono">No signature or Solana payment is requested.</p>
      </Dialog>
    </>
  );
}
