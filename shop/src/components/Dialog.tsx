import { useEffect, useId, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { CloseIcon } from './Icons';

/** Native top-layer dialog: focus stays inside and the rest of the page is inert. */
export function Dialog({ open, onClose, title, eyebrow, children, className = '' }: { className?: string; open: boolean; onClose: () => void; title: string; eyebrow: string; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      dialog.querySelector<HTMLElement>('[data-dialog-title]')?.focus({ preventScroll: true });
    } else if (!open && dialog.open) dialog.close();
    if (!open) return;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = overflow; };
  }, [open]);
  return createPortal(
    <dialog ref={ref} className={`modal ${className}`} aria-labelledby={titleId} aria-modal="true" onCancel={(e) => { e.preventDefault(); onClose(); }} onKeyDown={(e) => {
      if (e.key !== 'Tab') return;
      const controls = [...e.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])')].filter((x) => x.getClientRects().length > 0);
      const first = controls[0]; const last = controls.at(-1);
      if (!first || !last) return;
      const active = document.activeElement;
      if (e.shiftKey && (active === first || active?.hasAttribute('data-dialog-title'))) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && active === last) { e.preventDefault(); first.focus(); }
    }} onClick={(e) => {
      if (e.target !== e.currentTarget) return;
      const box = e.currentTarget.getBoundingClientRect();
      if (e.clientX < box.left || e.clientX > box.right || e.clientY < box.top || e.clientY > box.bottom) onClose();
    }}>
      <header className="modal-head">
        <div><p className="eyebrow">{eyebrow}</p><h2 id={titleId} className="display" tabIndex={-1} data-dialog-title>{title}</h2></div>
        <button className="icon-button" type="button" aria-label="Close dialog" onClick={onClose}><CloseIcon /></button>
      </header>
      <div className="modal-body">{children}</div>
    </dialog>, document.body,
  );
}
