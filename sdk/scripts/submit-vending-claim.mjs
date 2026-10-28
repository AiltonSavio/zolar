#!/usr/bin/env node
// Local-only Claim submitter. The rented prover never receives this signer.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
import { Connection, ComputeBudgetProgram, Keypair, TransactionMessage, VersionedTransaction } from '@solana/web3.js';
import { base58 } from '@scure/base';
import { claimInstruction, requireProof } from './check-vending-settlement.mjs';

export async function submitClaim(rpc, signer, before, result, out) {
  requireProof(result);
  assert.equal(await rpc.getGenesisHash(), 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG', 'Solana devnet only');
  assert.equal(signer.publicKey.toBase58(), before.submitter, 'Signer differs from baseline submitter');
  assert.equal(
    createHash('sha256').update(JSON.stringify(result)).digest('hex'),
    before.resultSha256,
    'Proof differs from baseline',
  );
  const block = await rpc.getLatestBlockhash('finalized');
  const tx = new VersionedTransaction(
    new TransactionMessage({
      payerKey: signer.publicKey,
      recentBlockhash: block.blockhash,
      instructions: [ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }), claimInstruction(before, result)],
    }).compileToV0Message(),
  );
  const simulation = (await rpc.simulateTransaction(tx, { sigVerify: false, commitment: 'finalized' })).value;
  assert.equal(simulation.err, null, 'Claim simulation failed; preserve proof and reconcile');
  tx.sign([signer]);
  const signature = base58.encode(tx.signatures[0]);
  const receipt = {
    status: 'signed',
    signature,
    blockhash: block.blockhash,
    lastValidBlockHeight: block.lastValidBlockHeight,
    resultSha256: before.resultSha256,
    machine: before.machine,
    reservation: before.reservation,
  };
  // Fail before broadcast if this attempt already has a journal. Never silently re-sign.
  writeFileSync(out, JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx' });
  const returned = await rpc.sendRawTransaction(tx.serialize(), { skipPreflight: false, maxRetries: 0 });
  assert.equal(returned, signature, 'RPC returned a different signature; reconcile the saved journal');
  writeFileSync(out, JSON.stringify({ ...receipt, status: 'submitted' }, null, 2) + '\n');
  return signature;
}

async function main() {
  const { values: v } = parseArgs({
    options: {
      help: { type: 'boolean' },
      ...Object.fromEntries(['rpc', 'result', 'before', 'keypair', 'out'].map((name) => [name, { type: 'string' }])),
    },
  });
  if (v.help) {
    console.log(
      'Local devnet Claim: --result JSON --before JSON --keypair PATH --out JOURNAL [--rpc URL]\nSigns and broadcasts once; check the saved signature after any timeout. Never put the keypair on the rented host.',
    );
    return;
  }
  assert(v.result && v.before && v.keypair && v.out, '--result, --before, --keypair and --out required');
  const result = JSON.parse(readFileSync(v.result, 'utf8'));
  requireProof(result);
  const before = JSON.parse(readFileSync(v.before, 'utf8'));
  const rpc = new Connection(v.rpc ?? 'https://api.devnet.solana.com', 'finalized');
  const check = spawnSync(
    'python3',
    [
      '-I',
      fileURLToPath(new URL('../../scripts/check-vending-execution.py', import.meta.url)),
      '--rpc',
      rpc.rpcEndpoint,
      '--result',
      v.result,
      '--machine',
      before.machine,
      '--reservation',
      before.reservation,
      '--require-proof',
      '--snapshot-dir',
      `${v.out}.accounts`,
    ],
    { encoding: 'utf8' },
  );
  assert.equal(check.status, 0, check.stderr || check.stdout || 'Fresh preflight failed');
  let signer;
  try {
    signer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(v.keypair, 'utf8'))));
  } catch {
    throw new Error('Cannot load signer from supplied keypair path');
  }
  const signature = await submitClaim(rpc, signer, before, result, v.out);
  console.log(
    `SUBMITTED ${signature}. Wait for finalization, then run check-vending-settlement.mjs after. This is not settlement evidence yet.`,
  );
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
