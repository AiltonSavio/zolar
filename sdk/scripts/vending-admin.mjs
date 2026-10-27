#!/usr/bin/env node
// Devnet-only administration. Secret material is consumed only by path, never logged.
import { readFileSync, writeFileSync } from 'node:fs';
import { randomBytes, createHash } from 'node:crypto';
import { parseArgs } from 'node:util';
import { pathToFileURL } from 'node:url';
import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  ComputeBudgetProgram,
} from '@solana/web3.js';
import { address, createNoopSigner, AccountRole } from '@solana/kit';
import { base58 } from '@scure/base';
import {
  getInitMachineInstruction,
  getReserveInstruction,
  getMachineDecoder,
  getReservationDecoder,
  VENDING_PROGRAM_ADDRESS,
} from '@zolar/vending-client';
import { decodeVendingRelay } from '../dist/vendingInstructions.js';
import { ironwoodReceiver } from '../dist/vendingAddress.js';
import { paymentLink } from '../dist/vending.js';

export const DEFAULT_RESERVATION_BLOCKS = 20;
export function checkedShop(shop) {
  const receiver = ironwoodReceiver(shop.address);
  const key = Buffer.from(shop.shop_viewing_key, 'hex');
  if (
    key.length !== 64 ||
    shop.network !== 'testnet' ||
    shop.diversifier !== Buffer.from(receiver.slice(0, 11)).toString('hex') ||
    shop.svk_hash !== createHash('sha256').update(key).digest('hex') ||
    !Buffer.from(receiver).equals(Buffer.from(shop.receiver))
  )
    throw new Error('Invalid dedicated testnet shop metadata');
  return { receiver, key };
}
const json = (path) => JSON.parse(readFileSync(path, 'utf8'));
const save = (path, value, fresh = false) =>
  writeFileSync(path, JSON.stringify(value, (_, v) => (typeof v === 'bigint' ? v.toString() : v), 2) + '\n', {
    flag: fresh ? 'wx' : 'w',
  });
const loadSigner = (path) => {
  try {
    return Keypair.fromSecretKey(Uint8Array.from(json(path)));
  } catch {
    throw new Error('Cannot load signer from the supplied keypair path');
  }
};
const key = (s) => new PublicKey(s);
const pda = (program, ...seeds) =>
  PublicKey.findProgramAddressSync(
    seeds.map((s) => (typeof s === 'string' ? Buffer.from(s) : s)),
    program,
  )[0];
function instruction(ix) {
  return new TransactionInstruction({
    programId: key(ix.programAddress),
    data: Buffer.from(ix.data),
    keys: ix.accounts.map((a) => ({
      pubkey: key(a.address),
      isSigner: [AccountRole.WRITABLE_SIGNER, AccountRole.READONLY_SIGNER].includes(a.role),
      isWritable: [AccountRole.WRITABLE, AccountRole.WRITABLE_SIGNER].includes(a.role),
    })),
  });
}
async function main() {
  const {
    values: v,
    positionals: [command],
  } = parseArgs({
    allowPositionals: true,
    options: {
      help: { type: 'boolean' },
      ...Object.fromEntries(
        [
          'rpc',
          'keypair',
          'mint-keypair',
          'shop',
          'machine',
          'seed',
          'nonce',
          'relay',
          'vkey-hash',
          'price-zat',
          'stock',
          'reservation-blocks',
          'out',
        ].map((k) => [k, { type: 'string' }]),
      ),
    },
  });
  if (v.help) {
    console.log(
      'Usage: vending-admin.sh init-machine|reserve|inspect [--rpc URL] [--keypair PATH] [--shop PUBLIC_JSON] [--out JSON]',
    );
    return;
  }
  const definition = readFileSync(new URL('../../clients/codama/vending.mjs', import.meta.url), 'utf8');
  const sourceProgram = definition.match(/export const VENDING_PROGRAM_ID\s*=\s*'([^']+)'/)?.[1];
  if (sourceProgram !== VENDING_PROGRAM_ADDRESS)
    throw new Error('Stale vending client: rebuild clients/vending-js, then run pnpm --dir sdk install --force');
  const rpc = new Connection(v.rpc || 'https://api.devnet.solana.com', 'finalized');
  if ((await rpc.getGenesisHash()) !== 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG')
    throw new Error('Solana devnet only');
  const program = key(VENDING_PROGRAM_ADDRESS);
  const read = async (address) => {
    const a = await rpc.getAccountInfo(key(address));
    if (!a) throw new Error(`Account missing: ${address}`);
    return a;
  };
  async function machine(address) {
    const a = await read(address);
    if (!a.owner.equals(program) || a.data.length !== 291 || a.executable)
      throw new Error('Wrong Machine owner/layout');
    return getMachineDecoder().decode(a.data);
  }
  async function relay(address) {
    const a = await read(address);
    const r = decodeVendingRelay(a.data, a.owner.toBase58(), Date.now());
    if (r.halted) throw new Error('Relay Halted');
    return r;
  }
  if (command === 'inspect') {
    const m = await machine(v.machine);
    console.log(
      JSON.stringify(
        {
          machine: v.machine,
          ...m,
          relayState: await relay(m.relay),
          mintState: (await rpc.getParsedAccountInfo(key(m.mint))).value?.data,
        },
        (_, x) => (typeof x === 'bigint' ? x.toString() : x),
        2,
      ),
    );
    return;
  }
  if (!v.out || !v.keypair) throw new Error('--out and --keypair paths required');
  const signer = loadSigner(v.keypair);
  const noop = createNoopSigner(address(signer.publicKey.toBase58()));
  let ix,
    signers = [signer],
    result;
  if (command === 'init-machine') {
    const shop = json(v.shop);
    const { receiver, key: svk } = checkedShop(shop);
    const mint = loadSigner(v['mint-keypair']);
    signers.push(mint);
    const seed = v.seed ? Buffer.from(v.seed, 'hex') : randomBytes(32);
    const vkey = Buffer.from(v['vkey-hash'] || '', 'hex');
    if (seed.length !== 32 || vkey.length !== 32) throw new Error('seed and vkey hash require 32 bytes');
    const ma = pda(program, 'machine', signer.publicKey.toBuffer(), seed);
    const authority = pda(program, 'mint', ma.toBuffer());
    const duration = Number(v['reservation-blocks'] || DEFAULT_RESERVATION_BLOCKS);
    await relay(v.relay);
    ix = getInitMachineInstruction({
      payee: noop,
      machine: address(ma.toBase58()),
      mint: createNoopSigner(address(mint.publicKey.toBase58())),
      mintAuthority: address(authority.toBase58()),
      relay: address(v.relay),
      seed,
      priceZat: BigInt(v['price-zat'] || '100000'),
      stock: Number(v.stock || 100),
      diversifier: receiver.slice(0, 11),
      svk,
      relayAddress: address(v.relay),
      vkeyHash: vkey,
      reservationBlocks: duration,
    });
    result = {
      program: program.toBase58(),
      machine: ma.toBase58(),
      mint: mint.publicKey.toBase58(),
      mintAuthority: authority.toBase58(),
      machineSeed: seed.toString('hex'),
      reservationBlocks: duration,
      relay: v.relay,
      shopAddress: shop.address,
    };
  } else if (command === 'reserve') {
    const m = await machine(v.machine);
    await relay(m.relay);
    const shop = json(v.shop);
    checkedShop(shop);
    if (
      shop.svk_hash !== Buffer.from(m.svkHash).toString('hex') ||
      shop.diversifier !== Buffer.from(m.diversifier).toString('hex') ||
      !Buffer.from(m.svk).equals(Buffer.from(shop.shop_viewing_key, 'hex'))
    )
      throw new Error('Shop differs from Machine');
    const nonce = v.nonce ? Buffer.from(v.nonce, 'hex') : randomBytes(32);
    if (nonce.length !== 32) throw new Error('nonce requires 32 bytes');
    const reservation = pda(program, 'reservation', key(v.machine).toBuffer(), nonce);
    ix = getReserveInstruction({
      payer: noop,
      machine: address(v.machine),
      reservation: address(reservation.toBase58()),
      relay: m.relay,
      nonce,
    });
    result = {
      program: program.toBase58(),
      machine: v.machine,
      reservation: reservation.toBase58(),
      nonceHex: nonce.toString('hex'),
      nonce: new PublicKey(nonce).toBase58(),
      payout: signer.publicKey.toBase58(),
      paymentUri: paymentLink(shop.address, m.priceZat, key(v.machine).toBytes(), nonce, signer.publicKey.toBytes()),
    };
  } else throw new Error('Use init-machine, reserve or inspect');
  // Persist identity before submission. A timeout must be reconciled, never retried with a fresh identity.
  save(v.out, { ...result, status: 'preparing' }, true);
  const block = await rpc.getLatestBlockhash();
  const tx = new Transaction({ feePayer: signer.publicKey, ...block }).add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: 200000 }),
    instruction(ix),
  );
  tx.sign(...signers);
  const simulation = await rpc.simulateTransaction(tx);
  if (simulation.value.err) throw new Error(`Simulation failed: ${JSON.stringify(simulation.value)}`);
  const expectedSignature = base58.encode(tx.signature);
  save(v.out, { ...result, status: 'signed', signature: expectedSignature });
  const signature = await rpc.sendRawTransaction(tx.serialize(), { skipPreflight: false });
  if (signature !== expectedSignature) throw new Error('RPC returned a different transaction signature');
  save(v.out, { ...result, status: 'submitted', signature });
  for (let attempt = 0; ; attempt++) {
    const status = (await rpc.getSignatureStatuses([signature], { searchTransactionHistory: true })).value[0];
    if (status?.err) throw new Error(`Transaction failed: ${JSON.stringify(status.err)}`);
    if (status?.confirmationStatus === 'finalized') break;
    if (attempt >= 90) throw new Error('Confirmation timed out; reconcile the saved signature before retrying');
    await new Promise((resolve) => setTimeout(resolve, 2000));
  }
  const executed = await rpc.getTransaction(signature, { commitment: 'finalized', maxSupportedTransactionVersion: 0 });
  result = {
    ...result,
    status: 'finalized',
    signature,
    computeUnits: executed?.meta?.computeUnitsConsumed,
    fee: executed?.meta?.fee,
  };
  if (command === 'init-machine') {
    result.config = await machine(result.machine);
    save(
      v.out.replace(/\.json$/, '') + '.binding.json',
      {
        version: 1,
        machineAddress: result.machine,
        programAddress: program.toBase58(),
        shopReceiver: json(v.shop).receiver,
        shopViewingKeyHash: Array.from(Buffer.from(json(v.shop).svk_hash, 'hex')),
      },
      true,
    );
  } else result.config = getReservationDecoder().decode((await read(result.reservation)).data);
  save(v.out, result);
  console.log(JSON.stringify(result, (_, x) => (typeof x === 'bigint' ? x.toString() : x), 2));
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch((e) => {
    console.error(e.message);
    process.exitCode = 1;
  });
