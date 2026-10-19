# Private Claim circuit

`src/private_claim.circom` is the circuit of ADR 0002: a Groth16 proof, made on the Payer's machine, that a
committed Zcash transaction is in a final block, with the transaction ID and salt never leaving that machine.
`src/private_claim_d8.circom` is the production instance (transaction trees up to 8 levels, 256 transactions;
larger blocks must use the Reveal Claim). `_d1` and `_d2` are reduced instances for tests.

Public inputs, in order: Final Block Tree `root`, Payment `commitment`, `lower` and `expiry` block heights.
The Escrow supplies the last three itself; the claimant only names the root.

## Tools (pinned)

circom 2.2.3 (release binary, sha256 `85342c7f...fe53a3`), snarkjs 0.7.6, circomlib 2.0.5, circomlibjs 0.1.7
(`pnpm --dir circuits install`), rapidsnark 0.0.8 (optional prover, much faster and leaner than snarkjs).
Large artifacts live outside git in `ZOLAR_DATA` (default `.zolar-data/t10` next to the repo checkout).
Every step runs under a hard memory cap (`ZOLAR_MEM_MAX`, default 2000M) and prints time and peak memory.

## Commands

```sh
scripts/build.sh 8                          # R1CS + wasm witness generator
scripts/setup.sh 8 $ZOLAR_DATA/ptau/ppot_0080_20.ptau   # throwaway single-party key + verification_key.json
scripts/prove.sh 8 input.json out/          # proof.json + public.json, verified locally
node scripts/claim-input.mjs fixtures/swap2-block.json --depth 8 --lower L --expiry E --out input.json
node --test test/*.test.mjs                 # circuit tests on the depth-2 build
cargo run -p zolar-claim --example gen_vk -- verification_key.json programs/escrow/src
```

`zolar_claim::private_claim::circuit_input` builds `input.json` from a block's transaction IDs in Rust;
`ProofBytes::from_snarkjs` turns `proof.json` into the bytes the Escrow takes.

## Ceremony (ticket 12)

Steps 1, 2 and 5 of `scripts/setup.sh` stay; replace the single throwaway contribution (step 3) with one
`snarkjs zkey contribute` per participant and finish with `snarkjs zkey beacon`. Publish the R1CS hash,
the ptau source and hash, every zkey hash and the `zkey verify` output. Then export the verification key,
replace `verification_key.json` here and regenerate `programs/escrow/src/verifying_key_generated.rs` with the
`gen_vk` command above; regenerate the proof fixtures with `scripts/prove.sh`.

## Proving from the Payer CLI

`zolar-payer private-claim` runs `scripts/prove.sh` as a subprocess on the Payer's machine (flags `--prove-script`, `--zolar-data`, `--mem-max`; environment `ZOLAR_PROVE_SCRIPT`, `ZOLAR_DATA`, `ZOLAR_MEM_MAX`). It needs, on that machine only:

1. `pnpm --dir circuits install` (snarkjs, for witness generation) and node 22.
2. The prover binary: `cargo build --release -p dev-groth16` (development key; `DEV_GROTH16` overrides the path `target/release/dev-groth16`). After the ceremony the zkey and rapidsnark replace it.
3. `$ZOLAR_DATA/build/d8/` holding `private_claim_d8_js/` (the wasm), `private_claim_d8.r1cs`, `dev_pk.bin` (or `claim_final.zkey`) and `verification_key.json`. A fresh user gets them in one of two ways: build them (`circuits/scripts/build.sh 8` needs circom 2.2.3 and 4.4 GB of memory, 130 s; `scripts/dev-setup.sh 8` makes the development key in 40 s), or copy an already built `build/d8/` from whoever published it, checking `private_claim_d8.r1cs.sha256` and that `verification_key.json` equals the one in this repository, because the on-chain verifying key is generated from it. On the development machine they are in `~/Work/zecathon/.zolar-data/t10`.
4. About 2 GiB free memory and about a minute (47 to 52 s on the 15 GB development machine); the prover runs in a capped systemd scope when `systemd-run` exists, otherwise set `ZOLAR_NO_CAP=1`.
