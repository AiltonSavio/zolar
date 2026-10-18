//! Equihash (200,9) verification split into resumable steps.
//!
//! The stock verifier needs about 4.46M compute units; Solana allows 1.4M per
//! transaction. This module checks the same conditions as the `equihash`
//! crate's `is_valid_solution`, but walks the 512 leaves left to right and
//! merges them like a binary counter, so the only state carried between steps
//! is a 10-node stack, the BLAKE2b midstate and a table of seen indices. All of
//! it lives in a caller-supplied byte buffer ([`STATE_LEN`] bytes, zeroed).

use crate::blake2b::{self, INPUT_LEN, LEAF_BYTES};

const N: u32 = 200;
const K: u32 = 9;
const COLLISION_BITS: usize = 20;
const INDEX_BITS: usize = 21;
const LEAVES: usize = 1 << K;
/// Bytes of the compactSize that prefixes the solution (`fd 40 05` = 1344).
pub const SOLUTION_PREFIX: [u8; 3] = [0xfd, 0x40, 0x05];
pub const SOLUTION_LEN: usize = LEAVES * INDEX_BITS / 8;
/// Serialised header: 140 bytes, compactSize, 1344-byte solution.
pub const HEADER_LEN: usize = INPUT_LEN + SOLUTION_PREFIX.len() + SOLUTION_LEN;

const STACK_SLOTS: usize = K as usize + 1;
const NODE_LEN: usize = 32;
const SEEN_SLOTS: usize = 1024;

const OFF_STAGE: usize = 0;
const OFF_DEPTH: usize = 1;
const OFF_NEXT: usize = 2;
const OFF_MID: usize = 8;
const OFF_STACK: usize = OFF_MID + 64;
const OFF_SEEN: usize = OFF_STACK + STACK_SLOTS * NODE_LEN;
/// Size of the state buffer.
pub const STATE_LEN: usize = OFF_SEEN + SEEN_SLOTS * 4;

const STAGE_RUNNING: u8 = 1;
const STAGE_VERIFIED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Header is not `HEADER_LEN` bytes or its solution prefix is wrong.
    MalformedHeader,
    /// State buffer is not `STATE_LEN` bytes.
    MalformedState,
    /// `step` called before `begin`, or after the solution was verified.
    WrongStage,
    /// Two subtrees do not collide on the required 20 bits.
    Collision,
    /// A subtree's first index is not smaller than its sibling's.
    OutOfOrder,
    /// The same index appears twice.
    DuplicateIndex,
    /// The root hash is not zero.
    NonZeroRoot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// This many leaves are done; call `step` again.
    Pending(u16),
    Verified,
}

#[derive(Clone, Copy)]
struct Node {
    hash: [u8; LEAF_BYTES],
    level: u8,
    first: u32,
}

fn input_of(header: &[u8]) -> Result<&[u8; INPUT_LEN], Error> {
    if header.len() != HEADER_LEN || header[INPUT_LEN..INPUT_LEN + 3] != SOLUTION_PREFIX {
        return Err(Error::MalformedHeader);
    }
    header[..INPUT_LEN]
        .try_into()
        .map_err(|_| Error::MalformedHeader)
}

fn index_at(solution: &[u8], j: usize) -> u32 {
    let bit = j * INDEX_BITS;
    let byte = bit / 8;
    let mut v = 0u32;
    for k in 0..4 {
        v = (v << 8) | solution.get(byte + k).copied().unwrap_or(0) as u32;
    }
    (v >> (32 - INDEX_BITS - bit % 8)) & ((1 << INDEX_BITS) - 1)
}

fn load_node(state: &[u8], slot: usize) -> Node {
    let o = OFF_STACK + slot * NODE_LEN;
    let mut hash = [0u8; LEAF_BYTES];
    hash.copy_from_slice(&state[o..o + LEAF_BYTES]);
    let mut first = [0u8; 4];
    first.copy_from_slice(&state[o + 28..o + 32]);
    Node {
        hash,
        level: state[o + LEAF_BYTES],
        first: u32::from_le_bytes(first),
    }
}

fn store_node(state: &mut [u8], slot: usize, node: &Node) {
    let o = OFF_STACK + slot * NODE_LEN;
    state[o..o + LEAF_BYTES].copy_from_slice(&node.hash);
    state[o + LEAF_BYTES] = node.level;
    state[o + 28..o + 32].copy_from_slice(&node.first.to_le_bytes());
}

/// True when the first `bits` bits of `a ^ b` are zero.
fn xor_prefix_zero(a: &[u8; LEAF_BYTES], b: &[u8; LEAF_BYTES], bits: usize) -> bool {
    let full = bits / 8;
    if a[..full] != b[..full] {
        return false;
    }
    let rest = bits % 8;
    rest == 0 || (a[full] ^ b[full]) >> (8 - rest) == 0
}

fn insert_seen(state: &mut [u8], index: u32) -> Result<(), Error> {
    let want = index + 1;
    let mut slot = (index.wrapping_mul(0x9E37_79B1) >> 22) as usize;
    loop {
        let o = OFF_SEEN + slot * 4;
        let mut cur = [0u8; 4];
        cur.copy_from_slice(&state[o..o + 4]);
        match u32::from_le_bytes(cur) {
            0 => {
                state[o..o + 4].copy_from_slice(&want.to_le_bytes());
                return Ok(());
            }
            v if v == want => return Err(Error::DuplicateIndex),
            _ => slot = (slot + 1) % SEEN_SLOTS,
        }
    }
}

/// Resets `state` for the header's solution and computes the BLAKE2b midstate.
pub fn begin(header: &[u8], state: &mut [u8]) -> Result<(), Error> {
    let input = input_of(header)?;
    if state.len() != STATE_LEN {
        return Err(Error::MalformedState);
    }
    state.fill(0);
    let mid = blake2b::midstate(input, N, K);
    for (i, w) in mid.iter().enumerate() {
        state[OFF_MID + i * 8..OFF_MID + i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    state[OFF_STAGE] = STAGE_RUNNING;
    Ok(())
}

/// Whether [`step`] already accepted this header's solution.
pub fn is_verified(state: &[u8]) -> bool {
    state.len() == STATE_LEN && state[OFF_STAGE] == STAGE_VERIFIED
}

/// Hashes and merges up to `max_leaves` further leaves.
pub fn step(header: &[u8], state: &mut [u8], max_leaves: u16) -> Result<Progress, Error> {
    let input = input_of(header)?;
    if state.len() != STATE_LEN {
        return Err(Error::MalformedState);
    }
    if state[OFF_STAGE] != STAGE_RUNNING {
        return Err(Error::WrongStage);
    }
    let solution = &header[INPUT_LEN + SOLUTION_PREFIX.len()..];
    let mut mid = [0u64; 8];
    for (i, w) in mid.iter_mut().enumerate() {
        let mut b = [0u8; 8];
        b.copy_from_slice(&state[OFF_MID + i * 8..OFF_MID + i * 8 + 8]);
        *w = u64::from_le_bytes(b);
    }
    let mut next = u16::from_le_bytes([state[OFF_NEXT], state[OFF_NEXT + 1]]) as usize;
    let mut depth = state[OFF_DEPTH] as usize;
    let stop = (next + max_leaves as usize).min(LEAVES);

    while next < stop {
        let index = index_at(solution, next);
        insert_seen(state, index)?;
        let mut node = Node {
            hash: blake2b::leaf_hash(&mid, input, index),
            level: 0,
            first: index,
        };
        next += 1;
        while depth > 0 {
            let left = load_node(state, depth - 1);
            if left.level != node.level {
                break;
            }
            depth -= 1;
            node = merge(&left, &node)?;
        }
        store_node(state, depth, &node);
        depth += 1;
    }

    state[OFF_DEPTH] = depth as u8;
    state[OFF_NEXT..OFF_NEXT + 2].copy_from_slice(&(next as u16).to_le_bytes());
    if next == LEAVES {
        state[OFF_STAGE] = STAGE_VERIFIED;
        Ok(Progress::Verified)
    } else {
        Ok(Progress::Pending(next as u16))
    }
}

fn merge(a: &Node, b: &Node) -> Result<Node, Error> {
    let level = a.level as usize + 1;
    if !xor_prefix_zero(&a.hash, &b.hash, level * COLLISION_BITS) {
        return Err(Error::Collision);
    }
    if level == K as usize && !xor_prefix_zero(&a.hash, &b.hash, LEAF_BYTES * 8) {
        return Err(Error::NonZeroRoot);
    }
    if b.first < a.first {
        return Err(Error::OutOfOrder);
    }
    let mut hash = [0u8; LEAF_BYTES];
    for (out, (x, y)) in hash.iter_mut().zip(a.hash.iter().zip(&b.hash)) {
        *out = x ^ y;
    }
    Ok(Node {
        hash,
        level: a.level + 1,
        first: a.first,
    })
}
