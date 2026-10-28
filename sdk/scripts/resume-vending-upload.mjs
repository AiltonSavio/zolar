#!/usr/bin/env node
// Sequential devnet buffer writes for public RPCs that reject parallel CLI uploads.
import { appendFileSync, readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { Connection, Keypair, PublicKey, Transaction, TransactionInstruction } from '@solana/web3.js';
import { base58 } from '@scure/base';

const { values: v } = parseArgs({
  options: Object.fromEntries(['rpc', 'keypair', 'buffer', 'elf', 'ledger'].map((k) => [k, { type: 'string' }])),
});
const rpc = new Connection(v.rpc || 'https://api.devnet.solana.com', 'confirmed');
if ((await rpc.getGenesisHash()) !== 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG')
  throw new Error('Solana devnet only');
let signer;
try {
  signer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(v.keypair, 'utf8'))));
} catch {
  throw new Error('Cannot load deployer from keypair path');
}
const buffer = new PublicKey(v.buffer);
const loader = new PublicKey('BPFLoaderUpgradeab1e11111111111111111111111');
const elf = readFileSync(v.elf);
const account = await rpc.getAccountInfo(buffer);
if (
  !account ||
  !account.owner.equals(loader) ||
  account.data.readUInt32LE(0) !== 1 ||
  account.data[4] !== 1 ||
  !account.data.subarray(5, 37).equals(signer.publicKey.toBuffer()) ||
  account.data.length !== elf.length + 37
)
  throw new Error('Buffer layout, size or authority differs');
for (let offset = 0; offset < elf.length; offset += 900) {
  const bytes = elf.subarray(offset, offset + 900);
  if (account.data.subarray(offset + 37, offset + 37 + bytes.length).equals(bytes)) continue;
  // Bincode UpgradeableLoaderInstruction::Write { offset: u32, bytes: Vec<u8> }.
  const data = Buffer.alloc(16 + bytes.length);
  data.writeUInt32LE(1, 0);
  data.writeUInt32LE(offset, 4);
  data.writeBigUInt64LE(BigInt(bytes.length), 8);
  bytes.copy(data, 16);
  const tx = new Transaction({ feePayer: signer.publicKey, ...(await rpc.getLatestBlockhash()) }).add(
    new TransactionInstruction({
      programId: loader,
      keys: [
        { pubkey: buffer, isSigner: false, isWritable: true },
        { pubkey: signer.publicKey, isSigner: true, isWritable: false },
      ],
      data,
    }),
  );
  tx.sign(signer);
  const signature = base58.encode(tx.signature);
  appendFileSync(v.ledger, JSON.stringify({ offset, length: bytes.length, signature, status: 'signed' }) + '\n');
  const returned = await rpc.sendRawTransaction(tx.serialize(), { skipPreflight: false });
  if (returned !== signature) throw new Error('Signature mismatch');
  for (let attempt = 0; ; attempt++) {
    const s = (await rpc.getSignatureStatuses([signature], { searchTransactionHistory: true })).value[0];
    if (s?.err) throw new Error(`Write failed: ${JSON.stringify(s.err)}`);
    if (s?.confirmationStatus === 'confirmed' || s?.confirmationStatus === 'finalized') break;
    if (attempt > 60) throw new Error(`Reconcile saved signature before retrying: ${signature}`);
    await new Promise((r) => setTimeout(r, 1500));
  }
  appendFileSync(v.ledger, JSON.stringify({ offset, signature, status: 'confirmed' }) + '\n');
  console.log(`Write offset=${offset} bytes=${bytes.length} signature=${signature}`);
}
const live = await rpc.getAccountInfo(buffer);
if (!live?.data.subarray(37).equals(elf)) throw new Error('Readback differs from ELF; do not deploy');
console.log(`PASS: ${elf.length} uploaded bytes exactly match the ELF; finalize with solana program deploy --buffer`);
