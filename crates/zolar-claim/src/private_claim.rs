//! Private Claim: the circuit input for a local proof, the public inputs the Escrow supplies, and the
//! conversion of a snarkjs proof into the bytes `groth16-solana` verifies.
//!
//! The circuit is `circuits/src/private_claim.circom`. Its public inputs are, in order, the Final Block
//! Tree root, the Payment Commitment, the lowest allowed block height and the highest (the Escrow's
//! expiry height). Everything else is private and never leaves the Payer's machine.

use ark_bn254::{Fq, Fr};
use ark_ff::{BigInteger, PrimeField};
use serde_json::{json, Value};

use crate::{
    block_leaf, fold_tx_path, payment_commitment, tx_merkle_path, tx_merkle_root, Error,
    FinalBlockTree, DEPTH,
};

/// Lowest block height a claim may prove for an Escrow created when the relay tip was `created_tip`:
/// `block + finality_depth > created_tip`, i.e. `block >= created_tip + 1 - finality_depth`. The Escrow
/// computes the same value for the public input.
pub fn claim_window_lower(created_tip: u32, finality_depth: u16) -> u32 {
    created_tip
        .saturating_add(1)
        .saturating_sub(finality_depth as u32)
}

#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    #[error(transparent)]
    Hash(#[from] Error),
    #[error("the transaction path has {need} levels but the circuit has {have}")]
    PathTooLong { need: usize, have: usize },
    #[error("transaction index {0} is outside the block")]
    BadIndex(usize),
    #[error("the Final Block Tree has no leaf for this block at that position")]
    LeafMismatch,
    #[error("height {height} is outside the window [{lower}, {expiry}]")]
    OutsideWindow {
        height: u32,
        lower: u32,
        expiry: u32,
    },
    #[error("bad proof file: {0}")]
    BadProof(String),
}

/// A leaf of the Final Block Tree in the tree of its first `size` leaves.
pub struct ClaimTree<'a> {
    pub tree: &'a FinalBlockTree,
    pub size: usize,
    /// Position of the block's leaf (`height - checkpoint_height - 1`).
    pub leaf: usize,
}

/// What the Payer knows about a payment.
pub struct Witness<'a> {
    /// All transaction IDs of the payment's block, internal byte order.
    pub txids: &'a [[u8; 32]],
    /// Position of the payment in the block.
    pub index: usize,
    /// 32-byte big-endian salt below the BN254 modulus.
    pub salt: [u8; 32],
    pub height: u32,
    /// Levels of the circuit (`PrivateClaim(tx_depth, 20)`), 8 in production.
    pub tx_depth: usize,
    pub tree: ClaimTree<'a>,
    pub lower: u32,
    pub expiry: u32,
}

/// The four public inputs as the Escrow builds them: 32-byte big-endian field elements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicInputs {
    pub root: [u8; 32],
    pub commitment: [u8; 32],
    pub lower: u32,
    pub expiry: u32,
}

impl PublicInputs {
    pub fn to_bytes(&self) -> [[u8; 32]; 4] {
        let num = |v: u32| {
            let mut b = [0u8; 32];
            b[28..].copy_from_slice(&v.to_be_bytes());
            b
        };
        [
            self.root,
            self.commitment,
            num(self.lower),
            num(self.expiry),
        ]
    }
}

pub struct CircuitInput {
    /// The `input.json` of `circuits/scripts/prove.sh`.
    pub json: Value,
    pub public: PublicInputs,
}

fn bits(bytes: &[u8; 32]) -> Vec<u8> {
    bytes
        .iter()
        .flat_map(|b| (0..8).map(move |k| (b >> (7 - k)) & 1))
        .collect()
}

fn decimal(bytes: &[u8; 32]) -> String {
    Fr::from_be_bytes_mod_order(bytes).to_string()
}

pub fn circuit_input(w: &Witness) -> Result<CircuitInput, ClaimError> {
    if w.height < w.lower || w.height > w.expiry {
        return Err(ClaimError::OutsideWindow {
            height: w.height,
            lower: w.lower,
            expiry: w.expiry,
        });
    }
    let txid = *w.txids.get(w.index).ok_or(ClaimError::BadIndex(w.index))?;
    let path = tx_merkle_path(w.txids, w.index);
    if path.len() > w.tx_depth {
        return Err(ClaimError::PathTooLong {
            need: path.len(),
            have: w.tx_depth,
        });
    }
    let block_root = tx_merkle_root(w.txids);
    debug_assert_eq!(fold_tx_path(&txid, w.index, &path), block_root);

    let leaf = block_leaf(w.height, &block_root)?;
    if w.tree.leaf >= w.tree.size || w.tree.size > w.tree.tree.len() {
        return Err(ClaimError::LeafMismatch);
    }
    let tree_path = w.tree.tree.path(w.tree.leaf, w.tree.size);
    let root = w.tree.tree.root(w.tree.size);
    if FinalBlockTree::fold(&leaf, w.tree.leaf, &tree_path) != root {
        return Err(ClaimError::LeafMismatch);
    }
    let commitment = payment_commitment(&txid, &w.salt)?;

    let zero = [0u8; 256];
    let mut sibling = Vec::new();
    let mut is_right = Vec::new();
    let mut active = Vec::new();
    for level in 0..w.tx_depth {
        match path.get(level) {
            Some(s) => {
                sibling.push(bits(s));
                is_right.push((w.index >> level) & 1);
                active.push(1);
            }
            None => {
                sibling.push(zero.to_vec());
                is_right.push(0);
                active.push(0);
            }
        }
    }
    let json = json!({
        "root": decimal(&root),
        "commitment": decimal(&commitment),
        "lower": w.lower.to_string(),
        "expiry": w.expiry.to_string(),
        "txid": bits(&txid),
        "salt": decimal(&w.salt),
        "txSibling": sibling,
        "txIsRight": is_right,
        "txActive": active,
        "height": w.height.to_string(),
        "leafIndex": (0..DEPTH).map(|l| (w.tree.leaf >> l) & 1).collect::<Vec<_>>(),
        "treeSibling": tree_path.iter().map(decimal).collect::<Vec<_>>(),
    });
    Ok(CircuitInput {
        json,
        public: PublicInputs {
            root,
            commitment,
            lower: w.lower,
            expiry: w.expiry,
        },
    })
}

/// A Groth16 proof in the byte layout of `groth16-solana`: `a` already negated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofBytes {
    pub a: [u8; 64],
    pub b: [u8; 128],
    pub c: [u8; 64],
}

fn field(v: &Value) -> Result<Fq, ClaimError> {
    v.as_str()
        .and_then(|s| s.parse::<Fq>().ok())
        .ok_or_else(|| ClaimError::BadProof(format!("not a base field element: {v}")))
}

fn be(f: Fq) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&f.into_bigint().to_bytes_be());
    out
}

impl ProofBytes {
    /// From snarkjs `proof.json`: `pi_a` and `pi_c` are `[x, y, 1]`, `pi_b` is `[[x0, x1], [y0, y1], [1, 0]]`
    /// with `c0` first; the verifier wants `x1 || x0 || y1 || y0` and `-A`.
    pub fn from_snarkjs(proof: &str) -> Result<Self, ClaimError> {
        let v: Value =
            serde_json::from_str(proof).map_err(|e| ClaimError::BadProof(e.to_string()))?;
        let at = |name: &str, i: usize| -> &Value { &v[name][i] };
        let mut out = Self {
            a: [0; 64],
            b: [0; 128],
            c: [0; 64],
        };
        out.a[..32].copy_from_slice(&be(field(at("pi_a", 0))?));
        out.a[32..].copy_from_slice(&be(-field(at("pi_a", 1))?));
        out.c[..32].copy_from_slice(&be(field(at("pi_c", 0))?));
        out.c[32..].copy_from_slice(&be(field(at("pi_c", 1))?));
        for (k, (coord, part)) in [(0, 1), (0, 0), (1, 1), (1, 0)].into_iter().enumerate() {
            out.b[32 * k..32 * k + 32].copy_from_slice(&be(field(&v["pi_b"][coord][part])?));
        }
        Ok(out)
    }
}

/// The public inputs of a snarkjs `public.json` as 32-byte big-endian numbers.
pub fn public_from_snarkjs(public: &str) -> Result<Vec<[u8; 32]>, ClaimError> {
    let v: Vec<String> =
        serde_json::from_str(public).map_err(|e| ClaimError::BadProof(e.to_string()))?;
    v.iter()
        .map(|s| {
            s.parse::<Fr>()
                .map(|f| {
                    let mut out = [0u8; 32];
                    out.copy_from_slice(&f.into_bigint().to_bytes_be());
                    out
                })
                .map_err(|_| ClaimError::BadProof(format!("not a field element: {s}")))
        })
        .collect()
}
