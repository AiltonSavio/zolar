export function shorten(s: string, head = 4, tail = 4): string {
  return s.length <= head + tail + 1 ? s : `${s.slice(0, head)}…${s.slice(-tail)}`;
}

export function ago(nowMs: number, thenMs: number): string {
  const s = Math.max(0, Math.round((nowMs - thenMs) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s ago`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m ago`;
}

export function formatWork(work: bigint): string {
  const s = work.toString();
  if (s.length <= 9) return Number(work).toLocaleString('en-US');
  const exp = s.length - 1;
  return `${s[0]}.${s.slice(1, 4)}e${exp}`;
}
