//! Field access and hashing for a serialised Zcash block header.
//!
//! A header is `HEADER_LEN` bytes: 140 bytes of fields, the compactSize `fd 40 05`
//! and the 1344-byte Equihash solution. Hashes are kept in internal byte order
//! (the raw SHA-256d output); explorers print them reversed.

pub use crate::equihash::HEADER_LEN;

/// A SHA-256 implementation supplied by the caller: the Solana syscall on-chain,
/// any library off-chain.
pub type Sha256 = fn(&[u8]) -> [u8; 32];

const PREV_HASH: usize = 4;
const MERKLE_ROOT: usize = 36;
const TIME: usize = 100;
const BITS: usize = 104;

fn word(header: &[u8; HEADER_LEN], at: usize) -> u32 {
    u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
}

fn hash_at(header: &[u8; HEADER_LEN], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&header[at..at + 32]);
    out
}

/// `nVersion` as a signed integer; consensus requires at least 4.
pub fn version(header: &[u8; HEADER_LEN]) -> i32 {
    word(header, 0) as i32
}

pub const MIN_VERSION: i32 = 4;

pub fn prev_hash(header: &[u8; HEADER_LEN]) -> [u8; 32] {
    hash_at(header, PREV_HASH)
}

pub fn merkle_root(header: &[u8; HEADER_LEN]) -> [u8; 32] {
    hash_at(header, MERKLE_ROOT)
}

pub fn time(header: &[u8; HEADER_LEN]) -> u32 {
    word(header, TIME)
}

pub fn bits(header: &[u8; HEADER_LEN]) -> u32 {
    word(header, BITS)
}

/// SHA-256d of the whole serialised header, solution included.
pub fn hash(sha256: Sha256, header: &[u8; HEADER_LEN]) -> [u8; 32] {
    sha256(&sha256(header))
}
