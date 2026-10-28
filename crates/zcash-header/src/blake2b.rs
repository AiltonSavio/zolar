//! BLAKE2b specialised for Equihash (200,9) leaf hashing.
//!
//! Solana has no BLAKE2b syscall, so the compression runs as SBF code. The
//! Equihash input is 140 bytes (header prefix and nonce) plus a 4-byte counter,
//! so every leaf hash is the same first block followed by one more block that
//! only differs in the counter. [`midstate`] computes the first block once and
//! [`leaf_hash`] finishes it with a single compression per leaf.

const IV: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// Length of the Equihash hash input: 108-byte header prefix and 32-byte nonce.
pub const INPUT_LEN: usize = 140;
/// Bytes of the final block that come from the input (the rest is the counter).
const TAIL_LEN: usize = INPUT_LEN - 128;
/// Bytes of a BLAKE2b output used per Equihash (200,9) leaf pair.
pub const LEAF_BYTES: usize = 25;

#[inline(always)]
fn g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

#[inline(always)]
fn compress(h: &mut [u64; 8], m: &[u64; 16], t: u64, last: bool) {
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= t;
    if last {
        v[14] = !v[14];
    }
    macro_rules! round {
        ($r:expr) => {{
            let s = &SIGMA[$r % 10];
            g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
            g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
            g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
            g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
            g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }};
    }
    round!(0);
    round!(1);
    round!(2);
    round!(3);
    round!(4);
    round!(5);
    round!(6);
    round!(7);
    round!(8);
    round!(9);
    round!(10);
    round!(11);
    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

fn le_u64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

/// Chaining state after the first 128 bytes of `input`, for personalisation
/// `"ZcashPoW" || n_le32 || k_le32` and a 50-byte digest.
pub fn midstate(input: &[u8; INPUT_LEN], n: u32, k: u32) -> [u64; 8] {
    let mut h = IV;
    h[0] ^= 0x0101_0000 ^ 50;
    let mut personal = [0u8; 16];
    personal[..8].copy_from_slice(b"ZcashPoW");
    personal[8..12].copy_from_slice(&n.to_le_bytes());
    personal[12..].copy_from_slice(&k.to_le_bytes());
    h[6] ^= le_u64(&personal[..8]);
    h[7] ^= le_u64(&personal[8..]);
    let mut m = [0u64; 16];
    for (i, w) in m.iter_mut().enumerate() {
        *w = le_u64(&input[i * 8..]);
    }
    compress(&mut h, &m, 128, false);
    h
}

/// The 25 bytes of leaf `index`: half of the hash of counter `index / 2`.
pub fn leaf_hash(mid: &[u64; 8], input: &[u8; INPUT_LEN], index: u32) -> [u8; LEAF_BYTES] {
    let mut tail = [0u8; 16];
    tail[..TAIL_LEN].copy_from_slice(&input[128..]);
    tail[TAIL_LEN..].copy_from_slice(&(index >> 1).to_le_bytes());
    let mut m = [0u64; 16];
    m[0] = le_u64(&tail[..8]);
    m[1] = le_u64(&tail[8..]);
    let mut h = *mid;
    compress(&mut h, &m, (INPUT_LEN + 4) as u64, true);
    let mut out = [0u8; 56];
    for (i, w) in h[..7].iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    let start = (index as usize & 1) * LEAF_BYTES;
    let mut leaf = [0u8; LEAF_BYTES];
    leaf.copy_from_slice(&out[start..start + LEAF_BYTES]);
    leaf
}
