/** Wallet/RPC adapter over the unmodified Codama-generated vending client. */
import { sha256 } from '@noble/hashes/sha256';
import { base58 } from '@scure/base';
import { Buffer } from 'buffer';
import { Connection, PublicKey, Transaction, TransactionInstruction, ComputeBudgetProgram } from '@solana/web3.js';
import { address, AccountRole, createNoopSigner, type Instruction } from '@solana/kit';
import {
  getMachineDecoder,
  getReservationDecoder,
  getReserveInstruction,
  getReserveInstructionDataEncoder,
  getClaimInstruction,
  getClaimInstructionDataEncoder,
  getRequestReleaseInstruction,
  getRequestReleaseInstructionDataEncoder,
  getReleaseReservationInstruction,
  getReleaseReservationInstructionDataEncoder,
} from '@zolar/vending-client';
import { ironwoodReceiver } from './vendingAddress';
import type { VendingProof, VendingRelay, VendingReservation } from './vending';

export { PublicKey };
const RELAY_OWNER = '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D';
const TOKEN = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const ASSOCIATED_TOKEN = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
/** Machine setup pins the address/key association; the wire format comes from Codama. */
export interface VendingBinding {
  version: 1;
  machineAddress: string;
  programAddress: string;
  shopReceiver: number[];
  shopViewingKeyHash: number[];
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
  requestRelease(wallet: VendingConnection, session: VendingSession): Promise<VendingSession>;
  release(wallet: VendingConnection, session: VendingSession): Promise<void>;
  claim(wallet: VendingConnection, session: VendingSession, proof: VendingProof): Promise<string>;
}
export function encodeVendingInstruction(
  input:
    | { kind: 'reserve'; nonce: Uint8Array }
    | { kind: 'release' }
    | { kind: 'requestRelease' }
    | ({ kind: 'claim' } & VendingProof),
): Uint8Array {
  if (input.kind === 'release') return new Uint8Array(getReleaseReservationInstructionDataEncoder().encode({}));
  if (input.kind === 'requestRelease') return new Uint8Array(getRequestReleaseInstructionDataEncoder().encode({}));
  if (input.kind === 'reserve') {
    if (input.nonce.length !== 32) throw new Error('Nonce must be 32 bytes');
    return new Uint8Array(getReserveInstructionDataEncoder().encode(input));
  }
  if (input.publicValues.length !== 249) throw new Error('Public values must be 249 bytes');
  return new Uint8Array(getClaimInstructionDataEncoder().encode(input));
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
export function canRequestRelease(
  relay: Pick<VendingRelay, 'tip' | 'finalityDepth' | 'halted'>,
  reservation: Pick<VendingReservation, 'expiryHeight' | 'createdAt'>,
  nowSeconds: number,
): boolean {
  return (
    relay.tip > reservation.expiryHeight + relay.finalityDepth ||
    (relay.halted && nowSeconds >= reservation.createdAt + 86_400)
  );
}
/** Null means release has not been requested; zero means the Claim window ended. */
export function releaseSecondsRemaining(
  reservation: Pick<VendingReservation, 'releaseRequested' | 'releaseRequestedAt'>,
  nowSeconds: number,
): number | null {
  return reservation.releaseRequested
    ? Math.max(0, Math.ceil(reservation.releaseRequestedAt + 900 - nowSeconds))
    : null;
}

export function validateVendingBinding(
  value: unknown,
  deployment: Pick<VendingDeployment, 'machine' | 'program'>,
): VendingBinding {
  const data = value as VendingBinding;
  if (
    !data ||
    data.version !== 1 ||
    data.machineAddress !== deployment.machine ||
    data.programAddress !== deployment.program
  )
    throw new Error('Deployment binding identity differs');
  const validBytes = (bytes: number[], length: number) =>
    Array.isArray(bytes) && bytes.length === length && bytes.every((b) => Number.isInteger(b) && b >= 0 && b < 256);
  if (!validBytes(data.shopReceiver, 43) || !validBytes(data.shopViewingKeyHash, 32))
    throw new Error('Missing trusted shop receiver binding');
  return data;
}

function walletInstruction(ix: Instruction): TransactionInstruction {
  return new TransactionInstruction({
    programId: new PublicKey(ix.programAddress),
    data: Buffer.from(ix.data ?? []),
    keys: (ix.accounts ?? []).map((meta) => ({
      pubkey: new PublicKey(meta.address),
      isWritable: meta.role === AccountRole.WRITABLE || meta.role === AccountRole.WRITABLE_SIGNER,
      isSigner: meta.role === AccountRole.READONLY_SIGNER || meta.role === AccountRole.WRITABLE_SIGNER,
    })),
  });
}

/** Codama defines all vending instruction bytes, account order, and account decoding. */
export function createVendingChain(
  config: VendingDeployment,
  fetchImpl: typeof fetch = globalThis.fetch.bind(globalThis),
): VendingChain {
  const rpc = new Connection(config.rpcUrl, { commitment: 'confirmed', fetch: fetchImpl });
  const machine = new PublicKey(config.machine);
  const program = new PublicKey(config.program);
  const receiver = ironwoodReceiver(config.address);
  let abiPromise: Promise<VendingBinding> | undefined;
  let networkChecked = false;
  let lastTip: number | undefined;
  let updatedAt = 0;
  async function abi() {
    abiPromise ??= fetchImpl(config.abiUrl, { signal: AbortSignal.timeout(10_000) })
      .then(async (response) => {
        if (!response.ok) throw new Error('Vending deployment ABI is unavailable');
        return validateVendingBinding(await response.json(), config);
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
    const bytes = await account(machine);
    if (bytes.length !== getMachineDecoder().fixedSize) throw new Error('Invalid Machine size');
    const data = getMachineDecoder().decode(bytes);
    if (data.discriminator !== 1 || data.version !== 1) throw new Error('Invalid Machine version');
    const viewingKeyHash = sha256(new Uint8Array(data.svk));
    if (
      binding.shopReceiver.length !== 43 ||
      binding.shopViewingKeyHash.length !== 32 ||
      !receiver.every((b, i) => binding.shopReceiver[i] === b) ||
      !viewingKeyHash.every((b, i) => binding.shopViewingKeyHash[i] === b && data.svkHash[i] === b)
    )
      throw new Error('Shop address is not the trusted receiver for this machine viewing key');
    if (!receiver.subarray(0, 11).every((b, i) => data.diversifier[i] === b))
      throw new Error('Shop address differs from the machine diversifier');
    return data;
  }
  async function read(): Promise<VendingSnapshot> {
    const data = await machineData();
    const relayAddress = new PublicKey(data.relay);
    const relayAccount = await rpc.getAccountInfo(relayAddress);
    if (!relayAccount) throw new Error('Relay is unavailable');
    const relay = decodeVendingRelay(relayAccount.data, relayAccount.owner.toBase58(), updatedAt);
    if (lastTip !== undefined && relay.tip > lastTip) updatedAt = Date.now();
    lastTip = relay.tip;
    relay.updatedAt = updatedAt;
    return {
      machine: machine.toBase58(),
      priceZat: data.priceZat,
      available: data.available,
      stock: data.stock,
      relay,
    };
  }
  async function reservation(nonce: Uint8Array): Promise<VendingSession> {
    const address = pda('reservation', nonce);
    const bytes = await account(address);
    if (bytes.length !== getReservationDecoder().fixedSize) throw new Error('Invalid Reservation size');
    const data = getReservationDecoder().decode(bytes);
    if (
      data.discriminator !== 2 ||
      data.version !== 1 ||
      bytes[119]! > 1 ||
      (!data.releaseRequested && data.releaseRequestedAt !== 0n)
    )
      throw new Error('Invalid Reservation version or release state');
    if (data.machine !== machine.toBase58() || !nonce.every((b, i) => data.nonce[i] === b))
      throw new Error('Reservation belongs to another request');
    return {
      nonce,
      address: address.toBase58(),
      payout: data.payout,
      priceZat: data.priceZat,
      expiryHeight: data.expiryHeight,
      createdAt: Number(data.createdAt),
      releaseRequested: data.releaseRequested,
      releaseRequestedAt: Number(data.releaseRequestedAt),
    };
  }
  async function send(
    kind: 'reserve' | 'claim' | 'release' | 'requestRelease',
    wallet: VendingConnection,
    nonce: Uint8Array,
    proof?: VendingProof,
  ) {
    const data = await machineData();
    const signer = new PublicKey(wallet.address);
    const payout = kind === 'reserve' ? signer : new PublicKey((await reservation(nonce)).payout);
    const mint = new PublicKey(data.mint);
    const submitter = createNoopSigner(address(wallet.address));
    const accounts = {
      submitter,
      machine: address(machine.toBase58()),
      reservation: address(pda('reservation', nonce).toBase58()),
      relay: data.relay,
    };
    const programConfig = { programAddress: address(config.program) };
    let generated: Instruction;
    if (kind === 'reserve') {
      if (nonce.length !== 32) throw new Error('Nonce must be 32 bytes');
      generated = getReserveInstruction({ ...accounts, payer: submitter, nonce }, programConfig);
    } else if (kind === 'requestRelease') generated = getRequestReleaseInstruction(accounts, programConfig);
    else if (kind === 'release')
      generated = getReleaseReservationInstruction({ ...accounts, payout: address(payout.toBase58()) }, programConfig);
    else {
      if (!proof || proof.publicValues.length !== 249) throw new Error('Public values must be 249 bytes');
      generated = getClaimInstruction(
        {
          ...accounts,
          submitter,
          payout: address(payout.toBase58()),
          mint: data.mint,
          mintAuthority: address(pda('mint').toBase58()),
          paymentTag: address(pda('tag', proof.publicValues.slice(77, 109)).toBase58()),
          payoutToken: address(
            PublicKey.findProgramAddressSync(
              [payout.toBuffer(), TOKEN.toBuffer(), mint.toBuffer()],
              ASSOCIATED_TOKEN,
            )[0].toBase58(),
          ),
          ...proof,
        },
        programConfig,
      );
    }
    const instruction = walletInstruction(generated);
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
    async requestRelease(wallet, session) {
      const current = await read();
      const stored = await reservation(session.nonce);
      if (stored.releaseRequested) throw new Error('Release already requested');
      if (!canRequestRelease(current.relay, stored, Date.now() / 1000))
        throw new Error('Reservation cannot request release yet');
      await send('requestRelease', wallet, session.nonce);
      return await reservation(session.nonce);
    },
    async release(wallet, session) {
      const stored = await reservation(session.nonce);
      const remaining = releaseSecondsRemaining(stored, Date.now() / 1000);
      if (remaining === null) throw new Error('Release not requested');
      if (remaining > 0) throw new Error('Claim window is still open');
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
