import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, rename, rm, stat } from "node:fs/promises";
import { dirname } from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";

/** Streaming digest of a file (`sha256` or `blake2b512`), lowercase hex. */
export async function digestFile(path, algorithm = "sha256") {
  const hash = createHash(algorithm);
  await pipeline(createReadStream(path), hash);
  return hash.digest("hex");
}

export async function exists(path) {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

/**
 * Makes `dest` hold the bytes whose sha256 is `sha256`: reuses `dest` when it already matches, else
 * downloads `url` (or copies nothing and fails if there is no url). A wrong hash deletes the download and throws.
 */
export const MAX_DOWNLOAD_BYTES = 3 * 1024 ** 3;
const DOWNLOAD_TIMEOUT_MS = 3 * 60 * 60 * 1000;

export async function fetchVerified({ url, dest, sha256, log = () => {}, maxBytes = MAX_DOWNLOAD_BYTES }) {
  if (!/^[0-9a-f]{64}$/.test(sha256)) throw new Error(`not a sha256: ${sha256}`);
  if ((await exists(dest)) && (await digestFile(dest)) === sha256) return dest;
  if (!url) throw new Error(`${dest} is missing or has the wrong hash and no url is known`);
  if (!/^https:\/\//.test(url)) throw new Error(`refusing a non-https url: ${url}`);
  await mkdir(dirname(dest), { recursive: true });
  const tmp = `${dest}.part`;
  log(`downloading ${url}`);
  const res = await fetch(url, { redirect: "follow", signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) });
  if (!res.ok || !res.body) throw new Error(`download failed: ${res.status} ${url}`);
  if (Number(res.headers.get("content-length") ?? 0) > maxBytes) throw new Error(`${url} is larger than ${maxBytes} bytes`);
  let seen = 0;
  const limit = new Transform({
    transform(chunk, _enc, done) {
      seen += chunk.length;
      done(seen > maxBytes ? new Error(`${url} exceeded ${maxBytes} bytes`) : null, chunk);
    },
  });
  try {
    await pipeline(Readable.fromWeb(res.body), limit, createWriteStream(tmp));
  } catch (e) {
    await rm(tmp, { force: true });
    throw e;
  }
  const got = await digestFile(tmp);
  if (got !== sha256) {
    await rm(tmp, { force: true });
    throw new Error(`sha256 mismatch for ${url}: expected ${sha256}, got ${got}`);
  }
  await rename(tmp, dest);
  return dest;
}
