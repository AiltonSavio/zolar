import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { h, sampleTranscript } from "./helpers.mjs";

const tool = fileURLToPath(new URL("../ceremony.mjs", import.meta.url));
const dir = mkdtempSync(join(tmpdir(), "check-vk-"));
const vk = join(dir, "vk.json");
writeFileSync(vk, '{"protocol":"groth16"}\n');
const vkSha = createHash("sha256").update('{"protocol":"groth16"}\n').digest("hex");

function run(transcript) {
  const file = join(dir, "t.json");
  if (transcript) writeFileSync(file, JSON.stringify(transcript));
  return spawnSync("node", [tool, "check-vk", "--transcript", file, "--vk", vk], { encoding: "utf8" });
}

test("without a transcript the check passes and says the key is the development key", () => {
  const r = spawnSync("node", [tool, "check-vk", "--transcript", join(dir, "absent.json"), "--vk", vk], { encoding: "utf8" });
  assert.equal(r.status, 0);
  assert.match(r.stdout, /development key/);
});

test("a finished transcript requires the committed key to match", () => {
  const t = sampleTranscript();
  t.final = { zkeySha256: t.rounds[4].sha256, verificationKeySha256: vkSha };
  assert.equal(run(t).status, 0);
  t.final.verificationKeySha256 = h("0");
  const r = run(t);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /final key/);
});

test("an unfinished transcript leaves the development key in place", () => {
  const t = sampleTranscript();
  t.rounds.pop();
  const r = run(t);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /not finished/);
});

test("once the ceremony is required, a missing transcript or final key fails", () => {
  const absent = spawnSync("node", [tool, "check-vk", "--require-complete", "--transcript", join(dir, "absent.json"), "--vk", vk], { encoding: "utf8" });
  assert.equal(absent.status, 1);
  assert.match(absent.stderr, /no transcript/);
  const t = sampleTranscript();
  t.rounds.pop();
  writeFileSync(join(dir, "t.json"), JSON.stringify(t));
  const open = spawnSync("node", [tool, "check-vk", "--require-complete", "--transcript", join(dir, "t.json"), "--vk", vk], { encoding: "utf8" });
  assert.equal(open.status, 1);
  assert.match(open.stderr, /no final key/);
});
