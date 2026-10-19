// Independent reference for the Header Relay's Final Block Tree (circomlibjs Poseidon, a plain
// recomputation of every level; shares no code with the Rust program).
//
//   node reference.mjs fixture <fixture.txt> <first-leaf-height> <last-leaf-height>
//   node reference.mjs leaves <leaves.json>        # [{"height": n, "merkleRoot": "<64 hex, internal byte order>"}]
//
// Prints JSON: zeros (hex, level 0..19), emptyRoot, leaves, roots (roots[k] = root after k leaves).
// Fixture lines are "<height> <raw header hex>"; the Merkle root is bytes 36..68 of the header.
import { buildPoseidon } from 'circomlibjs';
import { readFileSync } from 'node:fs';

const DEPTH = 20;
const poseidon = await buildPoseidon();
const F = poseidon.F;
const H = (...xs) => F.toObject(poseidon(xs));
const hex32 = (n) => n.toString(16).padStart(64, '0');

const zeros = [H(0n, 0n)];
for (let i = 1; i < DEPTH; i++) zeros.push(H(zeros[i - 1], zeros[i - 1]));

function rootOf(leaves) {
  let level = leaves.slice();
  for (let d = 0; d < DEPTH; d++) {
    if (level.length % 2 === 1) level.push(zeros[d]);
    const next = [];
    for (let i = 0; i < level.length; i += 2) next.push(H(level[i], level[i + 1]));
    level = next.length ? next : [H(zeros[d], zeros[d])];
  }
  return level[0];
}

function leafOf(height, merkleRootHex) {
  const b = Buffer.from(merkleRootHex, 'hex');
  return H(BigInt(height), BigInt('0x' + b.subarray(0, 16).toString('hex')), BigInt('0x' + b.subarray(16).toString('hex')));
}

const [mode, ...args] = process.argv.slice(2);
let entries;
if (mode === 'fixture') {
  const [file, first, last] = args;
  entries = readFileSync(file, 'utf8').trim().split('\n').map((line) => {
    const [h, raw] = line.split(' ');
    return { height: Number(h), merkleRoot: raw.slice(72, 136) };
  }).filter((e) => e.height >= Number(first) && e.height <= Number(last));
} else if (mode === 'leaves') {
  entries = JSON.parse(readFileSync(args[0], 'utf8'));
} else {
  console.error('usage: reference.mjs fixture <file> <first> <last> | leaves <json>');
  process.exit(1);
}
const leaves = entries.map((e) => leafOf(e.height, e.merkleRoot));
const roots = [hex32(rootOf([]))];
for (let k = 1; k <= leaves.length; k++) roots.push(hex32(rootOf(leaves.slice(0, k))));
console.log(JSON.stringify({
  zeros: zeros.map(hex32),
  emptyRoot: roots[0],
  leaves: entries.map((e, i) => ({ ...e, leaf: hex32(leaves[i]) })),
  roots,
}, null, 1));
