/** Deterministic test-only RPC/prover transport. No network, keys, or real proofs. */
import { Transaction, type TransactionInstruction } from '@solana/web3.js';
import { base58 } from '@scure/base';
import { Buffer } from 'buffer';
import type { VendingBinding, VendingConnection } from '../vendingInstructions.js';
import { PublicKey } from '../vendingInstructions.js';
import { hex } from '../vending.js';

export const fixtureMachine = 'Stake11111111111111111111111111111111111111';
export const fixtureProgram = 'BPFLoaderUpgradeab1e11111111111111111111111';
export const fixtureWallet = 'SysvarC1ock11111111111111111111111111111111';
export const fixtureMint = 'SysvarEpochSchedu1e111111111111111111111111';
export const fixtureRelay = 'SysvarRent111111111111111111111111111111111';
export const fixtureAddress =
  'utest1hap277en6ayclu5027232d44kv0s6nd0tacmnj07v0avarpuv4vzyfjreu3qexw05v9jvpx8ce37lpt5j4qexyrwhrlrma9msypee8x4';
const hash = [
  34, 94, 186, 121, 222, 27, 135, 136, 85, 84, 193, 107, 42, 172, 107, 59, 248, 115, 182, 32, 130, 167, 159, 26, 115,
  66, 14, 1, 188, 68, 12, 163,
];
export const fixtureBinding: VendingBinding = {
  version: 1,
  machineAddress: fixtureMachine,
  programAddress: fixtureProgram,
  shopReceiver: new Array(43).fill(7),
  shopViewingKeyHash: hash,
};
export function mockVendingTransport(state: string) {
  let reserved = false;
  let releaseRequested = false;
  let releaseRequestedAt = 0;
  const now = () => Math.floor(Date.now() / 1000);
  const instructions: TransactionInstruction[] = [];
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
    if (url.endsWith('/abi.json')) return response(fixtureBinding);
    if (url.includes('/find-payment'))
      return response(
        ['finality', 'proving', 'claiming', 'success', 'proof-failed', 'claim-failed'].includes(state) && reserved
          ? { status: 'found', txid: 'a'.repeat(64), height: 105, value_zat: 100_000, payout: fixtureWallet }
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
      values.set(new PublicKey(fixtureWallet).toBytes(), 189);
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
        const data = new Uint8Array(291);
        const view = new DataView(data.buffer);
        data.set([1, 1]);
        view.setBigUint64(132, 100_000n, true);
        view.setUint32(144, 100, true);
        view.setUint32(140, state === 'sold-out' ? 0 : 42, true);
        data.set(new PublicKey(fixtureRelay).toBytes(), 100);
        data.set(new PublicKey(fixtureMint).toBytes(), 68);
        data.fill(7, 148, 159);
        data.fill(8, 227, 291);
        data.set(hash, 159);
        value = rpcAccount(data, fixtureProgram);
      } else if (call.params[0] === fixtureRelay) {
        const data = new Uint8Array(5914);
        const view = new DataView(data.buffer);
        data.set([1, 1, 1, state === 'halted' ? 1 : 0]);
        data[95] = 1;
        data[96] = 10;
        view.setUint32(
          36,
          ['expired', 'release-window', 'release-ready'].includes(state) && reserved
            ? 150
            : 110 + (state === 'stalled' ? 0 : Math.min(relayReads++, 1)),
          true,
        );
        view.setUint32(
          1050,
          ['proving', 'claiming', 'success', 'proof-failed', 'claim-failed'].includes(state) ? 110 : 100,
          true,
        );
        value = rpcAccount(data, '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D');
      } else if (reserved && !claimed) {
        const data = new Uint8Array(128);
        const view = new DataView(data.buffer);
        data.set([2, 1]);
        data.set(machine, 3);
        data.set(nonce, 35);
        data.set(new PublicKey(fixtureWallet).toBytes(), 67);
        view.setBigUint64(99, 100_000n, true);
        view.setUint32(107, 120, true);
        view.setBigInt64(111, BigInt(now()), true);
        data[119] = releaseRequested ? 1 : 0;
        view.setBigInt64(120, BigInt(releaseRequestedAt), true);
        value = rpcAccount(data, fixtureProgram);
      }
      result = { context: { slot: 1 }, value };
    } else throw new Error(`Unexpected fixture RPC ${call.method}`);
    return response({ jsonrpc: '2.0', id: call.id, result });
  };
  const wallet: VendingConnection = {
    address: fixtureWallet,
    async send(bytes) {
      const tx = Transaction.from(bytes);
      const ix = tx.instructions.find((ix) => ix.programId.toBase58() === fixtureProgram)!;
      sends++;
      instructions.push(ix);
      if (ix.data[0] === 1) {
        nonce = new Uint8Array(ix.data.subarray(1));
        reserved = true;
        releaseRequested = ['release-window', 'release-ready'].includes(state);
        releaseRequestedAt = releaseRequested ? now() - (state === 'release-ready' ? 900 : 60) : 0;
      }
      if (ix.data[0] === 2) {
        if (state === 'claiming') await new Promise((resolve) => setTimeout(resolve, 600_000));
        if (state !== 'claim-failed') claimed = true;
      }
      if (ix.data[0] === 4) {
        releaseRequested = true;
        releaseRequestedAt = now();
      }
      if (ix.data[0] === 3) reserved = false;
      return base58.encode(new Uint8Array(64).fill(sends));
    },
  };
  return {
    fetchImpl,
    wallet,
    sentCount: () => sends,
    instructions: () => instructions,
  };
}
