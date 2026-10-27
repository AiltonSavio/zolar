import { describe, expect, it } from 'vitest';
import { checkedShop, DEFAULT_RESERVATION_BLOCKS } from './vending-admin.mjs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { fixtureAddress } from '../src/vendingAddress.test.ts';

describe('dedicated shop initialization guard', () => {
  it('starts the real Node CLI with package dependencies resolved', () => {
    const cli = fileURLToPath(new URL('../../scripts/vending-admin.sh', import.meta.url));
    const result = spawnSync(cli, ['--help'], { encoding: 'utf8' });
    expect(result.stderr).not.toContain('ERR_MODULE_NOT_FOUND');
    expect(result.status).toBe(0);
    expect(result.stdout).toContain('init-machine|reserve|inspect');
  });
  const receiver = Array(43).fill(7);
  const key = Buffer.alloc(64, 1);
  const fixture = {
    network: 'testnet',
    address: fixtureAddress(Uint8Array.from(receiver)),
    receiver,
    diversifier: Buffer.from(receiver.slice(0, 11)).toString('hex'),
    shop_viewing_key: key.toString('hex'),
    svk_hash: createHash('sha256').update(key).digest('hex'),
  };
  it('checks public address, receiver, diversifier and key hash before immutable init', () => {
    expect(checkedShop(fixture).key).toEqual(key);
    expect(DEFAULT_RESERVATION_BLOCKS).toBe(20);
    for (const mutation of [
      { network: 'mainnet' },
      { address: 'u1invalid' },
      { svk_hash: '00'.repeat(32) },
      { diversifier: '00'.repeat(11) },
      { receiver: Array(43).fill(8) },
      { shop_viewing_key: '00' },
    ])
      expect(() => checkedShop({ ...fixture, ...mutation })).toThrow();
  });
});
