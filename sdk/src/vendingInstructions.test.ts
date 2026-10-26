import { describe, expect, it } from 'vitest';
import { encodeVendingInstruction, decodeVendingRelay, canReleaseReservation } from './vendingInstructions';

const selectors = {
  reserve: [1],
  claim: [2],
  release: [3],
  claimFields: ['proof_len_u32', 'proof', 'public_values'] as const,
};
describe('replaceable vending instruction codec', () => {
  it('uses explicit deployment discriminators and exact binary argument order', () => {
    const nonce = new Uint8Array(32).fill(7);
    expect(encodeVendingInstruction(selectors, { kind: 'reserve', nonce })).toEqual(Uint8Array.from([1, ...nonce]));
    expect(encodeVendingInstruction(selectors, { kind: 'release' })).toEqual(new Uint8Array([3]));
    const publicValues = new Uint8Array(249).fill(9);
    expect(encodeVendingInstruction(selectors, { kind: 'claim', proof: new Uint8Array([4, 5]), publicValues })).toEqual(
      Uint8Array.from([2, 2, 0, 0, 0, 4, 5, ...publicValues]),
    );
  });
  it('validates relay owner, active state, testnet network, length, and finality depth', () => {
    const data = new Uint8Array(5914);
    data.set([1, 1, 1, 0]);
    data[95] = 1;
    data[96] = 10;
    const view = new DataView(data.buffer);
    view.setUint32(36, 110, true);
    view.setUint32(1050, 100, true);
    const owner = '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D';
    expect(decodeVendingRelay(data, owner, 123)).toEqual({
      tip: 110,
      finalHeight: 100,
      finalityDepth: 10,
      halted: false,
      updatedAt: 123,
    });
    expect(() => decodeVendingRelay(data, 'wrong', 123)).toThrow();
    data[95] = 0;
    expect(() => decodeVendingRelay(data, owner, 123)).toThrow();
    data[95] = 1;
    data[2] = 0;
    expect(() => decodeVendingRelay(data, owner, 123)).toThrow();
  });
  it('releases strictly after expiry plus depth or after the halted 24-hour backstop', () => {
    expect(
      canReleaseReservation({ tip: 130, finalityDepth: 10, halted: false }, { expiryHeight: 120, createdAt: 0 }, 0),
    ).toBe(false);
    expect(
      canReleaseReservation({ tip: 131, finalityDepth: 10, halted: false }, { expiryHeight: 120, createdAt: 0 }, 0),
    ).toBe(true);
    expect(
      canReleaseReservation(
        { tip: 100, finalityDepth: 10, halted: true },
        { expiryHeight: 120, createdAt: 10 },
        86_410,
      ),
    ).toBe(true);
  });
});

import { createVendingChain, validateVendingAbi } from './vendingInstructions';
import { fixtureAbi, fixtureAddress, fixtureMachine, fixtureProgram, mockVendingTransport } from './fixtures/vending';
const deployment = {
  machine: fixtureMachine,
  program: fixtureProgram,
  address: fixtureAddress,
  rpcUrl: 'http://fixture.invalid/rpc',
  abiUrl: 'http://fixture.invalid/abi.json',
};

describe('RPC and Wallet Standard transaction boundary', () => {
  it('reads the exact fixture accounts, confirms Reserve and Claim, and decodes a reserved price', async () => {
    const mock = mockVendingTransport('success');
    const chain = createVendingChain(deployment, mock.fetchImpl);
    expect((await chain.read()).relay.updatedAt).toBe(0);
    expect((await chain.read()).relay.updatedAt).toBeGreaterThan(0);
    const nonce = new Uint8Array(32).fill(9);
    const session = await chain.reserve(mock.wallet, nonce);
    expect(session.priceZat).toBe(100_000n);
    expect(session.expiryHeight).toBe(120);
    expect(session.payout).toBe(fixtureMachine);
    expect((await chain.restore(nonce))?.nonce).toEqual(nonce);
    const claim = await chain.claim(mock.wallet, session, {
      proof: new Uint8Array(356),
      publicValues: new Uint8Array(249),
    });
    expect(await chain.transactionStatus(claim)).toBe('confirmed');
    expect(await chain.restore(nonce)).toBeNull();
    expect(mock.sentCount()).toBe(2);
  });
  it('reuses the same reservation without signing a second Reserve', async () => {
    const mock = mockVendingTransport('payment');
    const chain = createVendingChain(deployment, mock.fetchImpl);
    await chain.read();
    await chain.read();
    const nonce = new Uint8Array(32).fill(9);
    await chain.reserve(mock.wallet, nonce);
    await chain.reserve(mock.wallet, nonce);
    expect(mock.sentCount()).toBe(1);
  });
  it('rejects missing pins, wrong deployment identity and malformed offsets', () => {
    expect(() => validateVendingAbi({ ...fixtureAbi, shopReceiver: [] }, deployment)).toThrow();
    expect(() => validateVendingAbi({ ...fixtureAbi, machineAddress: fixtureProgram }, deployment)).toThrow();
    expect(() =>
      validateVendingAbi({ ...fixtureAbi, machine: { ...fixtureAbi.machine, svk: 180 } }, deployment),
    ).toThrow();
  });
  it('rejects an address/viewing-key association that differs in pk_d with the same diversifier', async () => {
    const mock = mockVendingTransport('payment');
    const changed = structuredClone(fixtureAbi);
    changed.shopReceiver[42] = 42;
    const chain = createVendingChain(deployment, async (input, init) =>
      String(input).endsWith('/abi.json') ? new Response(JSON.stringify(changed)) : mock.fetchImpl(input, init),
    );
    await expect(chain.read()).rejects.toThrow(/trusted receiver/);
  });
});
