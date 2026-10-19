# Coordinator runbook

The coordinator (Ailton) makes round 0, passes the file from contributor to contributor, verifies each result,
applies the beacon and installs the key. The coordinator cannot forge a key (see the table in
[README.md](README.md)), but can delay or censor, so every step is public.

Prefix every command with `node circuits/ceremony/ceremony.mjs`. Work files live in
`$ZOLAR_DATA/ceremony/d8` (never in git). Run heavy steps under
`systemd-run --user --scope -p MemoryMax=<cap> -p MemorySwapMax=0 --` and set `ZOLAR_THREADS` and
`ZOLAR_NODE_MB` (see the measured table in the ticket 12 runbook).

1. **Start**: `init --ptau <ppot_0080_20.ptau>`. Checks the R1CS hash against the repository and the ptau against
   the pins, runs `groth16 setup` and verifies the result. Writes `ceremony/transcript.json` and
   `claim_0000.zkey`. Commit the transcript.
2. **Publish round N** (N = 0, 1, ...): create a GitHub release `ceremony-rN` on the repository with the file as
   an asset, then `set-url --round N --url <asset download url>` and commit. `next` prints the exact command to
   send the next contributor.
3. **Add a contribution**: the contributor sends a link (or file) and their file sha256; they publish their contribution
   hash. `add --in <file|https url> --sha256 <hex> --hash <contribution hash>`. It verifies the whole
   chain against the R1CS and ptau (`snarkjs zkey verify`), checks the earlier contributions are exactly the
   transcript's, checks the new last contribution hash equals the announced one, and appends the round. Commit the
   transcript, publish the file as round N+1 (step 2).
4. **Announce the beacon before the last contribution closes**: `announce-beacon --height <H>` where H is a
   Zcash mainnet height that will be mined **well after the contribution deadline** (75 s blocks; the tool
   requires H more than 100 blocks past the tip and an announcement is never changed). **Commit and push the
   transcript with `beaconPlan` first**, and post the height publicly.
5. **Close the contributions**, after the last `add` and before block H is mined: `close`. It records the hash of
   the last contribution file in the transcript; `add` refuses afterwards and the beacon applies to exactly that
   file. Commit, push, and post the printed hash publicly (a timestamped post or gist); anyone can later see that it
   preceded H.
6. **Apply the beacon** once H is 100 blocks deep: `beacon`. It fetches block H from Blockchair, recomputes the
   block hash from the header itself, hashes it 2^20 times into a final contribution and verifies the result.
   Compare the hash with a second explorer by hand.
7. **Finalize and install**: `finalize --install` exports the verification key to `circuits/verification_key.json`,
   records its hash in the transcript, sets `ceremonyComplete` in `config.json` (from then on CI requires a transcript with a final key) and runs `gen_vk` into `programs/escrow/src`. Then regenerate the two proof
   fixtures with `scripts/prove.sh` using `ZKEY=<claim_final.zkey>` (the proofs must pass `snarkjs groth16 verify`),
   run `cargo test -p escrow-tests --test private_claim`, and delete or mark dev-only `tools/dev-groth16`.
8. **Publish**: release the final zkey as `ceremony-final` (set its url with `set-url`), commit the transcript,
   key, constants and fixtures, upgrade the devnet Escrow, and run `verify` from a clean machine.

`announce-beacon --offline`, `close --offline` and `beacon --hash <hex> --iterations <n>` exist for rehearsals only (the production beacon refuses them); `ceremony/scripts/dry-run.sh`
runs the whole flow on the depth-2 circuit, and `ceremony/scripts/escrow-check.sh` checks a rehearsal key in
LiteSVM.
