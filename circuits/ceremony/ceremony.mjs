#!/usr/bin/env node
// Phase-2 ceremony tool for the Private Claim circuit (ticket 12). See ceremony/README.md.
//
//   contributor:  contribute --in <url|path> --sha256 <hex> --name <name>
//   anyone:       verify
//   coordinator:  init, add, announce-beacon, beacon, finalize, next
//   CI:           check-vk
import { randomBytes } from "node:crypto";
import os from "node:os";
import { copyFile, mkdir, readFile, rename, stat as fsStat, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline/promises";
import { fileURLToPath } from "node:url";
import { fetchZcashBlockHash } from "./lib/beacon.mjs";
import { digestFile, exists, fetchVerified } from "./lib/files.mjs";
import * as snark from "./lib/snark.mjs";
import { assertName, CHAIN, MIN_CONTRIBUTIONS, compareReport, FORMAT, normalizeBeaconHash, SAFE_URL, validateTranscript } from "./lib/transcript.mjs";

// snarkjs starts one wasm worker per CPU, each holding hundreds of MB; ZOLAR_THREADS caps that.
const threads = Number(process.env.ZOLAR_THREADS ?? 0);
if (threads > 0) os.cpus = () => Array.from({ length: threads }, () => ({ model: "capped", speed: 0, times: {} }));

const here = dirname(fileURLToPath(import.meta.url));
const circuits = resolve(here, "..");
const repo = resolve(circuits, "..");
const config = JSON.parse(await readFile(join(here, "config.json"), "utf8"));
const snarkjsVersion = JSON.parse(await readFile(join(circuits, "package.json"), "utf8")).dependencies.snarkjs;
const dataRoot = resolve(process.env.ZOLAR_DATA ?? join(repo, "../../.zolar-data/t10"));

const say = (...a) => console.log(...a);
const fail = (msg) => {
  console.error(`error: ${msg}`);
  process.exit(1);
};

function parseArgs(argv) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) out._.push(a);
    else if (a.includes("=")) out[a.slice(2, a.indexOf("="))] = a.slice(a.indexOf("=") + 1);
    else if (argv[i + 1] === undefined || argv[i + 1].startsWith("--")) out[a.slice(2)] = true;
    else out[a.slice(2)] = argv[++i];
  }
  return out;
}

/** A switch is on only when given bare (`--offline`), never as `--offline=false`. */
const flag = (args, name) => args[name] === true;

const shellQuote = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;

function paths(args) {
  const depth = Number(args.depth ?? config.productionDepth);
  if (!Number.isInteger(depth) || depth < 1) fail("--depth must be a positive integer");
  const production = depth === config.productionDepth;
  if (!production && !(args.transcript && args.vk)) {
    fail(`depth ${depth} is a test instance: pass --transcript <file> and --vk <file> so it never touches ceremony/transcript.json or circuits/verification_key.json`);
  }
  const name = `private_claim_d${depth}`;
  return {
    depth,
    production,
    policy: { minIterationsExp: production ? config.beacon.iterationsExp : 10 },
    name,
    transcript: resolve(args.transcript ?? join(here, "transcript.json")),
    dir: resolve(args.dir ?? join(dataRoot, "ceremony", `d${depth}`)),
    r1cs: resolve(args.r1cs ?? join(dataRoot, "build", `d${depth}`, `${name}.r1cs`)),
    vk: resolve(args.vk ?? join(circuits, "verification_key.json")),
  };
}

/** Reads and validates a transcript; it must be for this circuit depth and use a pinned Powers of Tau. */
async function loadTranscript(file, p) {
  const t = validateTranscript(JSON.parse(await readFile(file, "utf8")), p.policy);
  if (t.circuit.depth !== p.depth) fail(`${file} is for depth ${t.circuit.depth}, not depth ${p.depth}`);
  const pin = config.ptaus.find((x) => x.sha256 === t.ptau.sha256);
  if (!pin || ["file", "power", "blake2b512", "url"].some((k) => pin[k] !== t.ptau[k])) {
    fail("the transcript's Powers of Tau is not one of the pins in ceremony/config.json");
  }
  if (p.production && t.ptau.power !== config.productionPtauPower) fail(`production uses the 2^${config.productionPtauPower} Powers of Tau`);
  return t;
}

async function saveTranscript(file, t, policy) {
  validateTranscript(t, policy);
  await writeFile(`${file}.tmp`, `${JSON.stringify(t, null, 2)}\n`);
  await rename(`${file}.tmp`, file);
}

const zkeyFile = (dir, round, last) => join(dir, last ? "claim_final.zkey" : `claim_${String(round).padStart(4, "0")}.zkey`);

async function stat(file) {
  return { sha256: await digestFile(file), bytes: (await fsStat(file)).size };
}

async function r1csSha(p, production) {
  const sha256 = await digestFile(p.r1cs);
  if (production) {
    const pinned = (await readFile(join(circuits, "private_claim_d8.r1cs.sha256"), "utf8")).split(/\s+/)[0];
    if (sha256 !== pinned) fail(`${p.r1cs} has sha256 ${sha256}, the repository pins ${pinned} (circuits/private_claim_d8.r1cs.sha256)`);
  }
  return sha256;
}

function showVerified(report) {
  say(`  circuit hash ${report.circuitHash.slice(0, 32)}...`);
  for (const c of report.contributions) say(`  #${c.index} ${c.name}  ${c.hash.slice(0, 32)}...${c.beaconHash ? "  (beacon)" : ""}`);
}

// ---------------------------------------------------------------- coordinator

async function cmdInit(args) {
  const p = paths(args);
  if (!args.ptau) fail("init needs --ptau <file>");
  if (await exists(p.transcript)) fail(`${p.transcript} exists; a ceremony starts once`);
  const ptauSha = await digestFile(args.ptau);
  const pin = config.ptaus.find((x) => x.sha256 === ptauSha);
  if (!pin) fail(`${args.ptau} (sha256 ${ptauSha}) is not one of the pinned Powers of Tau files in ceremony/config.json`);
  if (p.production && pin.power !== config.productionPtauPower) fail(`production needs the 2^${config.productionPtauPower} file`);
  if ((await digestFile(args.ptau, "blake2b512")) !== pin.blake2b512) fail("blake2b512 of the ptau does not match the pin");
  const r1csSha256 = await r1csSha(p, p.production);
  await mkdir(p.dir, { recursive: true });
  const out = zkeyFile(p.dir, 0);
  say("creating the initial zkey (snarkjs groth16 setup) ...");
  await snark.newZkey({ r1cs: p.r1cs, ptau: args.ptau, out, echo: flag(args, "verbose") });
  say("verifying it against the R1CS and the ptau ...");
  const report = await snark.verifyZkey({ r1cs: p.r1cs, ptau: args.ptau, zkey: out });
  if (!report.ok || report.contributions.length !== 0) fail("the initial zkey does not verify");
  const { file, power, sha256, blake2b512, url, source } = pin;
  const t = {
    format: FORMAT,
    circuit: { name: p.name, depth: p.depth, r1csSha256, circuitHash: report.circuitHash },
    ptau: { file, power, sha256, blake2b512, url, source },
    snarkjs: snarkjsVersion,
    rounds: [{ round: 0, kind: "init", file: "claim_0000.zkey", ...(await stat(out)), date: new Date().toISOString() }],
  };
  await saveTranscript(p.transcript, t, p.policy);
  say(`round 0 written: ${out}\n  sha256 ${t.rounds[0].sha256}\nUpload it, then: ceremony.mjs set-url --round 0 --url <https url>`);
}

async function cmdSetUrl(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  const r = t.rounds[Number(args.round)];
  if (!r) fail("--round does not exist");
  if (!SAFE_URL.test(args.url ?? "")) fail("--url must be a plain https url (letters, digits and . _ ~ : / ? & = % + @ -)");
  r.url = args.url;
  await saveTranscript(p.transcript, t, p.policy);
  say(`round ${r.round} url set`);
}

async function cmdAdd(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  if (t.rounds.at(-1).kind === "beacon" || t.closed) fail("contributions are closed");
  if (args.url && !SAFE_URL.test(args.url)) fail("--url must be a plain https url");
  if (!args.in || !args.hash) fail("add needs --in <file|https url> --hash <contribution hash the contributor published> [--sha256 <hex>] [--url <public url of the stored file>]");
  const ptau = resolve(args.ptau ?? join(dataRoot, "ptau", t.ptau.file));
  const round = t.rounds.length;
  const dest = zkeyFile(p.dir, round);
  await mkdir(p.dir, { recursive: true });
  if (/^https:\/\//.test(args.in)) {
    if (!args.sha256) fail("a url needs --sha256 (the contributor's file hash)");
    await fetchVerified({ url: args.in, dest, sha256: args.sha256, log: say });
  } else {
    await copyFile(args.in, dest);
    if (args.sha256 && (await digestFile(dest)) !== args.sha256) fail("file hash differs from --sha256");
  }
  say("verifying the whole chain against the R1CS and the ptau ...");
  const report = await snark.verifyZkey({ r1cs: p.r1cs, ptau, zkey: dest });
  const next = report.contributions.at(-1);
  if (!report.ok || !next) fail("snarkjs rejects this zkey");
  if (next.beaconHash !== undefined) fail("this contribution is a beacon");
  const announced = String(args.hash).replace(/[^0-9a-f]/gi, "").toLowerCase();
  if (announced !== next.hash) fail(`the contribution hash in the file (${next.hash}) is not the one the contributor announced`);
  const name = args.name ?? next.name;
  assertName(name);
  const candidate = structuredClone(t);
  candidate.rounds.push({ round, kind: "contribution", name, contributionHash: next.hash, file: `claim_${String(round).padStart(4, "0")}.zkey`, ...(await stat(dest)), date: new Date().toISOString(), ...(args.url ? { url: args.url } : {}) });
  validateTranscript(candidate, p.policy);
  const problems = compareReport(candidate, report);
  if (problems.length) fail(`the zkey does not extend the transcript:\n  ${problems.join("\n  ")}`);
  await saveTranscript(p.transcript, candidate, p.policy);
  showVerified(report);
  say(`round ${round} (${name}) accepted: ${dest}\n  sha256 ${candidate.rounds[round].sha256}`);
}

async function cmdAnnounceBeacon(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  const height = Number(args.height);
  if (!Number.isInteger(height) || height < 1) fail("--height must be a Zcash mainnet block height");
  if (t.beaconPlan) fail(`the beacon is already announced (block ${t.beaconPlan.height}); an announcement is never changed`);
  if (!flag(args, "offline")) {
    let tip;
    try {
      tip = (await (await fetch("https://api.blockchair.com/zcash/blocks?limit=1")).json()).data[0].id;
    } catch (e) {
      fail(`could not read the Zcash tip (${e.message}); retry`);
    }
    if (height <= tip + 100) fail(`height ${height} must be more than 100 blocks past the tip (${tip})`);
    say(`tip ${tip}; ${height - tip} blocks to wait, about ${Math.round(((height - tip) * 75) / 3600)} hours`);
  }
  t.beaconPlan = { chain: CHAIN, height, announced: new Date().toISOString() };
  await saveTranscript(p.transcript, t, p.policy);
  say(`beacon announced: ${CHAIN} block ${height}.\nCommit and push the transcript and post the height publicly NOW, before the last contribution closes.`);
}

/** Ends the contributions: from now on `add` refuses, and the beacon applies to exactly this file. */
async function cmdClose(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  if (!t.beaconPlan) fail("announce the beacon first (announce-beacon)");
  if (t.closed) fail("already closed");
  const last = t.rounds.at(-1);
  const contributions = t.rounds.filter((r) => r.kind === "contribution").length;
  if (last.kind !== "contribution" || contributions < MIN_CONTRIBUTIONS) fail(`${contributions} contributions; ${MIN_CONTRIBUTIONS} are required`);
  if (!flag(args, "offline")) {
    let tip;
    try {
      tip = (await (await fetch("https://api.blockchair.com/zcash/blocks?limit=1")).json()).data[0].id;
    } catch (e) {
      fail(`could not read the Zcash tip (${e.message}); retry`);
    }
    if (tip >= t.beaconPlan.height) fail(`block ${t.beaconPlan.height} is already mined (tip ${tip}): the beacon is no longer unknown`);
  }
  t.closed = { round: last.round, zkeySha256: last.sha256, at: new Date().toISOString() };
  await saveTranscript(p.transcript, t, p.policy);
  say(`contributions closed at round ${last.round}, file sha256 ${last.sha256}.\nCommit and push, and post this hash publicly (a timestamped post) BEFORE block ${t.beaconPlan.height} is mined.`);
}

async function cmdBeacon(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  if (!t.beaconPlan) fail("announce the beacon first (announce-beacon)");
  if (!t.closed) fail("close the contributions first (close)");
  if (t.rounds.at(-1).kind === "beacon") fail("the beacon is applied");
  const ptau = resolve(args.ptau ?? join(dataRoot, "ptau", t.ptau.file));
  if (args.hash && p.production) fail("--hash is for rehearsals; the production beacon is fetched and recomputed from the block header");
  if (args.iterations && p.production) fail(`the production beacon hashes 2^${config.beacon.iterationsExp} times`);
  const hash = normalizeBeaconHash(args.hash ?? (await fetchZcashBlockHash(t.beaconPlan.height)));
  const iterationsExp = Number(args.iterations ?? config.beacon.iterationsExp);
  const round = t.rounds.length;
  const prior = zkeyFile(p.dir, round - 1);
  if ((await digestFile(prior)) !== t.closed.zkeySha256) fail("the last contribution on disk is not the file the contributions were closed on");
  const out = zkeyFile(p.dir, round, true);
  say(`applying beacon: ${CHAIN} block ${t.beaconPlan.height} = ${hash}, 2^${iterationsExp} hash iterations`);
  const contributionHash = await snark.beacon({ zkeyIn: prior, zkeyOut: out, name: "Beacon", hash, iterationsExp });
  const report = await snark.verifyZkey({ r1cs: p.r1cs, ptau, zkey: out });
  const candidate = structuredClone(t);
  candidate.rounds.push({
    round, kind: "beacon", name: "Beacon", contributionHash, file: "claim_final.zkey", ...(await stat(out)), date: new Date().toISOString(),
    beacon: { chain: CHAIN, height: t.beaconPlan.height, hash, iterationsExp },
  });
  validateTranscript(candidate, p.policy);
  const problems = compareReport(candidate, report);
  if (problems.length) fail(`final zkey does not match:\n  ${problems.join("\n  ")}`);
  await saveTranscript(p.transcript, candidate, p.policy);
  showVerified(report);
  say(`final zkey: ${out}\n  sha256 ${candidate.rounds[round].sha256}`);
}

/** snarkjs's own file layout for a verification key. */
function vkText(vk) {
  const strings = JSON.parse(JSON.stringify(vk, (_, v) => (typeof v === "bigint" ? v.toString() : v)));
  return `${JSON.stringify(strings, null, 1)}\n`;
}

async function cmdFinalize(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  const last = t.rounds.at(-1);
  if (last.kind !== "beacon") fail("apply the beacon first");
  const zkey = zkeyFile(p.dir, last.round, true);
  if ((await digestFile(zkey)) !== last.sha256) fail("the final zkey on disk is not the transcript's");
  const text = vkText(await snark.exportVerificationKey(zkey));
  await writeFile(p.vk, text);
  t.final = { zkeySha256: last.sha256, verificationKeyFile: "circuits/verification_key.json", verificationKeySha256: await digestFile(p.vk) };
  await saveTranscript(p.transcript, t, p.policy);
  say(`verification key written to ${p.vk}\n  sha256 ${t.final.verificationKeySha256}`);
  if (p.production) {
    await writeFile(join(here, "config.json"), `${JSON.stringify({ ...config, ceremonyComplete: true }, null, 2)}\n`);
    say("config.json: ceremonyComplete = true (CI now requires the transcript and the final key)");
  }
  if (flag(args, "install")) {
    if (!p.production) fail("--install is for the production ceremony only");
    say("installing it in the Escrow (gen_vk) ...");
    const r = spawnSync("cargo", ["run", "-p", "zolar-claim", "--example", "gen_vk", "--", p.vk, join(repo, "programs/escrow/src")], {
      cwd: repo, stdio: "inherit", env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "2" },
    });
    if (r.status !== 0) fail("gen_vk failed");
  } else {
    say("install it:  cargo run -p zolar-claim --example gen_vk -- circuits/verification_key.json programs/escrow/src");
  }
}

async function cmdNext(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  const last = t.rounds.at(-1);
  if (last.kind === "beacon") fail("the ceremony is closed");
  if (!last.url) fail(`round ${last.round} has no url yet (set-url)`);
  say(`Contribution ${last.round + 1}. Previous file: ${last.url}\n  sha256 ${last.sha256}  (${(last.bytes / 1e6).toFixed(0)} MB)\n`);
  say(`  node circuits/ceremony/ceremony.mjs contribute --in ${shellQuote(last.url)} --sha256 ${last.sha256} --name "<your name>"`);
}

// ---------------------------------------------------------------- contributor

async function cmdContribute(args) {
  if (!args.in || !args.name) fail('contribute needs --in <url|file> --name "<your name or handle>" (and --sha256 for a url)');
  assertName(args.name);
  const outDir = resolve(args.dir ?? ".");
  const out = resolve(args.out ?? join(outDir, "my_contribution.zkey"));
  await mkdir(dirname(out), { recursive: true });
  if (await exists(out)) fail(`${out} already exists; move it away (a second run must not overwrite your first contribution)`);
  let input = resolve(args.in);
  if (input === out) fail("--out must be a different file from --in");
  if (/^https?:\/\//.test(args.in)) {
    if (!args.sha256) fail("--sha256 is required with a url: it is the hash the coordinator published for the previous file");
    if (!SAFE_URL.test(args.in)) fail("the url must be plain https");
    input = join(dirname(out), "previous.zkey");
    if (input === out) fail("--out must not be previous.zkey");
    await fetchVerified({ url: args.in, dest: input, sha256: args.sha256.toLowerCase(), log: say });
  } else if (args.sha256 && (await digestFile(input)) !== args.sha256.toLowerCase()) {
    fail("the file's sha256 is not the one you were given");
  }
  let typed = "";
  if (process.stdin.isTTY && !flag(args, "no-prompt")) {
    const rl = createInterface({ input: process.stdin, output: process.stdout });
    typed = await rl.question("Type some random text (mash the keyboard, anything; it is mixed with the computer's randomness, never stored): ");
    rl.close();
  }
  const entropy = [typed, randomBytes(64).toString("hex"), process.hrtime.bigint().toString(), Date.now().toString()].join("|");
  say("contributing (this is the slow step; do not interrupt) ...");
  const hash = await snark.contribute({ zkeyIn: input, zkeyOut: out, name: args.name, entropy, echo: flag(args, "verbose") });
  const { sha256, bytes } = await stat(out);
  const receipt = [
    `Zolar phase-2 ceremony contribution by ${args.name}`,
    `contribution hash (publish this):`,
    `  ${hash}`,
    `file: ${out}`,
    `file sha256: ${sha256}`,
    `file bytes: ${bytes}`,
    `previous file sha256: ${args.sha256 ?? "(local file)"}`,
  ].join("\n");
  await writeFile(`${out}.receipt.txt`, `${receipt}\n`);
  say(`\n${receipt}\n`);
  say("Next: (1) send the file to the coordinator, (2) publish the contribution hash and the file sha256 somewhere public under your own name (a post, a gist), (3) delete the file and this terminal's history of the typed text.");
}

// ---------------------------------------------------------------- anyone

async function cmdVerify(args) {
  const p = paths(args);
  const t = await loadTranscript(p.transcript, p);
  const last = t.rounds.at(-1);
  const closed = last.kind === "beacon";
  await mkdir(dirname(p.r1cs), { recursive: true });
  const ptau = resolve(args.ptau ?? join(dataRoot, "ptau", t.ptau.file));
  say(`1/5 Powers of Tau ${t.ptau.file} (${t.ptau.source})`);
  await fetchVerified({ url: t.ptau.url, dest: ptau, sha256: t.ptau.sha256, log: say });
  if ((await digestFile(ptau, "blake2b512")) !== t.ptau.blake2b512) fail("ptau blake2b512 differs from the transcript");
  if (flag(args, "check-ptau")) {
    say("    verifying the Powers of Tau chain with snarkjs (slow) ...");
    if (!(await snark.verifyPtau(ptau, false))) fail("snarkjs powersoftau verify failed");
  }
  say(`2/5 R1CS ${t.circuit.name}`);
  if (!(await exists(p.r1cs)) || (await digestFile(p.r1cs)) !== t.circuit.r1csSha256) {
    if (t.circuit.r1csUrl) await fetchVerified({ url: t.circuit.r1csUrl, dest: p.r1cs, sha256: t.circuit.r1csSha256, log: say });
    else fail(`build the R1CS first: circuits/scripts/build.sh ${p.depth} (needs circom 2.2.3), or pass --r1cs; its sha256 must be ${t.circuit.r1csSha256}`);
  }
  if (p.production) {
    const pinned = (await readFile(join(circuits, "private_claim_d8.r1cs.sha256"), "utf8")).split(/\s+/)[0];
    if (pinned !== t.circuit.r1csSha256) fail("the transcript's R1CS is not the one pinned in the repository");
  }
  say(`3/5 zkey of round ${last.round}`);
  const zkey = resolve(args.zkey ?? zkeyFile(p.dir, last.round, closed));
  await fetchVerified({ url: last.url, dest: zkey, sha256: last.sha256, log: say });
  say("4/5 snarkjs zkey verify against the R1CS and the ptau, then the transcript");
  const report = await snark.verifyZkey({ r1cs: p.r1cs, ptau, zkey });
  const problems = compareReport(t, report);
  if (problems.length) fail(`TRANSCRIPT DOES NOT MATCH THE KEY:\n  ${problems.join("\n  ")}`);
  showVerified(report);
  if (!closed) {
    say(`\nPARTIAL: ${report.contributions.length} contributions verify, but the ceremony has no beacon yet, so this key is not final.`);
    return;
  }
  say("5/5 verification key");
  const text = vkText(await snark.exportVerificationKey(zkey));
  const committed = await readFile(p.vk, "utf8");
  if (JSON.stringify(JSON.parse(committed)) !== JSON.stringify(JSON.parse(text))) fail(`${p.vk} is not the key exported from the final zkey`);
  if (!t.final || (await digestFile(p.vk)) !== t.final.verificationKeySha256) fail("the committed verification key file differs from the transcript's final key");
  const b = last.beacon;
  if (flag(args, "offline")) {
    say(`\nOFFLINE CHECK ONLY: the chain of ${report.contributions.length - 1} contributions and the beacon parameters verify,`);
    say(`but beacon block ${b.height} was NOT recomputed. This is not a full verification; run again online.`);
    return;
  }
  const live = await fetchZcashBlockHash(b.height).catch((e) => fail(`could not recompute the beacon block ${b.height}: ${e.message} (retry, or use --offline for a partial check)`));
  if (live !== b.hash) fail(`beacon: Zcash block ${b.height} has hash ${live}, the transcript says ${b.hash}`);
  say(`    beacon block ${b.height} recomputed from its header: ${live}`);
  say(`\nTRANSCRIPT OK: ${report.contributions.length - 1} contributions and a beacon, key matches ${p.vk}`);
  say(`Check the beacon yourself: ${b.chain} block ${b.height} must have hash ${b.hash} (e.g. https://blockchair.com/zcash/block/${b.height}).`);
  say("Check your own contribution hash is in the list above, and that every contributor's public post matches theirs.");
}

async function cmdCheckVk(args) {
  const p = paths(args);
  const required = config.ceremonyComplete || flag(args, "require-complete");
  if (!(await exists(p.transcript))) {
    if (required) fail("the ceremony is marked complete (config.json) but there is no transcript");
    return say("no transcript yet: the committed key is still the development key");
  }
  const t = await loadTranscript(p.transcript, p);
  if (!t.final) {
    if (required) fail("the ceremony is marked complete (config.json) but the transcript has no final key");
    return say("ceremony not finished: the committed key is still the development key");
  }
  const got = await digestFile(p.vk);
  if (got !== t.final.verificationKeySha256) fail(`circuits/verification_key.json has sha256 ${got}, the transcript's final key is ${t.final.verificationKeySha256}`);
  say("the committed verification key is the ceremony's final key");
}

const commands = {
  init: cmdInit, "set-url": cmdSetUrl, add: cmdAdd, "announce-beacon": cmdAnnounceBeacon, close: cmdClose, beacon: cmdBeacon,
  finalize: cmdFinalize, next: cmdNext, contribute: cmdContribute, verify: cmdVerify, "check-vk": cmdCheckVk,
};
const [cmd, ...rest] = process.argv.slice(2);
if (!commands[cmd]) {
  console.error(`usage: ceremony.mjs <${Object.keys(commands).join("|")}> [--flag value ...]   (see circuits/ceremony/README.md)`);
  process.exit(2);
}
try {
  await commands[cmd](parseArgs(rest));
  process.exit(0); // snarkjs leaves curve worker threads alive
} catch (e) {
  fail(e.message);
}
