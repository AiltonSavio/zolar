//! SP1 v6.10.0 Groth16 verifier, ported from `succinctlabs/sp1-solana`.
//!
//! Accepts the SDK's 356-byte `SP1ProofWithPublicValues::bytes()` encoding:
//! `selector[4] || exit_code[32] || vk_root[32] || nonce[32] || A[64] || B[128] || C[64]`.
//! Scalars and affine coordinates are big-endian. Public values are opaque bytes.
//! The trusted guest key hash comes from `vk.bytes32()`, decoded to all 32 bytes.
#![no_std]

mod points;

use groth16_solana::groth16::{is_less_than_bn254_field_size_be, Groth16Verifier};

/// SP1 v6.10.0's gnark verification key (also used by the v6.1.0 circuit).
pub const GROTH16_VK_BYTES: &[u8; 492] = include_bytes!("groth16_vk.bin");
/// Exact SDK proof length, including the selector and three metadata scalars.
pub const PROOF_LEN: usize = 356;
/// Trusted recursion verifying-key allowlist root from SP1 v6.10.0.
pub const VK_ROOT_BYTES: [u8; 32] = [
    0x00, 0x2f, 0x85, 0x0e, 0xe9, 0x98, 0x97, 0x4d, 0x6c, 0xc0, 0x0e, 0x50, 0xcd, 0x08, 0x14, 0xb0,
    0x98, 0xc0, 0x5b, 0xfa, 0xde, 0x46, 0x6d, 0x28, 0x57, 0x32, 0x40, 0xd0, 0x57, 0xf2, 0x53, 0x52,
];

/// Errors returned by the SP1 v6 verifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidProofLength,
    Groth16VkeyHashMismatch,
    ExitCodeMismatch,
    VkeyRootMismatch,
    InvalidPublicInput,
    InvalidPoint,
    InvalidVerifyingKey,
    VerificationFailed,
}

/// Verify a successful execution of the trusted guest using SP1 v6.10.0's key.
///
/// Mirrors SP1's Rust verifier: require the key selector, trusted recursion root
/// and zero exit code; try masked SHA-256, then masked BLAKE3. The nonce is an
/// authenticated public input, with no application-specific freshness policy.
/// This function does not provide replay protection; the caller must enforce it.
pub fn verify_proof(proof: &[u8], public_values: &[u8], vkey_hash: &[u8; 32]) -> Result<(), Error> {
    if proof.len() != PROOF_LEN {
        return Err(Error::InvalidProofLength);
    }
    let vk_hash = solana_sha256_hasher::hash(GROTH16_VK_BYTES).to_bytes();
    if proof[..4] != vk_hash[..4] {
        return Err(Error::Groth16VkeyHashMismatch);
    }
    let exit_code = scalar(&proof[4..36]);
    let vk_root = scalar(&proof[36..68]);
    let nonce = scalar(&proof[68..100]);
    if vk_root != VK_ROOT_BYTES {
        return Err(Error::VkeyRootMismatch);
    }
    if exit_code != [0; 32] {
        return Err(Error::ExitCodeMismatch);
    }
    if !is_less_than_bn254_field_size_be(vkey_hash) || !is_less_than_bn254_field_size_be(&nonce) {
        return Err(Error::InvalidPublicInput);
    }

    let points::Proof { a, b, c } = points::load_proof(&proof[100..])?;
    let (vk, ic) = points::load_verifying_key(GROTH16_VK_BYTES)?;
    let vk = groth16_solana::groth16::Groth16Verifyingkey { vk_ic: &ic, ..vk };
    let mut inputs = [
        *vkey_hash,
        mask(solana_sha256_hasher::hash(public_values).to_bytes()),
        exit_code,
        vk_root,
        nonce,
    ];
    if verify_pairing(&a, &b, &c, &inputs, &vk).is_ok() {
        return Ok(());
    }
    inputs[1] = mask(solana_blake3_hasher::hash(public_values).to_bytes());
    verify_pairing(&a, &b, &c, &inputs, &vk)
}

fn verify_pairing(
    a: &[u8; 64],
    b: &[u8; 128],
    c: &[u8; 64],
    inputs: &[[u8; 32]; 5],
    vk: &groth16_solana::groth16::Groth16Verifyingkey<'_>,
) -> Result<(), Error> {
    Groth16Verifier::new(a, b, c, inputs, vk)
        .map_err(|_| Error::VerificationFailed)?
        .verify()
        .map_err(|_| Error::VerificationFailed)
}

fn scalar(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    out.copy_from_slice(bytes);
    out
}

fn mask(mut hash: [u8; 32]) -> [u8; 32] {
    // SP1 uses a 253-bit digest, not reduction modulo the BN254 scalar field.
    hash[0] &= 0x1f;
    hash
}
