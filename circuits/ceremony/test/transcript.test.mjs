import assert from "node:assert/strict";
import test from "node:test";
import { h, sampleTranscript as base } from "./helpers.mjs";
import { compareReport, normalizeBeaconHash, validateTranscript } from "../lib/transcript.mjs";

const report = (t) => ({
  ok: true,
  circuitHash: t.circuit.circuitHash,
  contributions: t.rounds.slice(1).map((r, i) => ({
    index: i + 1, name: r.name, hash: r.contributionHash,
    ...(r.beacon ? { beaconHash: r.beacon.hash, beaconIterationsExp: r.beacon.iterationsExp } : {}),
  })),
});

test("a well formed transcript validates and matches its own report", () => {
  const t = validateTranscript(base());
  assert.deepEqual(compareReport(t, report(t)), []);
});

test("a report with a swapped contribution hash is rejected", () => {
  const t = base();
  const r = report(t);
  r.contributions[0].hash = h("0", 128);
  assert.match(compareReport(t, r).join(), /contribution #1 \(Alice\): hash differs/);
});

test("a report that drops or adds a contribution is rejected", () => {
  const t = base();
  const r = report(t);
  r.contributions.splice(1, 1);
  assert.match(compareReport(t, r).join(), /holds 3 contributions, the transcript lists 4/);
});

test("a different beacon or circuit is rejected", () => {
  const t = base();
  const r = report(t);
  r.contributions[3].beaconHash = h("8");
  r.circuitHash = h("0", 128);
  const problems = compareReport(t, r).join();
  assert.match(problems, /beacon hash/);
  assert.match(problems, /circuit hash/);
});

test("the beacon must be last, announced, and preceded by a contribution", () => {
  const early = base();
  early.rounds.push({ ...early.rounds[2], round: 5, sha256: h("6"), contributionHash: h("6", 128) });
  assert.throws(() => validateTranscript(early), /beacon must be the last round/);
  const unannounced = base();
  unannounced.beaconPlan.height = 1;
  assert.throws(() => validateTranscript(unannounced), /announced in beaconPlan/);
});

test("names cannot carry newlines or hash lookalikes", () => {
  const t = base();
  t.rounds[1].name = "Eve\ncontribution #9:";
  assert.throws(() => validateTranscript(t), /contributor name/);
});

test("final must be the beacon file", () => {
  const t = base();
  t.final = { zkeySha256: h("2"), verificationKeySha256: h("a") };
  assert.throws(() => validateTranscript(t), /not the beacon round's file/);
});

test("beacon hashes are 64 hex characters", () => {
  assert.equal(normalizeBeaconHash(` 0x${"AB".repeat(32)}\n`), "ab".repeat(32));
  assert.throws(() => normalizeBeaconHash("abcd"), /64-hex/);
});

test("policy: three contributions, the Zcash chain, a closed round and a strong beacon", () => {
  const few = base();
  few.rounds.splice(3, 1);
  few.rounds[3].round = 3;
  few.closed = { round: 2, zkeySha256: h("3") };
  assert.throws(() => validateTranscript(few), /at least 3 contributions/);

  const chain = base();
  chain.beaconPlan.chain = "made-up-chain";
  chain.rounds[4].beacon.chain = "made-up-chain";
  assert.throws(() => validateTranscript(chain), /the beacon must come from zcash-mainnet/);

  const open = base();
  delete open.closed;
  assert.throws(() => validateTranscript(open), /must be closed/);

  const late = base();
  late.closed = { round: 2, zkeySha256: h("3") };
  assert.throws(() => validateTranscript(late), /contributions must be closed/);

  assert.throws(() => validateTranscript(base(), { minIterationsExp: 21 }), /iterationsExp must be 21/);
});

test("file names and urls cannot escape their place or carry shell syntax", () => {
  const path = base();
  path.ptau.file = "../../../../.bashrc";
  assert.throws(() => validateTranscript(path), /bare file name/);
  const url = base();
  url.rounds[1].url = "https://example.org/$(id)";
  assert.throws(() => validateTranscript(url), /plain https url/);
});
