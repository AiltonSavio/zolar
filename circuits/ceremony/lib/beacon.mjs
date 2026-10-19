// The public random beacon: the hash of a Zcash mainnet block whose height is announced in the
// transcript before the last contribution closes. Its hash is computed here from the block header,
// not copied from an explorer's summary.
import { createHash } from "node:crypto";

const sha256 = (b) => createHash("sha256").update(b).digest();

/** Block hash (explorer order, 64 hex) of a Zcash header with its Equihash solution: hex of 140 + 3 + 1344 bytes. */
export function blockHashFromHeader(headerHex) {
  const header = Buffer.from(headerHex, "hex");
  if (header.length !== 140 + 3 + 1344 || header[140] !== 0xfd || header.readUInt16LE(141) !== 1344) {
    throw new Error("not a Zcash header with a 1344-byte Equihash solution");
  }
  return Buffer.from(sha256(sha256(header))).reverse().toString("hex");
}

/** Reads the header from a raw block (hex) and returns its hash. */
export function blockHashFromRawBlock(rawHex) {
  return blockHashFromHeader(rawHex.slice(0, 2 * (140 + 3 + 1344)));
}

const API = "https://api.blockchair.com/zcash";
export const MIN_DEPTH = 100;

async function getJson(url, attempts = 4) {
  let last;
  for (let i = 0; i < attempts; i++) {
    try {
      const res = await fetch(url, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(30_000) });
      if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
      return await res.json();
    } catch (e) {
      last = new Error(`${url}: ${e.message}${e.cause ? ` (${e.cause.message ?? e.cause})` : ""}`);
      await new Promise((r) => setTimeout(r, 2000 * (i + 1)));
    }
  }
  throw last;
}

/**
 * Fetches block `height` of Zcash mainnet from Blockchair, recomputes its hash from the raw header and
 * refuses unless the explorer's own hash agrees and the block is at least MIN_DEPTH blocks deep.
 * Anyone must still compare the hash with a second explorer of their choice.
 */
export async function fetchZcashBlockHash(height) {
  const tip = (await getJson(`${API}/blocks?limit=1`)).data[0].id;
  if (tip - height < MIN_DEPTH) throw new Error(`block ${height} is only ${tip - height} blocks deep (tip ${tip}); wait for ${MIN_DEPTH}`);
  const raw = (await getJson(`${API}/raw/block/${height}`)).data[String(height)].raw_block;
  const computed = blockHashFromRawBlock(raw);
  const listed = (await getJson(`${API}/dashboards/block/${height}`)).data[String(height)].block.hash;
  if (computed !== listed) throw new Error(`explorer hash ${listed} differs from the hash of the header (${computed})`);
  return computed;
}
