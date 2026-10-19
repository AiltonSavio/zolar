# Phase-2 ceremony

The Escrow checks Private Claim proofs against a Groth16 verification key. Whoever knows the randomness
("toxic waste") the key was made from can forge proofs and drain every Escrow. The development key
(`tools/dev-groth16`) has exactly one such person. This ceremony replaces it with a key made by several
contributors, in turn, each adding randomness of their own. **The key is safe as long as at least one
contributor was honest and deleted theirs.**

Nothing here is a Reveal Claim concern: the Reveal Claim needs no key.

## How the key is made

1. **Powers of Tau (phase 1).** The Perpetual Powers of Tau, contribution 80, as published by Privacy
   Scaling Explorations, already prepared for phase 2: `ppot_0080_20.ptau` (2^20 points, 1.2 GB). It is not ours
   and needs no trust in us: pinned by hash in [`config.json`](config.json) and in the transcript, checkable with
   `snarkjs powersoftau verify` (the `--check-ptau` flag of `verify`).
2. **Circuit.** `circuits/src/private_claim_d8.circom` compiled with circom 2.2.3 to an R1CS with
   sha256 `0c07fe00...1dc` ([`../private_claim_d8.r1cs.sha256`](../private_claim_d8.r1cs.sha256)); CI recompiles it and
   fails on any difference.
3. **Round 0** (`claim_0000.zkey`): `snarkjs groth16 setup` of the R1CS and the ptau. No secret is involved, and
   `zkey verify` recomputes it from the R1CS and the ptau, so the coordinator cannot hand out a bad start.
4. **Contributions 1..N** (at least 3): each contributor downloads the previous file, adds their randomness,
   uploads the result, and publishes a *contribution hash* under their own name.
5. **Beacon**: the coordinator closes the contributions (the hash of the last file is recorded and posted publicly), then applies one more contribution whose randomness is a
   value nobody could know in advance: the hash of a **Zcash mainnet block** whose height was written into the
   transcript (`beaconPlan`) and committed *before* the contributions close, hashed 2^20 times (snarkjs'
   beacon construction). Anyone recomputes the block hash from its header.
6. **Final key**: the verification key is exported from the final zkey into `circuits/verification_key.json`,
   the Rust constants of the Escrow are generated from it with `gen_vk`, and the transcript records the key's
   hash. CI fails if the committed key is not the transcript's final key.

## Verify everything with one command

```sh
git clone <repo> && cd zolar
pnpm --dir circuits install --frozen-lockfile     # snarkjs 0.7.6, pinned
circuits/scripts/build.sh 8                        # R1CS from the circuit; needs circom 2.2.3 and 4.5 GB of memory
node circuits/ceremony/ceremony.mjs verify         # the command
```

`verify` downloads what it lacks, each file checked against the sha256 in [`transcript.json`](transcript.json)
(the 1.2 GB ptau, the final zkey) and then:

1. checks the ptau hashes (`--check-ptau` also runs `snarkjs powersoftau verify`, slow),
2. checks the R1CS hash against the transcript and the repository,
3. runs `snarkjs zkey verify` of the final zkey against the **R1CS and the ptau**: this re-derives round 0
   and checks every contribution in the chain, the beacon included,
4. compares the contribution hashes, names and beacon parameters snarkjs recomputed with the transcript,
5. exports the verification key from the zkey and compares it with `circuits/verification_key.json`,
6. recomputes the beacon block hash from the block header (needs the network) and checks it. If it cannot, it fails; `--offline` skips this and says plainly that the check is partial.

It ends with `TRANSCRIPT OK`, or an error naming the difference. Also check by hand: **your** contribution
hash is in the list, each contributor's public post shows the same hash, and a second block explorer shows
the beacon block hash. Memory: see the table in [COORDINATOR.md](COORDINATOR.md); set `ZOLAR_THREADS=2` and
`ZOLAR_NODE_MB` to trade speed for memory.

Without the heavy inputs, CI runs the cheap half: `node circuits/ceremony/ceremony.mjs check-vk` fails if
`circuits/verification_key.json` is not the transcript's final key, and the existing `gen_vk` step fails if
the Escrow's constants are not generated from it.

## Who can cheat, and how it shows

| Actor | What they could try | What stops it |
|---|---|---|
| Phase-1 contributors (PSE, 80 of them) | Keep tau | Same 1-of-N assumption as every Groth16 system on this ptau; `powersoftau verify` checks the chain. |
| Coordinator | Hand out a bad round 0 | `zkey verify` recomputes round 0 from the R1CS and ptau. |
| Coordinator | Drop or replace a contribution, or use a different final key | Each contributor checks their hash is in the final list; contributors' public posts are compared with the transcript; the chain is verified from round 0. |
| Coordinator | Pick the beacon after seeing the contributions | The block height is committed in git before the last contribution closes, and `verify` demands the beacon is the announced block. |
| Last contributor | Know the beacon in advance | Only if they could predict a Zcash block hash; the beacon height is later than the contribution deadline. |
| Everybody but one | Collude | At least one honest contributor who deleted their randomness is enough. |
| The circuit | Be under-constrained | Not covered by any ceremony: see ticket 10's review. |

Known limits, stated plainly (they came out of a hostile review and are not fixed in code):

- **Beacon source.** The block hash is recomputed from the header Blockchair serves; the tool checks neither
  Equihash nor ancestry, so verify the hash against a second explorer yourself. A miner can withhold a block to
  bias the hash, at the price of the block reward; this only matters if every contributor was dishonest.
- **Beacon timing is evidence, not proof.** `announce-beacon` and `close` fix the order inside the transcript,
  but the real timestamp is the public post and the git history of those two commits.
- **Names and per-round file hashes are not signed.** snarkjs binds a contribution hash to the key material, not
  to a person. Identity rests on each contributor's own public post; only the final zkey is downloaded and
  verified (`rounds[].sha256` of earlier rounds is bookkeeping).
- **A hostile submission** can cost the coordinator time (a valid contribution relabelled as a beacon makes
  `zkey verify` hash 2^n times before failing). Downloads are size and time bounded; verification is not sandboxed.

Contributor instructions: [CONTRIBUTE.md](CONTRIBUTE.md). Coordinator runbook: [COORDINATOR.md](COORDINATOR.md).

## Files

- `ceremony.mjs` the tool (`contribute`, `verify`, `check-vk` for everyone; `init`, `add`, `announce-beacon`, `beacon`, `finalize`, `next`, `set-url` for the coordinator)
- `config.json` the pinned Powers of Tau files and parameters
- `transcript.json` the public record (created by `init`; absent until the real ceremony starts)
- `scripts/dry-run.sh` full rehearsal on a reduced circuit, `scripts/escrow-check.sh` the LiteSVM check of a rehearsal key
- `test/` unit tests: `node --test circuits/ceremony/test/*.test.mjs`
