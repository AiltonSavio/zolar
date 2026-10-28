import { bech32m } from '@scure/base';
import { blake2b } from '@noble/hashes/blake2b';

/** ZIP-316 revision 0, testnet, exactly one type-3 receiver. ZIP-229 keeps this address format. */
export function ironwoodReceiver(address: string): Uint8Array {
  if (!address.startsWith('utest1') || address.length > 512)
    throw new Error('A testnet Ironwood-only unified address is required');
  const decoded = bech32m.decode(address as `${string}1${string}`, 512);
  if (decoded.prefix !== 'utest') throw new Error('Wrong address network');
  const bytes = new Uint8Array(bech32m.fromWords(decoded.words));
  // One receiver: type(1) + length(1) + receiver(43) + HRP padding(16).
  if (bytes.length !== 61) throw new Error('The shop address must have exactly one Ironwood receiver');
  const left = bytes.subarray(0, 30);
  const right = bytes.subarray(30);
  const text = new TextEncoder();
  function xor(target: Uint8Array, mask: Uint8Array) {
    target.forEach((value, i) => {
      target[i] = value ^ mask[i]!;
    });
  }
  const h = (round: number) =>
    blake2b(right, {
      dkLen: left.length,
      personalization: Uint8Array.from([...text.encode('UA_F4Jumble_H'), round, 0, 0]),
    });
  const g = (round: number) =>
    blake2b(left, {
      dkLen: 64,
      personalization: Uint8Array.from([...text.encode('UA_F4Jumble_G'), round, 0, 0]),
    }).subarray(0, right.length);
  xor(left, h(1));
  xor(right, g(1));
  xor(left, h(0));
  xor(right, g(0));
  const padding = Uint8Array.from([...text.encode('utest'), ...new Uint8Array(11)]);
  if (bytes[0] !== 3 || bytes[1] !== 43 || !padding.every((b, i) => bytes[45 + i] === b))
    throw new Error('Invalid single-receiver unified address');
  return bytes.slice(2, 45);
}
