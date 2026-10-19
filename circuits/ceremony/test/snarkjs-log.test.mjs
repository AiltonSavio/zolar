import assert from "node:assert/strict";
import test from "node:test";
import { hashFromBlock, parseContributionHash, parseZkeyVerify } from "../lib/snarkjs-log.mjs";

const block = (seed) =>
  [0, 1, 2, 3]
    .map((r) => "\t\t" + [0, 1, 2, 3].map((w) => (seed * 1000 + r * 4 + w).toString(16).padStart(8, "0")).join(" "))
    .join("\n");
const flat = (seed) => block(seed).replace(/\s/g, "");

// Messages in the order and format snarkjs 0.7.6 logs them in zkey verify.
const verifyMessages = [
  `Circuit Hash: \n${block(7)}`,
  "-------------------------",
  `contribution #2 Bob:\n${block(2)}`,
  "Beacon generator: abcdef",
  "Beacon iterations Exp: 20",
  "-------------------------",
  `contribution #1 Alice Example:\n${block(1)}`,
  "-------------------------",
  "ZKey Ok!",
];

test("zkey verify output yields contributions oldest first, with the beacon parameters", () => {
  const r = parseZkeyVerify(verifyMessages);
  assert.equal(r.ok, true);
  assert.equal(r.circuitHash, flat(7));
  assert.deepEqual(r.contributions.map((c) => [c.index, c.name, c.hash]), [
    [1, "Alice Example", flat(1)],
    [2, "Bob", flat(2)],
  ]);
  assert.equal(r.contributions[1].beaconHash, "abcdef");
  assert.equal(r.contributions[1].beaconIterationsExp, 20);
  assert.equal(r.contributions[0].beaconHash, undefined);
});

test("a report without ZKey Ok is not ok", () => {
  assert.equal(parseZkeyVerify(verifyMessages.filter((m) => m !== "ZKey Ok!")).ok, false);
});

test("a gap in the contribution numbers is an error", () => {
  const gap = verifyMessages.filter((m) => !m.startsWith("contribution #1"));
  assert.throws(() => parseZkeyVerify(gap), /skips contribution #1/);
});

test("the contribution hash comes from the Contribution Hash message", () => {
  assert.equal(parseContributionHash([`Circuit Hash: \n${block(7)}`, `Contribution Hash: \n${block(3)}`]), flat(3));
  assert.throws(() => parseContributionHash(["nothing"]), /no contribution hash/);
});

test("a hash block must be 64 bytes", () => {
  assert.equal(hashFromBlock(block(1)), flat(1));
  assert.throws(() => hashFromBlock("abcd"), /64-byte/);
});

test("a contribution whose name carries a line break cannot hide: the report is rejected", () => {
  const hidden = [
    `Circuit Hash: \n${block(7)}`,
    `contribution #2 Hidden\na:\n${block(2)}`,
    `contribution #1 Alice:\n${block(1)}`,
    "ZKey Ok!",
  ];
  assert.throws(() => parseZkeyVerify(hidden), /malformed contribution message/);
});
