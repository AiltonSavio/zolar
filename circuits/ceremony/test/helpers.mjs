import { readFileSync } from "node:fs";

const pin = JSON.parse(readFileSync(new URL("../config.json", import.meta.url), "utf8")).ptaus[0];
export const h = (c, n = 64) => c.repeat(n);

export const sampleTranscript = () => ({
  format: 1,
  circuit: { name: "private_claim_d8", depth: 8, r1csSha256: h("a"), circuitHash: h("c", 128) },
  ptau: { file: pin.file, power: pin.power, sha256: pin.sha256, blake2b512: pin.blake2b512, url: pin.url, source: pin.source },
  beaconPlan: { chain: "zcash-mainnet", height: 3600000 },
  closed: { round: 3, zkeySha256: h("5") },
  rounds: [
    { round: 0, kind: "init", sha256: h("1"), bytes: 10 },
    { round: 1, kind: "contribution", name: "Alice", contributionHash: h("e", 128), sha256: h("2"), bytes: 10 },
    { round: 2, kind: "contribution", name: "Bob", contributionHash: h("f", 128), sha256: h("3"), bytes: 10 },
    { round: 3, kind: "contribution", name: "Carol", contributionHash: h("8", 128), sha256: h("5"), bytes: 10 },
    {
      round: 4, kind: "beacon", name: "beacon", contributionHash: h("9", 128), sha256: h("4"), bytes: 10,
      beacon: { chain: "zcash-mainnet", height: 3600000, hash: h("7"), iterationsExp: 20 },
    },
  ],
});
