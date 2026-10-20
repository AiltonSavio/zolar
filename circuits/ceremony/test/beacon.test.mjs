import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { blockHashFromHeader, blockHashFromRawBlock } from "../lib/beacon.mjs";

const header = readFileSync(new URL("./fixtures/zcash-mainnet-3522201-header.hex", import.meta.url), "utf8").trim();

test("the hash of mainnet block 3522201 is recomputed from its header", () => {
  assert.equal(blockHashFromHeader(header), "00000000006143a0c057a33990d9d4eff5820f86d3ead0e9c65b9f42f4a131c2");
  assert.equal(blockHashFromRawBlock(`${header}deadbeef`), blockHashFromHeader(header));
});

test("a header with a changed byte hashes differently, a short one is refused", () => {
  const changed = header.slice(0, 100) + (header[100] === "0" ? "1" : "0") + header.slice(101);
  assert.notEqual(blockHashFromHeader(changed), blockHashFromHeader(header));
  assert.throws(() => blockHashFromHeader(header.slice(0, 200)), /not a Zcash header/);
});
