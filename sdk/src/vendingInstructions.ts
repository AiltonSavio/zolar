/** All handwritten vending ABI code lives here; replace this module with the Codama client. */
import { sha256 } from '@noble/hashes/sha256';
import { base58 } from '@scure/base';
import { Buffer } from 'buffer';
import {
  Connection,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  ComputeBudgetProgram,
} from '@solana/web3.js';
import { ironwoodReceiver } from './vendingAddress';
import type { VendingProof, VendingRelay, VendingReservation } from './vending';

export { PublicKey };
const RELAY_OWNER = '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D';
const TOKEN = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const ASSOCIATED_TOKEN = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
export type ClaimField = 'proof' | 'public_values' | 'proof_len_u32' | 'public_values_len_u32';
export interface VendingSelectors {
  reserve: readonly number[];
  claim: readonly number[];
  release: readonly number[];
  claimFields: readonly ClaimField[];
}
type AccountName =
  | 'signer'
  | 'payout'
  | 'machine'
  | 'reservation'
  | 'relay'
  | 'mint'
  | 'mintAuthority'
  | 'tag'
  | 'ticketAccount'
  | 'tokenProgram'
  | 'associatedTokenProgram'
  | 'systemProgram';
interface AccountLayout {
  length: number;
  prefix: number[];
}
/** Deployment-owned ABI manifest. The shared contract deliberately does not specify these offsets or account orders. */
export interface VendingAbi {
  version: 1;
  machineAddress: string;
  programAddress: string;
  /** Trusted association exported during machine setup, not derived by the browser. */
  shopReceiver: number[];
  shopViewingKeyHash: number[];
  selectors: VendingSelectors;
  machine: AccountLayout & {
    price: number;
    stock: number;
    available: number;
    relay: number;
    mint: number;
    diversifier: number;
    svkHash: number;
    svk: number;
  };
  reservation: AccountLayout & {
    machine: number;
    nonce: number;
    payout: number;
    price: number;
    expiry: number;
    createdAt: number;
  };
  accounts: Record<'reserve' | 'claim' | 'release', { name: AccountName; writable: boolean; signer: boolean }[]>;
}
export interface VendingDeployment {
  machine: string;
  program: string;
  address: string;
  rpcUrl: string;
  abiUrl: string;
}
export interface VendingConnection {
  address: string;
  send: (transaction: Uint8Array) => Promise<string>;
}
export interface VendingSession extends VendingReservation {
  address: string;
  payout: string;
  priceZat: bigint;
}
export interface VendingSnapshot {
  machine: string;
  priceZat: bigint;
  stock: number;
  available: number;
  relay: VendingRelay;
}
export interface VendingChain {
  transactionStatus(signature: string): Promise<'confirmed' | 'failed' | 'pending'>;
  restore(nonce: Uint8Array): Promise<VendingSession | null>;
  read(): Promise<VendingSnapshot>;
  reserve(wallet: VendingConnection, nonce: Uint8Array): Promise<VendingSession>;
  release(wallet: VendingConnection, session: VendingSession): Promise<void>;
  claim(wallet: VendingConnection, session: VendingSession, proof: VendingProof): Promise<string>;
}
function u32(value: number): Uint8Array {
  const out = new Uint8Array(4);
  new DataView(out.buffer).setUint32(0, value, true);
  return out;
}
function join(parts: readonly Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}
export function encodeVendingInstruction(
  selectors: VendingSelectors,
  input: { kind: 'reserve'; nonce: Uint8Array } | { kind: 'release' } | ({ kind: 'claim' } & VendingProof),
): Uint8Array {
  const prefix = Uint8Array.from(selectors[input.kind]);
  if (input.kind === 'release') return prefix;
  if (input.kind === 'reserve') {
    if (input.nonce.length !== 32) throw new Error('Nonce must be 32 bytes');
    return join([prefix, input.nonce]);
  }
  if (input.publicValues.length !== 249) throw new Error('Public values must be 249 bytes');
  return join([
    prefix,
    ...selectors.claimFields.map((field) =>
      field === 'proof'
        ? input.proof
        : field === 'public_values'
          ? input.publicValues
          : field === 'proof_len_u32'
            ? u32(input.proof.length)
            : u32(input.publicValues.length),
    ),
  ]);
}
export function decodeVendingRelay(data: Uint8Array, owner: string, updatedAt: number): VendingRelay {
  if (
    owner !== RELAY_OWNER ||
    data.length !== 5914 ||
    data[0] !== 1 ||
    data[1] !== 1 ||
    data[2] !== 1 ||
    data[95] !== 1
  )
    throw new Error('Invalid active testnet relay');
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const finalityDepth = v.getUint16(96, true);
  if (!finalityDepth) throw new Error('Invalid Finality Depth');
  return {
    tip: v.getUint32(36, true),
    finalHeight: v.getUint32(1050, true),
    finalityDepth,
    halted: data[3] !== 0,
    updatedAt,
  };
}
export function canReleaseReservation(
  relay: Pick<VendingRelay, 'tip' | 'finalityDepth' | 'halted'>,
  reservation: Pick<VendingReservation, 'expiryHeight' | 'createdAt'>,
  nowSeconds: number,
): boolean {
  return (
    relay.tip > reservation.expiryHeight + relay.finalityDepth ||
    (relay.halted && nowSeconds >= reservation.createdAt + 86_400)
  );
}
function checkLayout(data: Uint8Array, layout: AccountLayout) {
  if (data.length !== layout.length || !layout.prefix.every((b, i) => data[i] === b))
    throw new Error('Account does not match the deployment ABI');
}

/** Reject incomplete deployment artifacts before using offsets to read payment terms. */
export function validateVendingAbi(
  value: unknown,
  deployment: Pick<VendingDeployment, 'machine' | 'program'>,
): VendingAbi {
  const data = value as VendingAbi;
  if (
    !data ||
    data.version !== 1 ||
    data.machineAddress !== deployment.machine ||
    data.programAddress !== deployment.program
  )
    throw new Error('Deployment ABI identity differs');
  const validBytes = (bytes: readonly number[] | undefined, length?: number) =>
    Array.isArray(bytes) &&
    bytes.length > 0 &&
    (length === undefined || bytes.length === length) &&
    bytes.every((b) => Number.isInteger(b) && b >= 0 && b < 256);
  if (!validBytes(data.shopReceiver, 43) || !validBytes(data.shopViewingKeyHash, 32))
    throw new Error('Missing trusted shop receiver binding');
  const lengths = {
    machine: { price: 8, stock: 4, available: 4, relay: 32, mint: 32, diversifier: 11, svkHash: 32, svk: 64 },
    reservation: { machine: 32, nonce: 32, payout: 32, price: 8, expiry: 4, createdAt: 8 },
  };
  for (const name of ['machine', 'reservation'] as const) {
    const layout = data[name];
    if (!layout || !Number.isSafeInteger(layout.length) || layout.length <= 0 || !validBytes(layout.prefix))
      throw new Error('Invalid account layout');
    for (const [field, size] of Object.entries(lengths[name])) {
      const offset = (layout as unknown as Record<string, number>)[field]!;
      if (!Number.isSafeInteger(offset) || offset < layout.prefix.length || offset + size > layout.length)
        throw new Error(`Invalid ${name} ${field} offset`);
    }
  }
  const names = [
    'signer',
    'payout',
    'machine',
    'reservation',
    'relay',
    'mint',
    'mintAuthority',
    'tag',
    'ticketAccount',
    'tokenProgram',
    'associatedTokenProgram',
    'systemProgram',
  ];
  for (const kind of ['reserve', 'claim', 'release'] as const) {
    if (
      !validBytes(data.selectors?.[kind]) ||
      !Array.isArray(data.accounts?.[kind]) ||
      !data.accounts[kind].length ||
      data.accounts[kind].some(
        (meta) => !names.includes(meta.name) || typeof meta.writable !== 'boolean' || typeof meta.signer !== 'boolean',
      )
    )
      throw new Error('Invalid instruction layout');
  }
  const fields = data.selectors.claimFields;
  if (
    !Array.isArray(fields) ||
    fields.filter((f) => f === 'proof').length !== 1 ||
    fields.filter((f) => f === 'public_values').length !== 1 ||
    fields.some((f) => !['proof', 'public_values', 'proof_len_u32', 'public_values_len_u32'].includes(f))
  )
    throw new Error('Invalid Claim field layout');
  return data;
}

/** Requires an authoritative deployment ABI; no guessed selectors, account order or offsets are used. */
export function createVendingChain(
  config: VendingDeployment,
  fetchImpl: typeof fetch = globalThis.fetch.bind(globalThis),
): VendingChain {
  const rpc = new Connection(config.rpcUrl, { commitment: 'confirmed', fetch: fetchImpl });
  const machine = new PublicKey(config.machine);
  const program = new PublicKey(config.program);
  const receiver = ironwoodReceiver(config.address);
  let abiPromise: Promise<VendingAbi> | undefined;
  let networkChecked = false;
  let lastTip: number | undefined;
  let updatedAt = 0;
  async function abi() {
    abiPromise ??= fetchImpl(config.abiUrl, { signal: AbortSignal.timeout(10_000) })
      .then(async (response) => {
        if (!response.ok) throw new Error('Vending deployment ABI is unavailable');
        return validateVendingAbi(await response.json(), config);
      })
      .catch((error) => {
        abiPromise = undefined;
        throw error;
      });
    return abiPromise;
  }
  const pda = (name: string, ...seeds: Uint8Array[]) =>
    PublicKey.findProgramAddressSync(
      [Buffer.from(name), machine.toBuffer(), ...seeds.map((s) => Buffer.from(s))],
      program,
    )[0];
  async function account(key: PublicKey) {
    const value = await rpc.getAccountInfo(key);
    if (!value || !value.owner.equals(program)) throw new Error('Vending account is missing or has the wrong owner');
    return value.data;
  }
  async function machineData() {
    if (!networkChecked) {
      if ((await rpc.getGenesisHash()) !== 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG')
        throw new Error('Vending requires Solana devnet');
      networkChecked = true;
    }
    const binding = await abi();
    const layout = binding.machine;
    const bytes = await account(machine);
    checkLayout(bytes, layout);
    const viewingKeyHash = sha256(bytes.subarray(layout.svk, layout.svk + 64));
    if (
      binding.shopReceiver.length !== 43 ||
      binding.shopViewingKeyHash.length !== 32 ||
      !receiver.every((b, i) => binding.shopReceiver[i] === b) ||
      !viewingKeyHash.every((b, i) => binding.shopViewingKeyHash[i] === b && bytes[layout.svkHash + i] === b)
    )
      throw new Error('Shop address is not the trusted receiver for this machine viewing key');
    if (!receiver.subarray(0, 11).every((b, i) => bytes[layout.diversifier + i] === b))
      throw new Error('Shop address differs from the machine diversifier');
    return { layout, bytes, view: new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength) };
  }
  async function read(): Promise<VendingSnapshot> {
    const { layout, bytes, view } = await machineData();
    const relayAddress = new PublicKey(bytes.subarray(layout.relay, layout.relay + 32));
    const relayAccount = await rpc.getAccountInfo(relayAddress);
    if (!relayAccount) throw new Error('Relay is unavailable');
    const relay = decodeVendingRelay(relayAccount.data, relayAccount.owner.toBase58(), updatedAt);
    if (lastTip !== undefined && relay.tip > lastTip) updatedAt = Date.now();
    lastTip = relay.tip;
    relay.updatedAt = updatedAt;
    return {
      machine: machine.toBase58(),
      priceZat: view.getBigUint64(layout.price, true),
      available: view.getUint32(layout.available, true),
      stock: view.getUint32(layout.stock, true),
      relay,
    };
  }
  async function reservation(nonce: Uint8Array): Promise<VendingSession> {
    const layout = (await abi()).reservation;
    const address = pda('reservation', nonce);
    const bytes = await account(address);
    checkLayout(bytes, layout);
    if (
      !machine.toBytes().every((b, i) => bytes[layout.machine + i] === b) ||
      !nonce.every((b, i) => bytes[layout.nonce + i] === b)
    )
      throw new Error('Reservation belongs to another request');
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    return {
      nonce,
      address: address.toBase58(),
      payout: new PublicKey(bytes.subarray(layout.payout, layout.payout + 32)).toBase58(),
      priceZat: view.getBigUint64(layout.price, true),
      expiryHeight: view.getUint32(layout.expiry, true),
      createdAt: Number(view.getBigInt64(layout.createdAt, true)),
    };
  }
  async function send(
    kind: 'reserve' | 'claim' | 'release',
    wallet: VendingConnection,
    nonce: Uint8Array,
    proof?: VendingProof,
  ) {
    const binding = await abi();
    const { layout, bytes } = await machineData();
    const signer = new PublicKey(wallet.address);
    const payout = kind === 'reserve' ? signer : new PublicKey((await reservation(nonce)).payout);
    const mint = new PublicKey(bytes.subarray(layout.mint, layout.mint + 32));
    const accounts: Record<AccountName, PublicKey> = {
      signer,
      payout,
      machine,
      reservation: pda('reservation', nonce),
      relay: new PublicKey(bytes.subarray(layout.relay, layout.relay + 32)),
      mint,
      mintAuthority: pda('mint'),
      tag: pda('tag', proof?.publicValues.slice(77, 109) ?? new Uint8Array(32)),
      ticketAccount: PublicKey.findProgramAddressSync(
        [payout.toBuffer(), TOKEN.toBuffer(), mint.toBuffer()],
        ASSOCIATED_TOKEN,
      )[0],
      tokenProgram: TOKEN,
      associatedTokenProgram: ASSOCIATED_TOKEN,
      systemProgram: SystemProgram.programId,
    };
    const input =
      kind === 'reserve'
        ? ({ kind, nonce } as const)
        : kind === 'release'
          ? ({ kind } as const)
          : ({ kind, ...proof! } as const);
    const instruction = new TransactionInstruction({
      programId: program,
      keys: binding.accounts[kind].map((a) => ({
        pubkey: accounts[a.name],
        isSigner: a.signer,
        isWritable: a.writable,
      })),
      data: Buffer.from(encodeVendingInstruction(binding.selectors, input)),
    });
    const block = await rpc.getLatestBlockhash();
    const tx = new Transaction({ feePayer: signer, ...block }).add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
      instruction,
    );
    const signature = await wallet.send(tx.serialize({ requireAllSignatures: false, verifySignatures: false }));
    for (let attempt = 0; ; attempt++) {
      const result = await rpc.getSignatureStatuses([signature], { searchTransactionHistory: true });
      const status = result.value[0];
      if (status?.err) throw new Error(`Vending transaction failed: ${JSON.stringify(status.err)}`);
      if (status?.confirmationStatus === 'confirmed' || status?.confirmationStatus === 'finalized') break;
      if (attempt >= 59) throw new Error('Transaction confirmation timed out; check its signature before retrying');
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    return signature;
  }
  return {
    read,
    async transactionStatus(signature) {
      const result = await rpc.getSignatureStatuses([signature], { searchTransactionHistory: true });
      const status = result.value[0];
      return status?.err
        ? 'failed'
        : status?.confirmationStatus === 'confirmed' || status?.confirmationStatus === 'finalized'
          ? 'confirmed'
          : 'pending';
    },
    async restore(nonce) {
      const address = pda('reservation', nonce);
      if (!(await rpc.getAccountInfo(address))) return null;
      return await reservation(nonce);
    },
    async reserve(wallet, nonce) {
      const existing = await this.restore(nonce);
      if (existing) {
        if (existing.payout !== wallet.address) throw new Error('Reservation belongs to another wallet');
        return existing;
      }
      const current = await read();
      if (current.available === 0) throw new Error('Sold out');
      if (current.relay.halted || Date.now() - current.relay.updatedAt > 120_000)
        throw new Error('The relay is not advancing. Do not pay.');
      await send('reserve', wallet, nonce);
      const result = await reservation(nonce);
      if (result.payout !== wallet.address) throw new Error('Reservation wallet differs');
      return result;
    },
    async release(wallet, session) {
      const current = await read();
      if (!canReleaseReservation(current.relay, session, Date.now() / 1000))
        throw new Error('Reservation cannot be released yet');
      await send('release', wallet, session.nonce);
    },
    async claim(wallet, session, proof) {
      return await send('claim', wallet, session.nonce, proof);
    },
  };
}

export function signatureBase58(bytes: Uint8Array): string {
  if (bytes.length !== 64) throw new Error('Invalid transaction signature');
  return base58.encode(bytes);
}
