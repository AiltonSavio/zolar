import { describe, expect, it } from 'vitest';
import { getBase58Decoder } from '@solana/kit';
import { classifyTransaction, type RpcTransaction } from './claims';

const ESCROW_PROGRAM = 'HMew6N4iWjmWMbJBoK6WfQRucBt8ZfWpMadd9mpMtbML';
const ESCROW = 'Es1111111111111111111111111111111111111111';
const MINT = '6hcTJNrnzSfL3QNj3uQRsFuC9WHAD48ejMov7Nw7ZyqN';

const b58 = (bytes: number[]) => getBase58Decoder().decode(Uint8Array.from(bytes));

function tx(tag: number, extra: number[] = [], err: unknown = null): RpcTransaction {
  return {
    slot: 100,
    blockTime: 1_760_000_000,
    transaction: {
      signatures: ['SigAAA'],
      message: {
        accountKeys: ['Payer', ESCROW, 'Relay', ESCROW_PROGRAM],
        instructions: [{ programIdIndex: 3, accounts: tag === 0 || tag === 2 ? [0, 1, 2] : [1, 2], data: b58([tag, ...extra]) }],
      },
    },
    meta: {
      err,
      preTokenBalances: [
        { owner: ESCROW, mint: MINT, uiTokenAmount: { amount: '2500000' } },
        { owner: 'Other', mint: MINT, uiTokenAmount: { amount: '7' } },
      ],
    },
  };
}

describe('classifyTransaction', () => {
  it.each([
    [0, 'created'],
    [1, 'reveal-claim'],
    [2, 'bond-deposited'],
    [3, 'refund'],
    [4, 'private-claim'],
    [5, 'refund-requested'],
  ])('instruction tag %s is %s', (tag, kind) => {
    expect(classifyTransaction(tx(tag), ESCROW_PROGRAM)).toMatchObject([{ kind, signature: 'SigAAA', escrow: ESCROW }]);
  });

  it('carries the amount the Escrow held and its mint', () => {
    expect(classifyTransaction(tx(1), ESCROW_PROGRAM)[0]).toMatchObject({ amount: 2_500_000n, mint: MINT });
  });

  it('marks a failed transaction', () => {
    expect(classifyTransaction(tx(1, [], { InstructionError: [0, 'Custom'] }), ESCROW_PROGRAM)[0]?.failed).toBe(true);
  });

  it('ignores transactions that do not call the Escrow program', () => {
    const other = tx(1);
    other.transaction.message.instructions[0]!.programIdIndex = 0;
    expect(classifyTransaction(other, ESCROW_PROGRAM)).toEqual([]);
  });

  it('never carries anything from the Reveal Claim payload (the revealed txid)', () => {
    const txidBytes = Array.from({ length: 32 }, (_, i) => 200 - i);
    const raw = tx(1, txidBytes);
    const events = classifyTransaction(raw, ESCROW_PROGRAM);
    const dump = JSON.stringify(events, (_, v) => (typeof v === 'bigint' ? v.toString() : v));
    const data = raw.transaction.message.instructions[0]!.data;
    expect(dump).not.toContain(data);
    expect(dump).not.toContain(data.slice(4, 24));
    expect(Object.keys(events[0] ?? {}).sort()).toEqual(
      ['amount', 'blockTime', 'escrow', 'failed', 'index', 'kind', 'mint', 'signature', 'slot'].sort(),
    );
  });

  it('finds the Escrow when its address comes from an address lookup table', () => {
    const raw = tx(4);
    raw.transaction.message.accountKeys = ['Payer', 'Relay', ESCROW_PROGRAM];
    raw.transaction.message.instructions[0] = { programIdIndex: 2, accounts: [3, 1], data: b58([4]) };
    raw.meta!.loadedAddresses = { writable: [ESCROW], readonly: [] };
    expect(classifyTransaction(raw, ESCROW_PROGRAM)).toMatchObject([{ kind: 'private-claim', escrow: ESCROW }]);
  });

  it('reports every Escrow instruction of a transaction, in order', () => {
    const raw = tx(2);
    raw.transaction.message.instructions = [
      { programIdIndex: 3, accounts: [0, 1], data: b58([2]) },
      { programIdIndex: 3, accounts: [1, 2], data: b58([4]) },
    ];
    const events = classifyTransaction(raw, ESCROW_PROGRAM);
    expect(events.map((e) => [e.kind, e.index])).toEqual([['bond-deposited', 0], ['private-claim', 1]]);
  });
});
