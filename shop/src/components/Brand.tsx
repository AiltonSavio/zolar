/** The Zolar mark and name. The wordmark is text until its artwork is chosen. */
export function Brand({ size = 'large' }: { size?: 'large' | 'small' }) {
  return (
    <span className={`brand brand-${size}`}>
      <span className="brand-word">zolar</span>
      <img className="brand-mark" src="/zolar-mark.png" alt="" aria-hidden="true" />
    </span>
  );
}
