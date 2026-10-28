//! Conversion from gnark's compressed key and uncompressed proof to Solana BE points.
use crate::Error;
use groth16_solana::{
    decompression::{decompress_g1, decompress_g2},
    groth16::{negate_g1_be, Groth16Verifyingkey},
};

// BN254 base field; reject noncanonical coordinates before negating A.
const FQ: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
];

pub(crate) struct Proof {
    pub(crate) a: [u8; 64],
    pub(crate) b: [u8; 128],
    pub(crate) c: [u8; 64],
}

pub(crate) fn load_proof(bytes: &[u8]) -> Result<Proof, Error> {
    for coordinate in bytes.chunks_exact(32) {
        if coordinate >= FQ.as_slice() {
            return Err(Error::InvalidPoint);
        }
    }
    let mut a = [0; 64];
    let mut b = [0; 128];
    let mut c = [0; 64];
    a.copy_from_slice(&bytes[..64]);
    b.copy_from_slice(&bytes[64..192]);
    c.copy_from_slice(&bytes[192..]);
    // SP1's AffineG1/AffineG2 parser rejects infinity; the syscall accepts it.
    if a == [0; 64] || b == [0; 128] || c == [0; 64] {
        return Err(Error::InvalidPoint);
    }
    Ok(Proof {
        a: negate_g1_be(&a),
        b,
        c,
    })
}

/// gnark's flag is in the high bits of the first BE byte. Solana's BE
/// decompressor uses arkworks flags in that same byte; no limb swap is needed.
fn ark_flags<const N: usize>(bytes: &[u8]) -> Result<[u8; N], Error> {
    let mut out = [0; N];
    out.copy_from_slice(bytes);
    let flag = match out[0] & 0xc0 {
        0x80 => 0x00,
        0xc0 => 0x80,
        0x40 => 0x40,
        _ => return Err(Error::InvalidVerifyingKey),
    };
    out[0] = (out[0] & 0x3f) | flag;
    Ok(out)
}

fn g1(bytes: &[u8]) -> Result<[u8; 64], Error> {
    decompress_g1(&ark_flags(bytes)?).map_err(|_| Error::InvalidVerifyingKey)
}

fn g2(bytes: &[u8]) -> Result<[u8; 128], Error> {
    decompress_g2(&ark_flags(bytes)?).map_err(|_| Error::InvalidVerifyingKey)
}

pub(crate) fn load_verifying_key(
    bytes: &[u8; 492],
) -> Result<(Groth16Verifyingkey<'static>, [[u8; 64]; 6]), Error> {
    // gnark serializes alpha(G1), beta(G1), beta(G2), gamma(G2), delta(G1),
    // delta(G2), K count(u32 BE), six compressed K points, empty commitments.
    // beta(G1) and delta(G1) are not used in the pairing equation.
    if bytes[288..292] != 6u32.to_be_bytes() || bytes[484..] != [0; 8] {
        return Err(Error::InvalidVerifyingKey);
    }
    let mut ic = [[0; 64]; 6];
    for (point, compressed) in ic.iter_mut().zip(bytes[292..484].chunks_exact(32)) {
        *point = g1(compressed)?;
    }
    Ok((
        Groth16Verifyingkey {
            nr_pubinputs: 5,
            vk_alpha_g1: g1(&bytes[..32])?,
            vk_beta_g2: g2(&bytes[64..128])?,
            vk_gamma_g2: g2(&bytes[128..192])?,
            vk_delta_g2: g2(&bytes[224..288])?,
            vk_ic: &[],
            vk_commitment: None,
        },
        ic,
    ))
}
