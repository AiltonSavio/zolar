// Independent reference for the Private Claim circuit input (circomlibjs + node:crypto only).
//
//   node scripts/claim-input.mjs <fixture.json> --depth D --checkpoint H --leaves N --slot K \
//        --lower L --expiry E [--txs N] [--out input.json]
//
// The fixture lists the block's transaction IDs (explorer order) and the salt. The Final Block Tree
// holds N leaves for heights H+1 .. H+N; leaf K is the fixture's block (at height H+1+K), the others
// are blocks with Merkle roots sha256("zolar-fixture-block-<i>"). Prints the circuit input as JSON.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { buildPoseidon } from "circomlibjs";

const TREE_DEPTH = 20;
const args = process.argv.slice(2);
const fixtureFile = args.shift();
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i < 0 ? fallback : args[i + 1];
};
const depth = Number(opt("depth"));
const checkpoint = Number(opt("checkpoint", 4000000));
const leafCount = Number(opt("leaves", 6));
const slot = Number(opt("slot", 3));
const lower = Number(opt("lower"));
const expiry = Number(opt("expiry"));
const out = opt("out");

const fixture = JSON.parse(readFileSync(fixtureFile, "utf8"));
const take = Number(opt("txs", fixture.txids_display.length));
const internal = (hex) => Buffer.from(hex, "hex").reverse();
const txids = fixture.txids_display.slice(0, take).map(internal);
const index = Number(opt("index", fixture.index));
const sha = (b) => createHash("sha256").update(b).digest();
const shad = (b) => sha(sha(b));

// Transaction Merkle tree, Bitcoin style: an odd last node pairs with itself.
let level = txids;
let i = index;
const path = [];
while (level.length > 1) {
  path.push(level[i ^ 1] ?? level[i]);
  const next = [];
  for (let j = 0; j < level.length; j += 2) {
    next.push(shad(Buffer.concat([level[j], level[j + 1] ?? level[j]])));
  }
  level = next;
  i >>= 1;
}
const blockRoot = level[0];
if (take === fixture.txids_display.length && blockRoot.toString("hex") !== internal(fixture.block_merkle_root_display).toString("hex")) {
  throw new Error("the fixture's transactions do not hash to its Merkle root");
}
if (path.length > depth) throw new Error(`transaction path of ${path.length} levels exceeds depth ${depth}`);

const poseidon = await buildPoseidon();
const F = poseidon.F;
const H = (xs) => F.toObject(poseidon(xs));
const big = (b) => BigInt("0x" + Buffer.from(b).toString("hex"));
const limbs = (b) => [big(b.subarray(0, 16)), big(b.subarray(16))];
const bits = (b) => [...b].flatMap((byte) => Array.from({ length: 8 }, (_, k) => (byte >> (7 - k)) & 1));

const salt = BigInt("0x" + fixture.salt);
const [txHi, txLo] = limbs(txids[index]);
const commitment = H([txHi, txLo, salt]);

// Final Block Tree.
const height = checkpoint + 1 + slot;
const leaves = [];
for (let k = 0; k < leafCount; k++) {
  const root = k === slot ? blockRoot : sha(Buffer.from(`zolar-fixture-block-${k}`));
  const [hi, lo] = limbs(root);
  leaves.push(H([BigInt(checkpoint + 1 + k), hi, lo]));
}
const zeros = [H([0n, 0n])];
for (let l = 1; l < TREE_DEPTH; l++) zeros.push(H([zeros[l - 1], zeros[l - 1]]));
let nodes = leaves;
const treePath = [];
let at = slot;
for (let l = 0; l < TREE_DEPTH; l++) {
  treePath.push(nodes[at ^ 1] ?? zeros[l]);
  const next = [];
  for (let j = 0; j < nodes.length; j += 2) next.push(H([nodes[j], nodes[j + 1] ?? zeros[l]]));
  nodes = next;
  at >>= 1;
}
const treeRoot = nodes[0];

const pad = (a, n, fill) => [...a, ...Array.from({ length: n - a.length }, () => fill)];
const zeroBits = Array(256).fill(0);
const input = {
  root: treeRoot.toString(),
  commitment: commitment.toString(),
  lower: String(lower),
  expiry: String(expiry),
  txid: bits(txids[index]),
  salt: salt.toString(),
  txSibling: pad(path.map(bits), depth, zeroBits),
  txIsRight: pad(path.map((_, l) => (index >> l) & 1), depth, 0),
  txActive: pad(path.map(() => 1), depth, 0),
  height: String(height),
  leafIndex: Array.from({ length: TREE_DEPTH }, (_, l) => (slot >> l) & 1),
  treeSibling: treePath.map(String),
};
const json = JSON.stringify(input);
if (out) writeFileSync(out, json + "\n");
else console.log(json);
