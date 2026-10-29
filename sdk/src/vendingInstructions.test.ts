import {
  getMachineDecoder,
  getReservationDecoder,
  VENDING_ERROR__WRONG_PAYOUT,
  VENDING_ERROR__RELEASE_ALREADY_REQUESTED,
  VENDING_ERROR__RELEASE_NOT_REQUESTED,
  VENDING_ERROR__CLAIM_WINDOW_OPEN,
} from '@zolar/vending-client';
import { Buffer } from 'buffer';
import { readFileSync } from 'node:fs';
import { PublicKey } from '@solana/web3.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  encodeVendingInstruction,
  decodeVendingRelay,
  canRequestRelease,
  releaseSecondsRemaining,
} from './vendingInstructions.js';

afterEach(() => vi.restoreAllMocks());

describe('program vending instruction codec', () => {
  it('matches the program discriminators and u16 proof length', () => {
    const nonce = new Uint8Array(32).fill(7);
    expect(encodeVendingInstruction({ kind: 'reserve', nonce })).toEqual(Uint8Array.from([1, ...nonce]));
    expect(encodeVendingInstruction({ kind: 'requestRelease' })).toEqual(new Uint8Array([4]));
    expect(encodeVendingInstruction({ kind: 'release' })).toEqual(new Uint8Array([3]));
    const publicValues = new Uint8Array(249).fill(9);
    expect(encodeVendingInstruction({ kind: 'claim', proof: new Uint8Array(388), publicValues })).toEqual(
      Uint8Array.from([2, 132, 1, ...new Uint8Array(388), ...publicValues]),
    );
    expect(() => encodeVendingInstruction({ kind: 'reserve', nonce: new Uint8Array(31) })).toThrow();
    expect(() =>
      encodeVendingInstruction({ kind: 'claim', proof: new Uint8Array(1), publicValues: new Uint8Array(248) }),
    ).toThrow();
  });
  it('keeps the generated account sizes and program error codes', () => {
    expect(getMachineDecoder().fixedSize).toBe(291);
    expect(getReservationDecoder().fixedSize).toBe(128);
    expect([
      VENDING_ERROR__WRONG_PAYOUT,
      VENDING_ERROR__RELEASE_ALREADY_REQUESTED,
      VENDING_ERROR__RELEASE_NOT_REQUESTED,
      VENDING_ERROR__CLAIM_WINDOW_OPEN,
    ]).toEqual([21, 22, 23, 24]);
  });
  it('decodes the program-produced Machine used by the prover with the full Shop Viewing Key at 227', () => {
    const data = new Uint8Array(
      readFileSync(new URL('../../crates/vending-prover/tests/fixtures/machine-revision2.bin', import.meta.url)),
    );
    const witness = JSON.parse(
      readFileSync(new URL('../../crates/vending-guest/fixtures/testnet/witness.json', import.meta.url), 'utf8'),
    );
    const machine = getMachineDecoder().decode(data);
    expect(data.length).toBe(291);
    expect(machine.svk).toEqual(new Uint8Array(witness.shop_viewing_key));
    expect(machine.svk).toEqual(data.slice(227, 291));
    expect(machine.svkHash).toEqual(data.slice(159, 191));
    expect(machine.diversifier).toEqual(new Uint8Array(witness.diversifier));
    expect(machine.priceZat).toBe(100_000n);
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
      canRequestRelease({ tip: 130, finalityDepth: 10, halted: false }, { expiryHeight: 120, createdAt: 0 }, 0),
    ).toBe(false);
    expect(
      canRequestRelease({ tip: 131, finalityDepth: 10, halted: false }, { expiryHeight: 120, createdAt: 0 }, 0),
    ).toBe(true);
    expect(
      canRequestRelease({ tip: 100, finalityDepth: 10, halted: true }, { expiryHeight: 120, createdAt: 10 }, 86_410),
    ).toBe(true);
  });
});

import { createVendingChain, validateVendingBinding } from './vendingInstructions.js';
import {
  fixtureBinding,
  fixtureAddress,
  fixtureMachine,
  fixtureProgram,
  fixtureRelay,
  fixtureMint,
  mockVendingTransport,
} from './fixtures/vending.js';
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
    expect(session.payout).toBe(mock.wallet.address);
    expect((await chain.restore(nonce))?.nonce).toEqual(nonce);
    const claim = await chain.claim(mock.wallet, session, {
      proof: new Uint8Array(356),
      publicValues: new Uint8Array(249),
    });
    expect(await chain.transactionStatus(claim)).toBe('confirmed');
    expect(await chain.restore(nonce)).toBeNull();
    expect(mock.sentCount()).toBe(2);
    const ix = mock.instructions()[1]!;
    const program = new PublicKey(fixtureProgram);
    const machine = new PublicKey(fixtureMachine);
    const mintAuthority = PublicKey.findProgramAddressSync(
      [Buffer.from('mint'), machine.toBuffer()],
      program,
    )[0].toBase58();
    const paymentTag = PublicKey.findProgramAddressSync(
      [Buffer.from('tag'), machine.toBuffer(), Buffer.alloc(32)],
      program,
    )[0].toBase58();
    const payoutToken = PublicKey.findProgramAddressSync(
      [
        new PublicKey(session.payout).toBuffer(),
        new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA').toBuffer(),
        new PublicKey(fixtureMint).toBuffer(),
      ],
      new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL'),
    )[0].toBase58();
    expect(ix.keys.map((k) => k.pubkey.toBase58())).toEqual([
      mock.wallet.address,
      fixtureMachine,
      session.address,
      fixtureRelay,
      session.payout,
      fixtureMint,
      mintAuthority,
      payoutToken,
      paymentTag,
      'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
      'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL',
      '11111111111111111111111111111111',
    ]);
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
  it('rejects missing pins and wrong deployment identity', () => {
    expect(() => validateVendingBinding({ ...fixtureBinding, shopReceiver: [] }, deployment)).toThrow();
    expect(() => validateVendingBinding({ ...fixtureBinding, machineAddress: fixtureProgram }, deployment)).toThrow();
  });
  it('uses the real Machine/Reservation layouts and two-step release, including a request at unix zero', async () => {
    const mock = mockVendingTransport('expired');
    const chain = createVendingChain(deployment, mock.fetchImpl);
    await chain.read();
    await chain.read();
    const nonce = new Uint8Array(32).fill(9);
    const session = await chain.reserve(mock.wallet, nonce);
    expect(session.releaseRequested).toBe(false);
    await expect(chain.release(mock.wallet, session)).rejects.toThrow(/not requested/i);
    const requested = await chain.requestRelease(mock.wallet, session);
    expect(requested.releaseRequested).toBe(true);
    expect(releaseSecondsRemaining(requested, requested.releaseRequestedAt)).toBe(900);
    expect(releaseSecondsRemaining({ releaseRequested: true, releaseRequestedAt: 0 }, 899)).toBe(1);
    expect(releaseSecondsRemaining({ releaseRequested: true, releaseRequestedAt: 0 }, 900)).toBe(0);
    expect(releaseSecondsRemaining({ releaseRequested: false, releaseRequestedAt: 0 }, 1000)).toBeNull();
    await expect(chain.requestRelease(mock.wallet, requested)).rejects.toThrow(/already requested/i);
    await expect(chain.release(mock.wallet, requested)).rejects.toThrow(/Claim window/);
    const future = Date.now() + 900_000;
    vi.spyOn(Date, 'now').mockReturnValue(future);
    await chain.release(mock.wallet, requested);
    expect(await chain.restore(nonce)).toBeNull();
    expect(mock.instructions().map((ix) => ix.data[0])).toEqual([1, 4, 3]);
    const [reserve, request, release] = mock.instructions();
    expect(reserve!.keys.map((k) => [k.isWritable, k.isSigner])).toEqual([
      [true, true],
      [true, false],
      [true, false],
      [false, false],
      [false, false],
    ]);
    // Serialization promotes the fee payer to writable, even for a readonly signer meta.
    expect(request!.keys.map((k) => k.pubkey.toBase58())).toEqual([
      mock.wallet.address,
      fixtureMachine,
      session.address,
      fixtureRelay,
    ]);
    expect(release!.keys.map((k) => k.pubkey.toBase58())).toEqual([
      mock.wallet.address,
      fixtureMachine,
      session.address,
      fixtureRelay,
      session.payout,
    ]);
  });
  it('rejects an address/viewing-key association that differs in pk_d with the same diversifier', async () => {
    const mock = mockVendingTransport('payment');
    const changed = structuredClone(fixtureBinding);
    changed.shopReceiver[42] = 42;
    const chain = createVendingChain(deployment, async (input, init) =>
      String(input).endsWith('/abi.json') ? new Response(JSON.stringify(changed)) : mock.fetchImpl(input, init),
    );
    await expect(chain.read()).rejects.toThrow(/trusted receiver/);
  });
});
