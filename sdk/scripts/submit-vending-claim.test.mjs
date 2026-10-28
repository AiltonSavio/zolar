import { describe, expect, it, vi } from 'vitest';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Keypair, PublicKey } from '@solana/web3.js';
import { submitClaim } from './submit-vending-claim.mjs';

describe('local Claim submission journal', () => {
  it('persists the signed identity before a broadcast timeout and refuses a second attempt', async () => {
    const directory = mkdtempSync(join(tmpdir(), 'vending-claim-'));
    const out = join(directory, 'attempt.json');
    const signer = Keypair.generate();
    const pub = (n) => new PublicKey(Buffer.alloc(32, n)).toBase58();
    const result = { mode: 'groth16', proof: 'ab'.repeat(356), public_values: '00'.repeat(249) };
    const before = {
      program: pub(1),
      machine: pub(2),
      reservation: pub(3),
      relay: pub(4),
      payout: pub(5),
      submitter: signer.publicKey.toBase58(),
      mint: pub(6),
      tag: pub(7),
      mintAuthority: pub(8),
      payoutToken: pub(9),
      resultSha256: createHash('sha256').update(JSON.stringify(result)).digest('hex'),
    };
    const rpc = {
      getGenesisHash: async () => 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG',
      getLatestBlockhash: async () => ({ blockhash: pub(10), lastValidBlockHeight: 50 }),
      simulateTransaction: async () => ({ value: { err: null } }),
      sendRawTransaction: vi.fn(async () => {
        expect(JSON.parse(readFileSync(out, 'utf8')).status).toBe('signed');
        throw new Error('timeout');
      }),
    };
    try {
      await expect(submitClaim(rpc, signer, before, result, out)).rejects.toThrow('timeout');
      expect(JSON.parse(readFileSync(out, 'utf8')).signature).toBeTruthy();
      await expect(submitClaim(rpc, signer, before, result, out)).rejects.toThrow('EEXIST');
      expect(rpc.sendRawTransaction).toHaveBeenCalledTimes(1);
      await expect(
        submitClaim({ ...rpc, getGenesisHash: async () => 'mainnet' }, signer, before, result, out),
      ).rejects.toThrow('devnet');
      await expect(submitClaim(rpc, signer, before, { ...result, proof: 'cd'.repeat(356) }, out)).rejects.toThrow(
        'Proof differs',
      );
      writeFileSync(out, 'preserved');
      await expect(submitClaim(rpc, signer, before, result, out)).rejects.toThrow('EEXIST');
      expect(readFileSync(out, 'utf8')).toBe('preserved');
    } finally {
      rmSync(directory, { recursive: true });
    }
  });
});
