// Parsing of the text snarkjs prints through its logger. snarkjs 0.7.6 is pinned in package.json; the
// tests in test/snarkjs-log.test.mjs run against messages in that exact format.

/** Collects every logger message; `echo` also prints them. */
export function captureLogger(echo = false) {
  const messages = [];
  const push = (level) => (message) => {
    messages.push({ level, message: String(message) });
    if (echo || level === "error" || level === "warn") process.stderr.write(`${message}\n`);
  };
  return {
    messages,
    debug: () => {},
    info: push("info"),
    warn: push("warn"),
    error: push("error"),
  };
}

/** "\t\t0123abcd ...\n\t\t..." (4 rows of 4 words) -> 128 lowercase hex characters. */
export function hashFromBlock(block) {
  if (!HASH_BLOCK.test(block)) throw new Error("expected a 64-byte hash block");
  return blockHex(block);
}

const WORDS = "(?:\\t\\t[0-9a-f]{8}(?: [0-9a-f]{8}){3}\\n?){4}";
const HASH_BLOCK = new RegExp(`^${WORDS}$`);
const TITLED = new RegExp(`^([^\\n]*):[ ]?\\n(${WORDS})$`);

function blockHex(block) {
  return block.replace(/[^0-9a-f]/g, "");
}

/**
 * Reads the report of `zkey verify`: the circuit hash and, oldest first, every contribution with its
 * name, hash and, for the beacon, the beacon hash and iterations exponent.
 * Strict: a message that looks like a contribution but does not have exactly the expected shape (for
 * example a name containing a newline) is an error, never silently skipped.
 */
export function parseZkeyVerify(messages) {
  const texts = messages.map((m) => (typeof m === "string" ? m : m.message));
  let circuitHash = null;
  const found = [];
  let last = null;
  for (const text of texts) {
    if (text.startsWith("Circuit Hash")) {
      const m = TITLED.exec(text);
      if (!m) throw new Error("malformed circuit hash message from snarkjs");
      circuitHash = blockHex(m[2]);
    } else if (text.startsWith("contribution #")) {
      const m = TITLED.exec(text);
      const head = m && /^contribution #(\d+)(?: (.*))?$/.exec(m[1]);
      if (!head) throw new Error(`malformed contribution message from snarkjs (a name with a line break?): ${JSON.stringify(text.slice(0, 120))}`);
      last = { index: Number(head[1]), name: head[2] ?? "", hash: blockHex(m[2]) };
      found.push(last);
    } else if (last && text.startsWith("Beacon generator: ")) {
      if (!/^Beacon generator: [0-9a-f]+$/.test(text)) throw new Error("malformed beacon message from snarkjs");
      last.beaconHash = text.slice("Beacon generator: ".length);
    } else if (last && text.startsWith("Beacon iterations Exp: ")) {
      if (!/^Beacon iterations Exp: \d+$/.test(text)) throw new Error("malformed beacon message from snarkjs");
      last.beaconIterationsExp = Number(text.slice("Beacon iterations Exp: ".length));
    }
  }
  const ok = texts.includes("ZKey Ok!");
  const contributions = found.sort((a, b) => a.index - b.index);
  contributions.forEach((c, i) => {
    if (c.index !== i + 1) throw new Error(`zkey verify report skips contribution #${i + 1}`);
  });
  return { ok, circuitHash, contributions };
}

/** The hash a contributor must publish, from the messages of `zkey contribute` or `zkey beacon`. */
export function parseContributionHash(messages) {
  for (const m of messages) {
    const text = typeof m === "string" ? m : m.message;
    if (!text.startsWith("Contribution Hash")) continue;
    const t = TITLED.exec(text);
    if (!t) throw new Error("malformed contribution hash message from snarkjs");
    return blockHex(t[2]);
  }
  throw new Error("snarkjs printed no contribution hash");
}
