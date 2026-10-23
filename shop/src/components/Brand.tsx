/** The Zolar wordmark and mark. */
export function Brand({ size = 'large' }: { size?: 'large' | 'small' }) {
  return (
    <span className={`brand brand-${size}`}>
      <img className="brand-word" src="/zolar-wordmark.png" alt="zolar" />
      <img className="brand-mark" src="/zolar-mark.png" alt="" aria-hidden="true" />
    </span>
  );
}
