// The public transcript of the ceremony (ceremony/transcript.json) and the checks that tie it to a zkey.
//
// rounds[0] is the initial zkey (no contribution), rounds[1..] are contributions in order, and the last
// round is the public random beacon. `final` names the key the Escrow uses.

export const FORMAT = 1;
const HEX = (n) => new RegExp(`^[0-9a-f]{${n}}$`);
export const CHAIN = "zcash-mainnet";
export const MIN_CONTRIBUTIONS = 3;
/** Anything that ends up in a shell command or a file name must match these. */
export const SAFE_URL = /^https:\/\/[A-Za-z0-9._~:/?&=%+@-]+$/;
const PTAU_FILE = /^[A-Za-z0-9_-][A-Za-z0-9_.-]*\.ptau$/;
const NAME = /^[\p{L}\p{N} ._@/:-]{1,64}$/u;

export function assertName(name) {
  if (typeof name !== "string" || !NAME.test(name)) {
    throw new Error(`contributor name must be 1-64 characters (letters, digits, space and ._@/:-), got ${JSON.stringify(name)}`);
  }
}

function must(cond, message) {
  if (!cond) throw new Error(`transcript: ${message}`);
}

/**
 * Throws unless the transcript is well formed and meets the ceremony policy. Does not look at any zkey.
 * `policy.minIterationsExp`: the weakest beacon hashing accepted (20 for the production ceremony).
 */
export function validateTranscript(t, policy = {}) {
  const minIterationsExp = policy.minIterationsExp ?? 10;
  must(t && t.format === FORMAT, `format must be ${FORMAT}`);
  must(t.circuit && HEX(64).test(t.circuit.r1csSha256) && Number.isInteger(t.circuit.depth), "circuit needs depth and r1csSha256");
  must(t.circuit.name === `private_claim_d${t.circuit.depth}`, "circuit name does not match its depth");
  must(t.ptau && HEX(64).test(t.ptau.sha256) && HEX(128).test(t.ptau.blake2b512) && SAFE_URL.test(t.ptau.url), "ptau needs sha256, blake2b512 and a plain https url");
  must(PTAU_FILE.test(t.ptau.file ?? ""), "ptau.file must be a bare file name");
  must(Array.isArray(t.rounds) && t.rounds.length >= 1, "rounds missing");
  const seen = new Set();
  t.rounds.forEach((r, i) => {
    must(r.round === i, `round ${i} is numbered ${r.round}`);
    must(HEX(64).test(r.sha256) && Number.isInteger(r.bytes) && r.bytes > 0, `round ${i} needs sha256 and bytes`);
    must(!seen.has(r.sha256), `round ${i} repeats an earlier file`);
    seen.add(r.sha256);
    must(r.url === undefined || SAFE_URL.test(r.url), `round ${i} url must be a plain https url`);
    if (i === 0) {
      must(r.kind === "init", "round 0 must be the init zkey");
      must(HEX(128).test(t.circuit.circuitHash ?? ""), "circuit.circuitHash (snarkjs circuit hash) missing");
      return;
    }
    must(r.kind === "contribution" || r.kind === "beacon", `round ${i} has kind ${r.kind}`);
    assertName(r.name);
    must(HEX(128).test(r.contributionHash), `round ${i} needs a 128-hex contributionHash`);
    must(!seen.has(r.contributionHash), `round ${i} repeats a contribution hash`);
    seen.add(r.contributionHash);
    if (r.kind === "beacon") {
      must(i === t.rounds.length - 1 && i >= 2, "the beacon must be the last round, after at least one contribution");
      const b = r.beacon;
      must(b && HEX(64).test(b.hash) && Number.isInteger(b.height) && Number.isInteger(b.iterationsExp), "beacon needs height, hash and iterationsExp");
      must(b.chain === CHAIN, `the beacon must come from ${CHAIN}`);
      must(b.iterationsExp >= minIterationsExp && b.iterationsExp <= 31, `beacon iterationsExp must be ${minIterationsExp}..31`);
      must(t.beaconPlan && t.beaconPlan.height === b.height && t.beaconPlan.chain === b.chain, "the beacon must be the one announced in beaconPlan");
      must(i - 1 >= MIN_CONTRIBUTIONS, `at least ${MIN_CONTRIBUTIONS} contributions are required before the beacon`);
      must(t.closed && t.closed.round === i - 1 && t.closed.zkeySha256 === t.rounds[i - 1].sha256, "the contributions must be closed (close) on the file the beacon was applied to");
    }
  });
  if (t.final !== undefined) {
    const last = t.rounds[t.rounds.length - 1];
    must(last.kind === "beacon", "final is set but the last round is not the beacon");
    must(t.final.zkeySha256 === last.sha256, "final.zkeySha256 is not the beacon round's file");
    must(HEX(64).test(t.final.verificationKeySha256), "final.verificationKeySha256 missing");
  }
  if (t.beaconPlan !== undefined) {
    must(Number.isInteger(t.beaconPlan.height) && t.beaconPlan.chain === CHAIN, `beaconPlan needs a ${CHAIN} height`);
  }
  if (t.closed !== undefined) {
    must(t.beaconPlan, "contributions cannot be closed before the beacon is announced");
    const c = t.closed;
    must(HEX(64).test(c.zkeySha256) && Number.isInteger(c.round) && c.round >= 1, "closed needs round and zkeySha256");
    must(t.rounds[c.round] && t.rounds[c.round].kind === "contribution" && t.rounds[c.round].sha256 === c.zkeySha256, "closed does not name a contribution file");
    const after = t.rounds.slice(c.round + 1);
    must(after.length === 0 || (after.length === 1 && after[0].kind === "beacon"), "a round was added after the contributions closed");
  }
  return t;
}

/** Contributions (rounds after the init) the transcript says a zkey of round `upTo` holds. */
export function expectedContributions(t, upTo = t.rounds.length - 1) {
  return t.rounds.slice(1, upTo + 1);
}

/**
 * Compares the report of `zkey verify` of a zkey with the transcript up to round `upTo`.
 * Returns the list of differences; empty means they agree.
 */
export function compareReport(t, report, upTo = t.rounds.length - 1) {
  const problems = [];
  if (!report.ok) problems.push("snarkjs did not report ZKey Ok");
  if (report.circuitHash !== t.circuit.circuitHash) problems.push("the zkey's circuit hash differs from the transcript's");
  const want = expectedContributions(t, upTo);
  if (report.contributions.length !== want.length) {
    problems.push(`the zkey holds ${report.contributions.length} contributions, the transcript lists ${want.length}`);
  }
  want.forEach((w, i) => {
    const got = report.contributions[i];
    if (!got) return;
    if (got.hash !== w.contributionHash) problems.push(`contribution #${i + 1} (${w.name}): hash differs from the transcript`);
    if (got.name !== w.name) problems.push(`contribution #${i + 1}: name ${JSON.stringify(got.name)} differs from ${JSON.stringify(w.name)}`);
    if (w.kind === "beacon") {
      if (got.beaconHash !== w.beacon.hash) problems.push("beacon hash in the zkey differs from the transcript's");
      if (got.beaconIterationsExp !== w.beacon.iterationsExp) problems.push("beacon iterations in the zkey differ from the transcript's");
    } else if (got.beaconHash !== undefined) {
      problems.push(`contribution #${i + 1} is a beacon but the transcript says it is not`);
    }
  });
  return problems;
}

/** The 64-hex beacon: a block hash in explorer order (as shown by block explorers). */
export function normalizeBeaconHash(s) {
  const h = String(s).trim().toLowerCase().replace(/^0x/, "");
  if (!HEX(64).test(h)) throw new Error("a beacon must be a 64-hex block hash");
  return h;
}
