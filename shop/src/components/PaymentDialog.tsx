import type { OrderTerms } from '@zolar/sdk';
import { Dialog } from './Dialog';
import { ArrowIcon, ShieldIcon, WalletIcon } from './Icons';
import { zec } from '../config';

export function PaymentDialog({ open, onClose, order, item, price, payout, network, approvalUrl, onOpenWallet, opened, blocked }: {
  open: boolean; onClose: () => void; order: OrderTerms | null; item: string; price: number; payout: string; network: string;
  approvalUrl: string | null; onOpenWallet: () => void; opened: boolean; blocked: boolean;
}) {
  return (
    <Dialog className="payment-modal" open={open} onClose={onClose} title="Pay with Zcash" eyebrow="Private checkout">
      <div className="payment-total"><span className="payment-shield"><ShieldIcon /></span><div><p className="eyebrow">You pay</p><p className="price">{zec(order?.item.price_zats ?? price)} ZEC</p><p className="mono small muted">Shielded · Zcash {order?.network ?? network}</p></div></div>
      <dl className="payment-terms">
        <div><dt>You receive</dt><dd>{order?.item.amount ?? 1} × {order?.item.name ?? item}</dd></div>
        <div><dt>Payee</dt><dd>{order?.merchant ?? 'Shielded Night'}</dd></div>
        <div className="payment-wallet"><dt>Ticket wallet</dt><dd className="mono full-address"><WalletIcon />{order?.payout ?? payout}</dd></div>
      </dl>
      <div className="wallet-handoff" role="status"><span className="handoff-dot" /><div><strong>{!approvalUrl ? 'Preparing your checkout…' : blocked ? 'Authorization window blocked' : opened ? 'Authorize Zkool payment' : 'Ready for your approval'}</strong><p className="mono small muted">{!approvalUrl ? 'Getting the payment terms from the Payee.' : opened ? 'Authorize or reject through your local Zkool connection. This page updates automatically.' : 'Review the authorization for your connected Zkool wallet.'}</p></div></div>
      <button className="primary dialog-primary" type="button" disabled={!approvalUrl} onClick={onOpenWallet}>{opened ? 'Return to payment authorization' : 'Authorize Zkool payment'}<ArrowIcon /></button>
      {blocked && approvalUrl && <a className="approval-fallback mono small" href={approvalUrl} target="_blank" rel="noreferrer">Open payment authorization</a>}
      <p className="modal-footnote mono">{approvalUrl ? <>Approval: <span className="approval-origin">{new URL(approvalUrl).origin}</span>.</> : 'Your connection asks for approval before requesting a wallet signature.'} Your proof is made locally; your Zcash transaction ID stays off Solana.</p>
    </Dialog>
  );
}
