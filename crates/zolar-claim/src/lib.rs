//! Off-chain builders for the data a Reveal Claim carries, and the same hashes the Escrow checks.
//!
//! Byte conventions match the Header Relay and the Escrow: a transaction ID or block Merkle root is in
//! internal byte order (the order of the block's Merkle tree, the reverse of the display form), and
//! every Poseidon input is a 32-byte big-endian field element. A 256-bit value is split into two
//! 128-bit limbs, the first 16 bytes and the last 16 bytes.

use ark_bn254::Fr;
use light_poseidon::{Poseidon, PoseidonBytesHasher, PoseidonError};
use sha2::{Digest, Sha256};

/// Levels of the Final Block Tree.
pub const DEPTH: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Poseidon input is not below the BN254 modulus")]
    NotAFieldElement,
    #[error("Poseidon failed: {0}")]
    Poseidon(String),
}

impl From<PoseidonError> for Error {
    fn from(e: PoseidonError) -> Self {
        match e {
            PoseidonError::InputLargerThanModulus => Error::NotAFieldElement,
            other => Error::Poseidon(other.to_string()),
        }
    }
}

/// Poseidon (circom parameters) over BN254 of 32-byte big-endian field elements.
pub fn poseidon(inputs: &[&[u8; 32]]) -> Result<[u8; 32], Error> {
    let slices: Vec<&[u8]> = inputs.iter().map(|i| &i[..]).collect();
    let mut hasher = Poseidon::<Fr>::new_circom(inputs.len())?;
    Ok(hasher.hash_bytes_be(&slices)?)
}

fn limbs(value: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut hi = [0u8; 32];
    let mut lo = [0u8; 32];
    hi[16..].copy_from_slice(&value[..16]);
    lo[16..].copy_from_slice(&value[16..]);
    (hi, lo)
}

/// `Poseidon(txid_hi, txid_lo, salt)`; the salt must be below the BN254 modulus.
pub fn payment_commitment(txid: &[u8; 32], salt: &[u8; 32]) -> Result<[u8; 32], Error> {
    let (hi, lo) = limbs(txid);
    poseidon(&[&hi, &lo, salt])
}

/// The Final Block Tree leaf of a block: `Poseidon(height, root_hi, root_lo)`.
pub fn block_leaf(height: u32, merkle_root: &[u8; 32]) -> Result<[u8; 32], Error> {
    let mut h = [0u8; 32];
    h[28..].copy_from_slice(&height.to_be_bytes());
    let (hi, lo) = limbs(merkle_root);
    poseidon(&[&h, &hi, &lo])
}

pub fn sha256d_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut data = [0u8; 64];
    data[..32].copy_from_slice(left);
    data[32..].copy_from_slice(right);
    Sha256::digest(Sha256::digest(data)).into()
}

/// The transaction Merkle root of a block: SHA-256d pairs, an odd last node paired with itself.
pub fn tx_merkle_root(txids: &[[u8; 32]]) -> [u8; 32] {
    assert!(!txids.is_empty(), "a block has at least a coinbase");
    let mut level = txids.to_vec();
    while level.len() > 1 {
        level = next_tx_level(&level);
    }
    level[0]
}

fn next_tx_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    level
        .chunks(2)
        .map(|pair| sha256d_pair(&pair[0], pair.get(1).unwrap_or(&pair[0])))
        .collect()
}

/// The siblings from the transaction at `index` up to the Merkle root (empty for a one-transaction block).
pub fn tx_merkle_path(txids: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
    let mut level = txids.to_vec();
    let mut index = index;
    let mut path = Vec::new();
    while level.len() > 1 {
        let sibling = level.get(index ^ 1).unwrap_or(&level[index]);
        path.push(*sibling);
        level = next_tx_level(&level);
        index /= 2;
    }
    path
}

/// The root a transaction Merkle path leads to (what the Escrow computes).
pub fn fold_tx_path(txid: &[u8; 32], index: usize, path: &[[u8; 32]]) -> [u8; 32] {
    let mut node = *txid;
    for (level, sibling) in path.iter().enumerate() {
        node = if (index >> level) & 1 == 0 {
            sha256d_pair(&node, sibling)
        } else {
            sha256d_pair(sibling, &node)
        };
    }
    node
}

/// The Final Block Tree rebuilt from its leaves: depth 20, empty leaf `Poseidon(0, 0)`.
pub struct FinalBlockTree {
    leaves: Vec<[u8; 32]>,
    zeros: Vec<[u8; 32]>,
}

impl FinalBlockTree {
    pub fn new(leaves: Vec<[u8; 32]>) -> Self {
        let zero = [0u8; 32];
        let mut zeros = vec![poseidon(&[&zero, &zero]).expect("zero is a field element")];
        for level in 1..DEPTH {
            let z = zeros[level - 1];
            zeros.push(poseidon(&[&z, &z]).expect("a hash is a field element"));
        }
        Self { leaves, zeros }
    }

    /// The empty-leaf value at each of the 20 levels (level 0 is a leaf).
    pub fn zeros(&self) -> &[[u8; 32]] {
        &self.zeros
    }

    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    fn levels(&self, size: usize) -> Vec<Vec<[u8; 32]>> {
        assert!(size <= self.leaves.len());
        let mut levels = vec![self.leaves[..size].to_vec()];
        for d in 0..DEPTH {
            let prev = &levels[d];
            let next: Vec<[u8; 32]> = prev
                .chunks(2)
                .map(|pair| {
                    poseidon(&[&pair[0], pair.get(1).unwrap_or(&self.zeros[d])])
                        .expect("tree values are field elements")
                })
                .collect();
            levels.push(next);
        }
        levels
    }

    /// The root of the tree holding the first `size` leaves.
    pub fn root(&self, size: usize) -> [u8; 32] {
        let levels = self.levels(size);
        match levels[DEPTH].first() {
            Some(root) => *root,
            None => {
                let z = &self.zeros[DEPTH - 1];
                poseidon(&[z, z]).expect("a hash is a field element")
            }
        }
    }

    /// The 20 siblings of leaf `index` in the tree holding the first `size` leaves.
    pub fn path(&self, index: usize, size: usize) -> Vec<[u8; 32]> {
        assert!(index < size);
        let levels = self.levels(size);
        (0..DEPTH)
            .map(|d| {
                let i = (index >> d) ^ 1;
                levels[d].get(i).copied().unwrap_or(self.zeros[d])
            })
            .collect()
    }

    /// Drops the trailing siblings that are empty subtrees: the Escrow fills those levels in itself, which
    /// keeps a Reveal Claim inside one transaction while the tree is young.
    pub fn compress(&self, mut path: Vec<[u8; 32]>) -> Vec<[u8; 32]> {
        while path
            .last()
            .is_some_and(|last| *last == self.zeros[path.len() - 1])
        {
            path.pop();
        }
        path
    }

    /// The root a leaf and its path lead to (what the Escrow computes).
    pub fn fold(leaf: &[u8; 32], index: usize, path: &[[u8; 32]]) -> [u8; 32] {
        let mut node = *leaf;
        for (level, sibling) in path.iter().enumerate() {
            node = if (index >> level) & 1 == 0 {
                poseidon(&[&node, sibling])
            } else {
                poseidon(&[sibling, &node])
            }
            .expect("tree values are field elements");
        }
        node
    }
}
