import { address } from '@solana/kit';
import { getRelayEncoder } from '@relay-client';

export const bytes = (fill: number, n = 32) => new Uint8Array(n).fill(fill);

export function relayAccount(over: Record<string, unknown> = {}) {
  const roots = Array.from({ length: 128 }, (_, i) => bytes(i + 1));
  const data = getRelayEncoder().encode({
    version: 1,
    state: 1,
    halted: 0,
    creator: address('7zr1j185zT8r6jcysC4LyrdSEJ13Up3xq4TFu6KQqh7M'),
    tipHeight: 4_517_300,
    tipSlot: 5,
    tipSeq: 9,
    tipWork: 123_456_789_012_345_678_901n,
    tipHash: Uint8Array.from({ length: 32 }, (_, i) => i),
    checkpoint: {
      version: 1, network: 1, finalityDepth: 10, nu7Height: 4_465_026, height: 4_517_091,
      nonce: 1n, hash: bytes(0), times: Array(113).fill(0), bits: Array(113).fill(0),
    },
    finalizedHeight: 4_517_290,
    finalizedHash: bytes(7),
    leafCount: 199,
    haltHeight: 0,
    haltHash: bytes(0),
    reserved: new Uint8Array(52),
    filledSubtrees: Array.from({ length: 20 }, () => bytes(0)),
    roots,
    ...over,
  });
  return data;
}

