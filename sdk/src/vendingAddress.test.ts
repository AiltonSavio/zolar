import { describe, expect, it } from 'vitest';
import { bech32m } from '@scure/base';
import { blake2b } from '@noble/hashes/blake2b';
import { ironwoodReceiver } from './vendingAddress.js';

// Encoder is independent of the production inverse; fixture is an address, never a key.
export function fixtureAddress(receiver = new Uint8Array(43).fill(7), extra = new Uint8Array()): string {
  const plain = Uint8Array.from([
    3,
    43,
    ...receiver,
    ...extra,
    ...new TextEncoder().encode('utest'),
    ...new Uint8Array(11),
  ]);
  const left = plain.slice(0, Math.min(64, Math.floor(plain.length / 2)));
  const right = plain.slice(left.length);
  function xor(target: Uint8Array, mask: Uint8Array) {
    target.forEach((v, i) => {
      target[i] = v ^ mask[i]!;
    });
  }
  function h(i: number) {
    return blake2b(right, {
      dkLen: left.length,
      personalization: Uint8Array.from([...new TextEncoder().encode('UA_F4Jumble_H'), i, 0, 0]),
    });
  }
  function g(i: number) {
    const output = new Uint8Array(right.length);
    for (let j = 0; j * 64 < right.length; j++)
      output.set(
        blake2b(left, {
          dkLen: 64,
          personalization: Uint8Array.from([...new TextEncoder().encode('UA_F4Jumble_G'), i, j & 255, j >> 8]),
        }).slice(0, right.length - j * 64),
        j * 64,
      );
    return output;
  }
  xor(right, g(0));
  xor(left, h(0));
  xor(right, g(1));
  xor(left, h(1));
  return bech32m.encode('utest', bech32m.toWords(Uint8Array.from([...left, ...right])), false);
}
describe('Ironwood-only address', () => {
  it('decodes the only Orchard receiver (Ironwood uses its address structure)', () => {
    expect(ironwoodReceiver(fixtureAddress())).toEqual(new Uint8Array(43).fill(7));
  });
  it('rejects bad checksums, mainnet, and a second receiver', () => {
    const address = fixtureAddress();
    expect(() => ironwoodReceiver(address.slice(0, -1) + 'x')).toThrow();
    expect(() => ironwoodReceiver('u1abc')).toThrow();
    expect(() => ironwoodReceiver(fixtureAddress(new Uint8Array(43), new Uint8Array([4, 1, 0])))).toThrow();
  });
});
