#!/usr/bin/env node
// Read-only evidence collection. This tool never loads a signer or sends a transaction.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  Connection,
  PublicKey,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from '@solana/web3.js';
import { AccountRole, address, createNoopSigner } from '@solana/kit';
import { base58 } from '@scure/base';
import {
  getClaimInstruction,
  getMachineDecoder,
  getReservationDecoder,
  VENDING_PROGRAM_ADDRESS,
} from '@zolar/vending-client';

const TOKEN = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const ATA = 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL';
const DEVNET = 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG';
const json = (path) => JSON.parse(readFileSync(path, 'utf8'));
const save = (path, data) => writeFileSync(path, JSON.stringify(data, null, 2) + '\n', { flag: 'wx' });
const digest = (result) => createHash('sha256').update(JSON.stringify(result)).digest('hex');
const key = (value) => new PublicKey(value);
const pda = (program, ...seeds) =>
  PublicKey.findProgramAddressSync(
    seeds.map((s) => (typeof s === 'string' ? Buffer.from(s) : s)),
    key(program),
  )[0].toBase58();

export function requireProof(result) {
  assert.equal(result.mode, 'groth16', 'Execution is not a Groth16 Payment Proof');
  assert.match(result.proof ?? '', /^(?:[0-9a-fA-F]{712}|[0-9a-fA-F]{776})$/, 'Malformed SP1 proof');
  assert.match(result.public_values ?? '', /^[0-9a-fA-F]{498}$/, 'Expected 249 public bytes');
}

export function claimInstruction(before, result) {
  requireProof(result);
  const ix = getClaimInstruction(
    {
      submitter: createNoopSigner(address(before.submitter)),
      ...Object.fromEntries(
        ['machine', 'reservation', 'relay', 'payout', 'mint', 'mintAuthority', 'payoutToken'].map((name) => [
          name,
          address(before[name]),
        ]),
      ),
      paymentTag: address(before.tag),
      proof: Buffer.from(result.proof, 'hex'),
      publicValues: Buffer.from(result.public_values, 'hex'),
    },
    { programAddress: address(before.program) },
  );
  return new TransactionInstruction({
    programId: key(before.program),
    data: Buffer.from(ix.data),
    keys: ix.accounts.map((a) => ({
      pubkey: key(a.address),
      isSigner: [AccountRole.WRITABLE_SIGNER, AccountRole.READONLY_SIGNER].includes(a.role),
      isWritable: [AccountRole.WRITABLE, AccountRole.WRITABLE_SIGNER].includes(a.role),
    })),
  });
}

export function verifyTransaction(tx, before, result, signature) {
  assert(tx && tx.meta, 'Finalized Claim transaction is unavailable');
  assert.equal(tx.meta.err, null, 'Claim transaction failed');
  assert(tx.slot >= before.slot, 'Claim predates the baseline');
  assert.equal(tx.transaction.signatures[0], signature, 'Wrong transaction signature');
  const message = tx.transaction.message;
  const keys = [
    ...(message.staticAccountKeys ?? message.accountKeys),
    ...(tx.meta.loadedAddresses?.writable ?? []),
    ...(tx.meta.loadedAddresses?.readonly ?? []),
  ].map((p) => p.toBase58());
  const instructions = message.compiledInstructions ?? message.instructions;
  const claims = instructions.filter((ix) => keys[ix.programIdIndex] === before.program);
  assert.equal(claims.length, 1, 'Expected exactly one vending instruction');
  const actual = claims[0];
  const expected = claimInstruction(before, result);
  const bytes = typeof actual.data === 'string' ? base58.decode(actual.data) : actual.data;
  assert.deepEqual(Buffer.from(bytes), expected.data, 'Transaction contains a different proof/Claim');
  assert.deepEqual(
    Array.from(actual.accountKeyIndexes ?? actual.accounts, (index) => keys[index]),
    expected.keys.map((a) => a.pubkey.toBase58()),
    'Claim account binding differs',
  );
  // Token deltas are from this transaction, so unrelated purchases cannot make this pass.
  const index = keys.indexOf(before.payoutToken);
  assert(index >= 0, 'Payout token account missing');
  const post = tx.meta.postTokenBalances?.find((b) => b.accountIndex === index);
  const pre = tx.meta.preTokenBalances?.find((b) => b.accountIndex === index);
  assert(post && tx.meta.preTokenBalances, 'Missing token balance metadata');
  for (const balance of [pre, post].filter(Boolean)) {
    assert.equal(balance.mint, before.mint, 'Wrong ticket mint');
    assert.equal(balance.owner, before.payout, 'Wrong ticket payout');
    assert.equal(balance.uiTokenAmount.decimals, 0, 'Ticket mint must have zero decimals');
  }
  const delta = BigInt(post.uiTokenAmount.amount) - BigInt(pre?.uiTokenAmount.amount ?? '0');
  assert.equal(delta, 1n, 'Claim must deliver exactly one ticket');
  assert(Number.isInteger(tx.meta.computeUnitsConsumed), 'Missing Claim CU evidence');
  return {
    signature,
    slot: tx.slot,
    tickets: delta.toString(),
    computeUnits: tx.meta.computeUnitsConsumed,
    fee: tx.meta.fee,
  };
}

export function requireReplayRejection(simulation, program) {
  assert.deepEqual(
    simulation.err,
    { InstructionError: [0, { Custom: 16 }] },
    'Replay must fail with MissingReservation, not an unrelated simulation error',
  );
  assert(simulation.logs?.includes(`Program ${program} invoke [1]`), 'Vending program did not execute');
  assert(
    simulation.logs?.includes(`Program ${program} failed: custom program error: 0x10`),
    'MissingReservation was not emitted by vending',
  );
}

function decode(account, decoder, program) {
  assert(account && account.owner.toBase58() === program && !account.executable, 'Wrong account owner');
  assert.equal(account.data.length, decoder.fixedSize, 'Wrong account size');
  const value = decoder.decode(account.data);
  assert.equal(value.version, 1, 'Wrong account version');
  return value;
}

async function main() {
  const {
    values: v,
    positionals: [phase],
  } = parseArgs({
    allowPositionals: true,
    options: {
      help: { type: 'boolean' },
      ...Object.fromEntries(
        ['rpc', 'result', 'machine', 'reservation', 'submitter', 'before', 'signature', 'out'].map((name) => [
          name,
          { type: 'string' },
        ]),
      ),
    },
  });
  if (v.help) {
    console.log(
      'Read-only: before --result JSON --machine ADDRESS --reservation ADDRESS [--submitter ADDRESS] --out JSON\nafter --before JSON --result JSON --signature SIGNATURE --out JSON\nBoth phases require devnet. No signer files, payment, Claim or release is sent.',
    );
    return;
  }
  assert(['before', 'after'].includes(phase), 'Use before or after');
  assert(v.result && v.out, '--result and --out required');
  const result = json(v.result);
  requireProof(result);
  const rpc = new Connection(v.rpc ?? 'https://api.devnet.solana.com', 'finalized');
  assert.equal(await rpc.getGenesisHash(), DEVNET, 'Solana devnet only');
  if (phase === 'before') {
    assert(v.machine && v.reservation, '--machine and --reservation required');
    const script = fileURLToPath(new URL('../../scripts/check-vending-execution.py', import.meta.url));
    const check = spawnSync(
      'python3',
      [
        '-I',
        script,
        '--rpc',
        rpc.rpcEndpoint,
        '--result',
        v.result,
        '--machine',
        v.machine,
        '--reservation',
        v.reservation,
        '--require-proof',
        '--snapshot-dir',
        `${v.out}.accounts`,
      ],
      { encoding: 'utf8' },
    );
    assert.equal(check.status, 0, check.stderr || check.stdout || 'Account preflight failed');
    const accounts = await rpc.getMultipleAccountsInfoAndContext([key(v.machine), key(v.reservation)]);
    const [ma, ra] = accounts.value;
    const m = decode(ma, getMachineDecoder(), VENDING_PROGRAM_ADDRESS);
    const r = decode(ra, getReservationDecoder(), VENDING_PROGRAM_ADDRESS);
    assert.equal(r.machine, v.machine, 'Wrong Reservation machine');
    assert.equal(r.discriminator, 2);
    assert.equal(m.discriminator, 1);
    assert.equal(
      v.reservation,
      pda(VENDING_PROGRAM_ADDRESS, 'reservation', key(v.machine).toBuffer(), r.nonce),
      'Wrong Reservation PDA',
    );
    assert(!r.releaseRequested, 'Release requested; reconcile before proceeding');
    const pv = Buffer.from(result.public_values, 'hex');
    const before = {
      version: 1,
      program: VENDING_PROGRAM_ADDRESS,
      machine: v.machine,
      reservation: v.reservation,
      submitter: v.submitter ?? r.payout,
      payout: r.payout,
      relay: m.relay,
      mint: m.mint,
      tag: pda(VENDING_PROGRAM_ADDRESS, 'tag', key(v.machine).toBuffer(), pv.subarray(77, 109)),
      mintAuthority: pda(VENDING_PROGRAM_ADDRESS, 'mint', key(v.machine).toBuffer()),
      payoutToken: pda(ATA, key(r.payout).toBuffer(), key(TOKEN).toBuffer(), key(m.mint).toBuffer()),
      slot: accounts.context.slot,
      reservationLamports: ra.lamports,
      resultSha256: digest(result),
    };
    assert.equal(await rpc.getAccountInfo(key(before.tag)), null, 'Payment Tag already consumed');
    save(v.out, before);
    console.log(
      `PASS preflight at slot ${before.slot}; sign Claim promptly, then run after. Proof acceptance remains unverified until Claim finalizes.`,
    );
    return;
  }
  assert(v.before && v.signature, '--before and --signature required');
  const before = json(v.before);
  assert.equal(before.version, 1);
  assert.equal(before.program, VENDING_PROGRAM_ADDRESS, 'Baseline program differs');
  assert.equal(before.resultSha256, digest(result), 'Proof changed since baseline');
  const tx = await rpc.getTransaction(v.signature, { commitment: 'finalized', maxSupportedTransactionVersion: 0 });
  const receipt = verifyTransaction(tx, before, result, v.signature);
  const current = await rpc.getMultipleAccountsInfoAndContext(
    [key(before.reservation), key(before.tag), key(before.mint), key(before.payoutToken)],
    { commitment: 'finalized', minContextSlot: receipt.slot },
  );
  const [reservation, tag, mint, token] = current.value;
  assert.equal(reservation, null, 'Reservation did not close');
  assert(
    tag && !tag.executable && tag.owner.toBase58() === before.program && tag.data.equals(Buffer.from([3, 1])),
    'Permanent Payment Tag missing or invalid',
  );
  assert(
    mint && mint.owner.toBase58() === TOKEN && mint.data.length === 82 && mint.data[44] === 0 && mint.data[45] === 1,
    'Invalid ticket mint',
  );
  assert.equal(mint.data.readUInt32LE(0), 1, 'Mint authority absent');
  assert.equal(new PublicKey(mint.data.subarray(4, 36)).toBase58(), before.mintAuthority, 'Wrong mint authority');
  assert.equal(mint.data.readUInt32LE(46), 0, 'Unexpected freeze authority');
  assert(
    token && token.owner.toBase58() === TOKEN && token.data.length === 165 && token.data[108] === 1,
    'Payout token account missing',
  );
  assert.equal(new PublicKey(token.data.subarray(0, 32)).toBase58(), before.mint);
  assert.equal(new PublicKey(token.data.subarray(32, 64)).toBase58(), before.payout);
  assert(token.data.readBigUInt64LE(64) >= 1n, 'Ticket no longer held by payout; reconcile transfers');
  const block = await rpc.getLatestBlockhash('finalized');
  const replay = new VersionedTransaction(
    new TransactionMessage({
      payerKey: key(before.submitter),
      recentBlockhash: block.blockhash,
      instructions: [claimInstruction(before, result)],
    }).compileToV0Message(),
  );
  const simulation = (
    await rpc.simulateTransaction(replay, {
      sigVerify: false,
      commitment: 'finalized',
      minContextSlot: current.context.slot,
    })
  ).value;
  save(`${v.out}.replay.json`, simulation);
  requireReplayRejection(simulation, before.program);
  save(v.out, {
    ...receipt,
    machine: before.machine,
    reservation: before.reservation,
    payout: before.payout,
    mint: before.mint,
    tag: before.tag,
    mintSupply: mint.data.readBigUInt64LE(36).toString(),
    currentTicketBalance: token.data.readBigUInt64LE(64).toString(),
    reservationClosed: true,
    tagConsumed: true,
    replay: 'MissingReservation (simulation only)',
    resultSha256: before.resultSha256,
  });
  console.log(
    `PASS finalized Claim: exactly one ticket to ${before.payout}, Reservation closed, Tag consumed, replay rejected; ${receipt.computeUnits} CU.`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
