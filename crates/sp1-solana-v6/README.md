# SP1 v6 on Solana

`no_std` SP1 v6.10.0 Groth16 verification using the same
`groth16-solana` revision as Zolar's Escrow:
`ac9d959f9b14479c20c9d4f8168fa4cc47c13071`. On SBF, hashing, decompression,
BN254 arithmetic and pairings use Solana syscalls without heap allocation. No SP1 SDK or proving
dependencies are built.

```rust,ignore
sp1_solana_v6::verify_proof(&proof, &public_values, &vkey_hash)?;
```

`vkey_hash: [u8; 32]` is the trusted guest's `vk.bytes32()` decoded from hex;
retain its leading byte. `public_values` is the guest's raw committed bytes
(249 bytes for vending; this library does not interpret their application fields).
`proof` is the SDK's `SP1ProofWithPublicValues::bytes()`, exactly 356 bytes:

| Offset | Bytes | Encoding |
| --- | --- | --- |
| 0 | 4 | SHA-256(verifying key) selector `4388a21c` |
| 4 | 32 | exit code, big-endian, must be zero |
| 36 | 32 | trusted recursion VK root, big-endian |
| 68 | 32 | proof nonce, big-endian BN254 scalar |
| 100 | 64 | uncompressed gnark A, not negated |
| 164 | 128 | uncompressed gnark B |
| 292 | 64 | uncompressed gnark C |

Public inputs are `[vkey_hash, masked_hash(public_values), exit_code, vk_root,
proof_nonce]`. As in SP1's Rust verifier, SHA-256 is tried first, then BLAKE3;
the high three digest bits are cleared (`hash[0] &= 0x1f`). The nonce is bound
by the pairing, never replaced with zero. It is not a replay-prevention token:
vending must still enforce its Payment Tag and Reservation rules.

The recursion root is fixed to
`002f850ee998974d6cc00e50cd0814b098c05bfade466d28573240d057f25352`.
No arbitrary VK/root override or unchecked verification entrypoint is exposed.
Malformed lengths, noncanonical scalars/coordinates and infinity encodings fail.

## Sources and fixtures

The conversion follows [sp1-solana at 4181cae](https://github.com/succinctlabs/sp1-solana/tree/4181cae00d7493ede8a33066cb56683acb1dca72/verifier).
The five-input policy, hash masking, exit-code policy, root and embedded 492-byte
key follow [SP1 v6.10.0 at d83ed91](https://github.com/succinctlabs/sp1/tree/d83ed91f96701209613720708f36ffce0e98ed33/crates/verifier).
The key's SHA-256 is
`4388a21c687fdd5f218d7e3d13190cac4c5355818d3605fd5fb811df468ee696`.

The v6.10.0 release has no committed wrapped Groth16 fixture: its verifier tests
generate proofs at runtime. The first-party [project-template fixture at d1b0924](https://github.com/succinctlabs/sp1-project-template/blob/d1b0924587efd7f4b6c2797d6fdd8282da71d088/contracts/src/fixtures/groth16-fixture.json)
is v6.0.0, with selector `0e78f4db` and a different recursion root; it is included
as `v6_0_proof.bin` and must be rejected.

The successful fixture comes from **the contributor-authored, unmerged
[sp1-solana PR #24](https://github.com/succinctlabs/sp1-solana/pull/24), not an
official v6.10.0 release fixture**. Its pinned
[fibonacci_proof.bin at db4b894](https://github.com/succinctlabs/sp1-solana/blob/db4b89415b7b4992be9c220f2518aa147b1e60af/proofs/fibonacci_proof.bin)
reports SP1 `v6.1.0`; its full Groth16 VK hash and recursion root equal v6.10.0's.
Native/SBF acceptance establishes compatibility with that real wrapped proof;
it does not establish that a vending guest proof has been produced or verified.
No proving was performed to create these test assets.

`upstream_sp1_proof.bin` preserves the exact 1,707-byte bincode source, SHA-256
`9fb91cc514d6667ba0cd83d96be51d507867371c0bbbcd7119151f375eeb0a45`.
`proof.bin`, `public_values.bin` (12 bytes), and `vkey_hash.bin` are extracted
without compiling the SDK. `scripts/check_fixture.py` reproduces and checks the
extraction, all five public inputs, metadata and source/key hashes offline.

The root equality check addresses [Code4rena M-01](https://code4rena.com/reports/2025-09-succinct).
That report's M-06 is nonce clobbering during execution-record packing, which
rejects valid multi-shard executions; this verifier does no packing and binds
the supplied nonce exactly. Tests mutate both the root and nonce. The older
[SP1 advisory on missing root checks](https://github.com/succinctlabs/sp1/security/advisories/GHSA-6248-228x-mmvh)
explains why pairing verification alone does not authenticate an arbitrary root.

## Verification

Run from the repository root. Before each build/test/clippy, check `free -g`;
if available memory is below 3 GB, wait. Every Cargo invocation must use:

```sh
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 -- env CARGO_BUILD_JOBS=2 cargo <arguments>
```

Arguments for the native library:

```text
build -p sp1-solana-v6 --locked
fmt -p sp1-solana-v6 --check
clippy -p sp1-solana-v6 --all-targets --locked -- -D warnings
test -p sp1-solana-v6 --locked
```

Check extraction with `python3 -I crates/sp1-solana-v6/scripts/check_fixture.py`.

SBF adapter and LiteSVM harness have separate workspaces/lockfiles, keeping the
SBF rustc 1.84 dependency graph separate from the host's LiteSVM graph:

```text
build-sbf --manifest-path crates/sp1-solana-v6/tests/program/Cargo.toml -- --locked
fmt --manifest-path crates/sp1-solana-v6/tests/program/Cargo.toml --check
clippy --manifest-path crates/sp1-solana-v6/tests/program/Cargo.toml --all-targets --locked -- -D warnings
fmt --manifest-path crates/sp1-solana-v6/tests/litesvm/Cargo.toml --check
clippy --manifest-path crates/sp1-solana-v6/tests/litesvm/Cargo.toml --all-targets --locked -- -D warnings
test --manifest-path crates/sp1-solana-v6/tests/litesvm/Cargo.toml --locked -- --nocapture
```

The harness loads `tests/program/target/deploy/sp1_v6_test_program.so`; build it
first. It reports verifier instruction CU (subtracting the compute-budget
instruction's 150 CU), enforces a 200k CU success budget, and checks adversarial
inputs through the actual SBF entrypoint. Negative pairing checks may need two
pairings for SP1's hash fallback, so transactions request the 1.4M CU cap.

Measured on 2026-10-26: **143,197 CU** for the successful proof with its 12-byte
public values; altered public values, guest key or nonce cost **239,110 CU**
before rejection. Native: 14 passing tests. LiteSVM: 4 passing tests. These
measurements cover this fixture and the test adapter, not the complete vending
Claim instruction or a proof with vending's 249-byte public values.

For consumers building with the older SBF toolchain, preserve compatible lockfile
versions: `zeroize 1.8.2`, `zeroize_derive 1.4.3`, `blake3 1.8.2`,
`solana-hash 4.0.0`. The included test-program lockfile pins these. The library
does not impose these transitive versions on unrelated native workspace crates.

This port has not received an independent security audit. No devnet deployment
or end-to-end vending proof is part of these tests.

Ported code and embedded upstream artifacts retain the MIT notices in
`LICENSE-MIT` (Bhargav Annem / Succinct Labs).
