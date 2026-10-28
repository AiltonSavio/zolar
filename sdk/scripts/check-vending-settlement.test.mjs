import { describe, expect, it } from 'vitest';
import { PublicKey, Transaction } from '@solana/web3.js';
import { base58 } from '@scure/base';
import {
  claimInstruction,
  requireProof,
  verifyTransaction,
  requireReplayRejection,
} from './check-vending-settlement.mjs';

const pub = (n) => new PublicKey(Buffer.alloc(32, n)).toBase58();
const before = {
  program: pub(1),
  machine: pub(2),
  reservation: pub(3),
  relay: pub(4),
  payout: pub(5),
  submitter: pub(5),
  mint: pub(6),
  tag: pub(7),
  mintAuthority: pub(8),
  payoutToken: pub(9),
  slot: 10,
};
const result = { mode: 'groth16', proof: 'ab'.repeat(356), public_values: '00'.repeat(249) };
function receipt() {
  const ix = claimInstruction(before, result);
  const tx = new Transaction({ feePayer: new PublicKey(before.submitter), recentBlockhash: pub(10) }).add(ix);
  const message = tx.compileMessage();
  const accountIndex = message.accountKeys.findIndex((p) => p.toBase58() === before.payoutToken);
  const token = (amount) => ({
    accountIndex,
    mint: before.mint,
    owner: before.payout,
    uiTokenAmount: { amount, decimals: 0 },
  });
  return {
    slot: 20,
    transaction: { message, signatures: ['signature'] },
    meta: {
      err: null,
      computeUnitsConsumed: 123,
      fee: 5000,
      preTokenBalances: [token('4')],
      postTokenBalances: [token('5')],
    },
  };
}
describe('vending settlement evidence', () => {
  it('rejects execute-only, empty, odd-length and malformed proof output', () => {
    for (const bad of [{ mode: 'execute', proof: null }, { proof: '' }, { proof: 'a' }, { proof: 'zz'.repeat(356) }])
      expect(() => requireProof({ ...result, ...bad })).toThrow();
    expect(() => requireProof(result)).not.toThrow();
  });
  it('accepts a matching Claim with a transaction-local increase of exactly one ticket', () => {
    expect(verifyTransaction(receipt(), before, result, 'signature').tickets).toBe('1');
  });
  it('rejects an unrelated, failed, old or redirected transaction', () => {
    const tx = receipt();
    for (const bad of [null, { ...tx, slot: 5 }, { ...tx, meta: { ...tx.meta, err: 'failed' } }])
      expect(() => verifyTransaction(bad, before, result, 'signature')).toThrow();
    expect(() => verifyTransaction(tx, before, result, 'other-signature')).toThrow();
    expect(() => verifyTransaction(tx, { ...before, payout: pub(11) }, result, 'signature')).toThrow();
    expect(() => verifyTransaction(tx, before, { ...result, proof: 'cd'.repeat(356) }, 'signature')).toThrow();
  });
  it('rejects zero or multiple tickets and missing token metadata', () => {
    for (const amount of ['4', '6']) {
      const tx = receipt();
      tx.meta.postTokenBalances[0].uiTokenAmount.amount = amount;
      expect(() => verifyTransaction(tx, before, result, 'signature')).toThrow();
    }
    const tx = receipt();
    delete tx.meta.postTokenBalances;
    expect(() => verifyTransaction(tx, before, result, 'signature')).toThrow();
  });
  it('does not treat arbitrary simulation failure as replay prevention', () => {
    const logs = [
      `Program ${before.program} invoke [1]`,
      `Program ${before.program} failed: custom program error: 0x10`,
    ];
    expect(() =>
      requireReplayRejection({ err: { InstructionError: [0, { Custom: 16 }] }, logs }, before.program),
    ).not.toThrow();
    for (const bad of [
      { err: 'BlockhashNotFound', logs },
      { err: null, logs },
      { err: { InstructionError: [0, { Custom: 4 }] }, logs },
      { err: { InstructionError: [0, { Custom: 16 }] }, logs: [] },
    ])
      expect(() => requireReplayRejection(bad, before.program)).toThrow();
  });
  it('decodes the generated Claim without adding an extra public-values prefix', () => {
    const ix = claimInstruction(before, result);
    expect(ix.data.length).toBe(1 + 2 + 356 + 249);
    expect(base58.encode(ix.data).length).toBeGreaterThan(0);
  });
});
