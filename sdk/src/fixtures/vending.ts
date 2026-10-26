/** Deterministic test-only RPC/prover transport. No network, keys, or real proofs. */
import { Transaction } from '@solana/web3.js';
import { base58 } from '@scure/base';
import { Buffer } from 'buffer';
import type { VendingAbi, VendingConnection } from '../vendingInstructions';
import { PublicKey } from '../vendingInstructions';
import { hex } from '../vending';

export const fixtureMachine = '11111111111111111111111111111111';
export const fixtureProgram = 'BPFLoaderUpgradeab1e11111111111111111111111';
export const fixtureRelay = 'SysvarRent111111111111111111111111111111111';
export const fixtureAddress =
  'utest1hap277en6ayclu5027232d44kv0s6nd0tacmnj07v0avarpuv4vzyfjreu3qexw05v9jvpx8ce37lpt5j4qexyrwhrlrma9msypee8x4';
const hash = [
  34, 94, 186, 121, 222, 27, 135, 136, 85, 84, 193, 107, 42, 172, 107, 59, 248, 115, 182, 32, 130, 167, 159, 26, 115,
  66, 14, 1, 188, 68, 12, 163,
];
const meta = (name: VendingAbi['accounts']['reserve'][number]['name'], writable = false, signer = false) => ({
  name,
  writable,
  signer,
});
/** Illustrative fixture ABI only: never use this artifact as a deployment ABI. */
export const fixtureAbi: VendingAbi = {
  version: 1,
  machineAddress: fixtureMachine,
  programAddress: fixtureProgram,
  shopReceiver: new Array(43).fill(7),
  shopViewingKeyHash: hash,
  selectors: { reserve: [1], claim: [2], release: [3], claimFields: ['proof_len_u32', 'proof', 'public_values'] },
  machine: {
    length: 189,
    prefix: [1, 1],
    price: 2,
    stock: 10,
    available: 14,
    relay: 18,
    mint: 50,
    diversifier: 82,
    svk: 93,
    svkHash: 157,
  },
  reservation: {
    length: 118,
    prefix: [2, 1],
    machine: 2,
    nonce: 34,
    payout: 66,
    price: 98,
    expiry: 106,
    createdAt: 110,
  },
  accounts: {
    reserve: [
      meta('signer', true, true),
      meta('machine', true),
      meta('reservation', true),
      meta('relay'),
      meta('systemProgram'),
    ],
    claim: [
      meta('signer', true, true),
      meta('machine', true),
      meta('reservation', true),
      meta('payout', true),
      meta('tag', true),
      meta('relay'),
      meta('mint', true),
      meta('mintAuthority'),
      meta('ticketAccount', true),
      meta('tokenProgram'),
      meta('associatedTokenProgram'),
      meta('systemProgram'),
    ],
    release: [meta('machine', true), meta('reservation', true), meta('payout', true), meta('relay')],
  },
};
export function mockVendingTransport(state: string) {
  let reserved = false;
  let nonce = new Uint8Array(32);
  let relayReads = 0;
  let sends = 0;
  let claimed = false;
  const machine = new PublicKey(fixtureMachine).toBytes();
  const rpcAccount = (data: Uint8Array, owner: string) => ({
    data: [Buffer.from(data).toString('base64'), 'base64'],
    owner,
    executable: false,
    lamports: 1_000_000,
    rentEpoch: 0,
  });
  const response = (value: unknown) =>
    new Response(JSON.stringify(value), { headers: { 'Content-Type': 'application/json' } });
  const fetchImpl: typeof fetch = async (input, init) => {
    const url = String(input);
    if (url.endsWith('/abi.json')) return response(fixtureAbi);
    if (url.includes('/find-payment'))
      return response(
        ['finality', 'proving', 'claiming', 'success', 'proof-failed', 'claim-failed'].includes(state) && reserved
          ? { status: 'found', txid: 'a'.repeat(64), height: 105, value_zat: 100_000, payout: fixtureMachine }
          : { status: 'pending' },
      );
    if (url.endsWith('/prove')) return response({ job: 'fixture-job' });
    if (url.endsWith('/prove/fixture-job')) {
      if (state === 'proof-failed')
        return response({ status: 'failed', error: 'Proof service unavailable. Try again.' });
      if (state === 'proving') return response({ status: 'proving' });
      const values = new Uint8Array(249);
      values[0] = 1;
      values[188] = 3;
      values.set(machine, 5);
      values.set(nonce, 37);
      values.set(machine, 189);
      return response({ status: 'done', proof: '00'.repeat(356), public_values: hex(values) });
    }
    const call = JSON.parse(String(init?.body)) as { id: string; method: string; params: string[] };
    let result: unknown;
    if (call.method === 'getGenesisHash') result = 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG';
    else if (call.method === 'getLatestBlockhash')
      result = { context: { slot: 1 }, value: { blockhash: fixtureMachine, lastValidBlockHeight: 500 } };
    else if (call.method === 'getSignatureStatuses')
      result = {
        context: { slot: 1 },
        value: [
          {
            slot: 1,
            confirmations: 1,
            confirmationStatus: 'confirmed',
            err: state === 'claim-failed' && sends > 1 ? { InstructionError: [0, { Custom: 1 }] } : null,
          },
        ],
      };
    else if (call.method === 'getAccountInfo') {
      let value: unknown = null;
      if (call.params[0] === fixtureMachine) {
        const data = new Uint8Array(189);
        const view = new DataView(data.buffer);
        data.set([1, 1]);
        view.setBigUint64(2, 100_000n, true);
        view.setUint32(10, 100, true);
        view.setUint32(14, state === 'sold-out' ? 0 : 42, true);
        data.set(new PublicKey(fixtureRelay).toBytes(), 18);
        data.fill(7, 82, 93);
        data.fill(8, 93, 157);
        data.set(hash, 157);
        value = rpcAccount(data, fixtureProgram);
      } else if (call.params[0] === fixtureRelay) {
        const data = new Uint8Array(5914);
        const view = new DataView(data.buffer);
        data.set([1, 1, 1, state === 'halted' ? 1 : 0]);
        data[95] = 1;
        data[96] = 10;
        view.setUint32(
          36,
          state === 'expired' && reserved ? 150 : 110 + (state === 'stalled' ? 0 : Math.min(relayReads++, 1)),
          true,
        );
        view.setUint32(
          1050,
          ['proving', 'claiming', 'success', 'proof-failed', 'claim-failed'].includes(state) ? 110 : 100,
          true,
        );
        value = rpcAccount(data, '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D');
      } else if (reserved && !claimed) {
        const data = new Uint8Array(118);
        const view = new DataView(data.buffer);
        data.set([2, 1]);
        data.set(machine, 2);
        data.set(nonce, 34);
        data.set(machine, 66);
        view.setBigUint64(98, 100_000n, true);
        view.setUint32(106, 120, true);
        view.setBigInt64(110, BigInt(Math.floor(Date.now() / 1000)), true);
        value = rpcAccount(data, fixtureProgram);
      }
      result = { context: { slot: 1 }, value };
    } else throw new Error(`Unexpected fixture RPC ${call.method}`);
    return response({ jsonrpc: '2.0', id: call.id, result });
  };
  const wallet: VendingConnection = {
    address: fixtureMachine,
    async send(bytes) {
      const tx = Transaction.from(bytes);
      const ix = tx.instructions.find((ix) => ix.programId.toBase58() === fixtureProgram)!;
      sends++;
      if (ix.data[0] === 1) {
        nonce = new Uint8Array(ix.data.subarray(1));
        reserved = true;
      }
      if (ix.data[0] === 2) {
        if (state === 'claiming') await new Promise((resolve) => setTimeout(resolve, 600_000));
        if (state !== 'claim-failed') claimed = true;
      }
      if (ix.data[0] === 3) reserved = false;
      return base58.encode(new Uint8Array(64).fill(sends));
    },
  };
  return { fetchImpl, wallet, sentCount: () => sends };
}
