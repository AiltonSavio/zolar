import type { ReadonlyUint8Array } from '@solana/kit';
export function toHex(bytes: ReadonlyUint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** Zcash shows hashes in the reverse of their internal byte order. */
export function toExplorerHash(internal: ReadonlyUint8Array): string {
  return toHex(Uint8Array.from(internal).reverse());
}
