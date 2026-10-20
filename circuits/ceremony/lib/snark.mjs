// Thin wrappers over the pinned snarkjs API. Each returns what the transcript needs, never trusting
// a file's own claims: contribution hashes come from snarkjs re-deriving them while verifying.
import * as snarkjs from "snarkjs";
import { captureLogger, parseContributionHash, parseZkeyVerify } from "./snarkjs-log.mjs";

export async function newZkey({ r1cs, ptau, out, echo }) {
  const log = captureLogger(echo);
  await snarkjs.zKey.newZKey(r1cs, ptau, out, log);
}

/** Full `zkey verify`: the key is derived from the R1CS and the ptau and every contribution checks. */
export async function verifyZkey({ r1cs, ptau, zkey, echo }) {
  const log = captureLogger(echo);
  const ok = await snarkjs.zKey.verifyFromR1cs(r1cs, ptau, zkey, log);
  const report = parseZkeyVerify(log.messages);
  return { ...report, ok: ok === true && report.ok };
}

export async function contribute({ zkeyIn, zkeyOut, name, entropy, echo }) {
  const log = captureLogger(echo);
  await snarkjs.zKey.contribute(zkeyIn, zkeyOut, name, entropy, log);
  return parseContributionHash(log.messages);
}

export async function beacon({ zkeyIn, zkeyOut, name, hash, iterationsExp, echo }) {
  const log = captureLogger(echo);
  const ok = await snarkjs.zKey.beacon(zkeyIn, zkeyOut, name, hash, iterationsExp, log);
  if (ok === false) throw new Error("snarkjs refused the beacon");
  return parseContributionHash(log.messages);
}

export async function exportVerificationKey(zkey) {
  return snarkjs.zKey.exportVerificationKey(zkey, captureLogger(false));
}

export async function verifyPtau(ptau, echo) {
  return (await snarkjs.powersOfTau.verify(ptau, captureLogger(echo))) === true;
}
