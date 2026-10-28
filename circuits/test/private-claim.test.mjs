// Circuit tests on the depth-2 instance (blocks of up to 4 transactions) with real transactions
// of the ticket 08 devnet swap. Needs the compiled circuit:
//   scripts/build.sh 2         (writes $ZOLAR_DATA/build/d2, default ../.zolar-data/t10 next to the repo)
//   node --test test/
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import * as snarkjs from "snarkjs";

const here = new URL(".", import.meta.url).pathname;
const data = process.env.ZOLAR_DATA ?? join(here, "../../../../.zolar-data/t10");
const wasm = join(data, "build/d2/private_claim_d2_js/private_claim_d2.wasm");
const work = mkdtempSync(join(tmpdir(), "zolar-circuit-"));

const WINDOW = ["--lower", "4000000", "--expiry", "4000017"];
function makeInput(fixture, extra = [], depth = 2) {
  const out = join(work, `input-${Math.random().toString(36).slice(2)}.json`);
  execFileSync("node", [join(here, "../scripts/claim-input.mjs"), join(here, "../fixtures", fixture), "--depth", String(depth), ...WINDOW, ...extra, "--out", out]);
  return JSON.parse(readFileSync(out, "utf8"));
}
async function satisfies(input) {
  try {
    await snarkjs.wtns.calculate(input, wasm, join(work, "w.wtns"));
    return true;
  } catch (e) {
    // Only a violated constraint counts as a rejection; a missing wasm or bad input is a broken test.
    if (!/Assert Failed/i.test(String(e))) throw e;
    return false;
  }
}
const clone = (x) => structuredClone(x);

test("the payment of the devnet swap proves membership of its block in the tree", async () => {
  assert.ok(await satisfies(makeInput("swap2-block.json")));
});

test("a three-transaction block, last transaction paired with itself, and a one-level path", async () => {
  assert.ok(await satisfies(makeInput("swap2-block.json", ["--txs", "3", "--index", "2"])));
});

test("public inputs come out in the order root, commitment, lower, expiry", async () => {
  const input = makeInput("swap2-block.json");
  await snarkjs.wtns.calculate(input, wasm, join(work, "o.wtns"));
  const w = await snarkjs.wtns.exportJson(join(work, "o.wtns"));
  assert.deepEqual(w.slice(1, 5).map(String), [input.root, input.commitment, input.lower, input.expiry]);
});

test("rejects: wrong commitment, root, window, and tampered witness", async () => {
  const base = makeInput("swap2-block.json");
  const cases = {
    commitment: (i) => { i.commitment = (BigInt(i.commitment) + 1n).toString(); },
    root: (i) => { i.root = (BigInt(i.root) + 1n).toString(); },
    salt: (i) => { i.salt = (BigInt(i.salt) + 1n).toString(); },
    txidBit: (i) => { i.txid[17] ^= 1; },
    txSibling: (i) => { i.txSibling[0][5] ^= 1; },
    txSide: (i) => { i.txIsRight[1] ^= 1; },
    notBoolean: (i) => { i.txid[3] = 2; },
    treeSibling: (i) => { i.treeSibling[0] = "1"; },
    treeSide: (i) => { i.leafIndex[0] ^= 1; },
    heightBelowWindow: (i) => { i.lower = String(Number(i.height) + 1); },
    heightAboveWindow: (i) => { i.expiry = String(Number(i.height) - 1); },
    heightNotInTree: (i) => { i.height = String(Number(i.height) + 1); },
    pathCutShort: (i) => { i.txActive[1] = 0; },
    activeNotAPrefix: (i) => { i.txActive = ["0", "1"]; },
  };
  for (const [name, mutate] of Object.entries(cases)) {
    const input = clone(base);
    mutate(input);
    assert.equal(await satisfies(input), false, `${name} must be rejected`);
  }
});
