# Vending SP1 guest

Ana's wallet pays Ben's dedicated shop address. The guest recomputes the v6
transaction ID, checks its SHA-256d transaction path and depth-20 Poseidon Final
Block Tree path, fully decrypts the selected Ironwood output with the public
Shop Viewing Key, and commits the 249-byte vending v1 public values. The
Solana program must accept the committed root against its running Header Relay.

`verify(&Witness) -> Result<vending_types::PublicValues, Error>` is the same
statement compiled natively and into SP1 v6.10.0. `Witness` uses serde/bincode
through `SP1Stdin::write(&witness)`; the entrypoint reads one `Witness` and calls
`commit_slice(&values.encode())`. There is no extra bincode length prefix in
the public values. Fields and byte orders are documented on `Witness`.

Full librustzcash decryption checks AEAD, recomputed `cmx`, and derived `epk`.
The guest supplies no plaintext value, rseed or rho as independent trusted
inputs. It requires the decrypted diversifier to equal the committed one and
strictly parses the revision 2 text memo, including its zero padding. All transaction
effects, including the encrypted memo and paying action's nullifier, enter the
recomputed txid. Funding being shielded-only remains a page check in v1.

The guest depends on `zolar-claim` with default features disabled, reusing its
actual SHA-256d/Poseidon implementation rather than a second copy. The native
suite checks leaves against the existing circomlibjs reference fixture.

## Build on the VPS

The ELF is a build artifact and is **not committed**. SP1 and the RISC-V C
compiler are installed on the VPS; the script runs without Docker:

```sh
rsync -az --exclude target/ --exclude node_modules/ --exclude .git \
  --exclude AGENTS.md --exclude CLAUDE.md ./ \
  vps:spikes/zolar-vending/worktree-guest/
ssh vps 'cd ~/spikes/zolar-vending/worktree-guest && crates/vending-guest/build-elf.sh'
```

`build-elf.sh` checks available memory, waits below 3 GiB, sets
`CC_riscv64im_succinct_zkvm_elf`, and uses a 5 GB/no-swap systemd scope on the VPS. The
output is `crates/vending-guest/elf/vending-guest`. The toolchain must be SP1
**v6.10.0**. Host-only builds use ordinary Rust without the `zkvm` feature.

## Fixtures

`ticket02-tx.hex` and `shop.ivk.hex` are the recorded Zcash-testnet transaction
and its **public incoming viewing key**, copied from the S1/S2 spike. The
recorded ticket 02 payment has unrelated text memo `zolar t02 spike`, which
correctly fails the vending parser.

`fixtures/testnet/` contains the real 0.001 TAZ revision 2 payment, raw tx,
real P2P block header and transaction path, witness, metadata and a public
Solana-devnet Relay snapshot authenticating the Final Block Tree root.
The fixture is public chain data and the public incoming viewing key only.
Its archived root ages out of the live Relay's history; the archived snapshot
records root membership at collection time. The payment is txid
`5f4dd9423abe8530be89d6d904bc5b429a9fba739de49808b072fafe17cab514`,
height **4,543,894**, value **100,000 zat**. Both inclusion paths are real.
See the prover README for execution.

`fixtures/synthetic/*.json` are explicit **non-consensus-valid** mutations of
that transaction: the original proof/signatures are retained after replacing
one encrypted output. Their block and Final Block Tree roots are synthetic.
They exercise the complete guest and failure cases, never prove a real payment,
and must never be broadcast. The valid fixture has a two-level SHA-256d path
(three synthetic transaction leaves) and a full 20-level Poseidon path.
`value` and `rseed` encrypt changed notes while retaining the original cmx;
`rho` changes the action nullifier; `memo` encrypts an unsupported prefix.
`uppercase`, `spaces` and `base58` encrypt malformed text memos. Native and SP1
tests also change the expected payout and require `Error::Payout`.

`Witness` now requires `payout: [u8;32]`, checked against the decrypted memo and
committed at public-value offsets `189..221`. Hosts must regenerate old witness
JSON/bincode; revision 1 inputs are intentionally incompatible.

Regenerate deterministically:

```sh
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- \
  env CARGO_BUILD_JOBS=2 cargo run --manifest-path crates/vending-guest/Cargo.toml \
  --example fixture -- crates/vending-guest/fixtures/synthetic
```

See `../vending-prover/README.md` for SP1 execution and negative checks.

## Revision 2 execution artifact

The full real testnet witness executes on SP1 v6.10.0 in **24,108,690 cycles**,
committing 249 bytes identical to native verification. The integration VPS CLI
execution plus program-key setup completed in **165.28 s** with a 5 GiB cap.
This is execution only; no Groth16 proof or on-chain Claim was produced.

```text
ELF SHA-256: 08dd3408160a34db63ee5a086380572c1be3b68cd735393060f789c33c9137cd
SP1 vkey:   0x000eeda101ba026e2f37c86e96abc5372ddc0330ea99d56e7ddfbef8aae711e7
```

These identify the October 27 integration build in `worktree-integration` on the
VPS. Its ELF and vkey differ from the earlier worker artifact, despite matching
cycles and public values. Pin this ELF with this vkey for a future Machine;
recompute the vkey from the exact ELF after rebuilding. No Vending Machine has
been deployed with either artifact by this integration run.

Initialize a Machine with this vkey hash for this exact guest. Rebuilding after
any source, dependency, toolchain or feature change requires checking the hash
again. The ELF stays outside git; raw transaction and witness fixtures are
committed under `fixtures/testnet/`.

## Live payment execution after deployment (October 27)

The exact ELF and vkey above are pinned by devnet Machine
`GiF2qqYCRHL6L4DVVDMPfNDk9G5f4qzovMCM62UHFjkV`. Its real testnet payment
`c9cea1a5a3da02bc89a6afc58e7132eb0ce0610938cb647a81b354c9c8ac575e`
executed at 24,103,795 cycles with 249 bytes equal to native verification.
The account comparison also checked its actual Reservation payout/nonce, shop
configuration, NU7 branch, expiry and live Relay root membership.
This is execution evidence only: no Groth16 vending proof or Claim mint exists
from this run. The root README records the deployed addresses and setup steps.
