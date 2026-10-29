// Inline stroke icons (decorative; the text next to them carries the meaning).
const base = { width: 20, height: 20, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const, 'aria-hidden': true };

export const PinIcon = () => (
  <svg {...base}><path d="M12 21s-7-6.2-7-11.5A7 7 0 0 1 19 9.5C19 14.8 12 21 12 21z" /><circle cx="12" cy="9.5" r="2.5" /></svg>
);
export const CalendarIcon = () => (
  <svg {...base}><rect x="3.5" y="5" width="17" height="15" rx="2" /><path d="M3.5 10h17M8 3v4M16 3v4" /></svg>
);
export const ClockIcon = () => (
  <svg {...base}><circle cx="12" cy="12" r="8.5" /><path d="M12 7.5V12l3 2" /></svg>
);
export const WalletIcon = () => (
  <svg {...base}><path d="M4 7.5A2.5 2.5 0 0 1 6.5 5H18v4" /><rect x="4" y="7.5" width="16.5" height="11.5" rx="2" /><path d="M20.5 11h-4a1.75 1.75 0 0 0 0 3.5h4z" /></svg>
);
export const ShieldIcon = () => (
  <svg {...base}><path d="M12 3.5 5 6v5.5c0 4.2 3 7.6 7 9 4-1.4 7-4.8 7-9V6z" /><path d="m9 12 2 2 4-4" /></svg>
);
export const LinkIcon = () => (
  <svg {...base} width={18} height={18}><path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" /></svg>
);
export const CheckIcon = () => (
  <svg {...base} width={18} height={18} strokeWidth={3}><path d="m5 12.5 4.5 4.5L19 7.5" /></svg>
);
export const CloseIcon = () => <svg {...base}><path d="m6 6 12 12M18 6 6 18" /></svg>;
export const ArrowIcon = () => <svg {...base}><path d="M5 12h14m-5-5 5 5-5 5" /></svg>;
export const ChevronIcon = () => <svg {...base}><path d="m9 5 7 7-7 7" /></svg>;
