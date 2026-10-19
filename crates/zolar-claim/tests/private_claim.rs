//! The Private Claim circuit input against the independent JavaScript reference
//! (`circuits/scripts/claim-input.mjs`, circomlibjs and node:crypto), for a real transaction of the
//! ticket 08 devnet swap in a synthetic Final Block Tree.

use sha2::{Digest, Sha256};
use zolar_claim::private_claim::{circuit_input, claim_window_lower, ClaimTree, Witness};
use zolar_claim::{block_leaf, tx_merkle_root, FinalBlockTree};

const FIXTURE: &str = include_str!("../../../circuits/fixtures/swap2-block.json");
const EXPECTED_INPUT: &str = include_str!("../../../circuits/fixtures/swap2-d2.input.json");

fn internal(display: &str) -> [u8; 32] {
    let mut b: [u8; 32] = hex::decode(display).unwrap().try_into().unwrap();
    b.reverse();
    b
}

/// The tree of `circuits/scripts/claim-input.mjs --checkpoint 4000000 --leaves 6 --slot 3`.
fn synthetic_tree(real_root: &[u8; 32]) -> FinalBlockTree {
    let leaves = (0..6u32)
        .map(|k| {
            let root: [u8; 32] = if k == 3 {
                *real_root
            } else {
                Sha256::digest(format!("zolar-fixture-block-{k}")).into()
            };
            block_leaf(4_000_001 + k, &root).unwrap()
        })
        .collect();
    FinalBlockTree::new(leaves)
}

fn fixture() -> (Vec<[u8; 32]>, usize, [u8; 32]) {
    let f: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let txids: Vec<[u8; 32]> = f["txids_display"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| internal(t.as_str().unwrap()))
        .collect();
    let salt = hex::decode(f["salt"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    (txids, f["index"].as_u64().unwrap() as usize, salt)
}

#[test]
fn the_circuit_input_equals_the_javascript_reference() {
    let (txids, index, salt) = fixture();
    let tree = synthetic_tree(&tx_merkle_root(&txids));
    let input = circuit_input(&Witness {
        txids: &txids,
        index,
        salt,
        height: 4_000_004,
        tx_depth: 2,
        tree: ClaimTree {
            tree: &tree,
            size: 6,
            leaf: 3,
        },
        lower: 4_000_000,
        expiry: 4_000_017,
    })
    .unwrap();
    let expected: serde_json::Value = serde_json::from_str(EXPECTED_INPUT).unwrap();
    assert_eq!(input.json, expected);
}

#[test]
fn the_window_lower_bound_matches_the_reveal_claim_rule() {
    // Reveal Claim accepts a block iff block + finality_depth > created_tip.
    for created_tip in [0u32, 1, 2, 9, 10, 11, 100, u32::MAX - 1] {
        for depth in [0u16, 1, 2, 10, 100] {
            let lower = claim_window_lower(created_tip, depth);
            for block in lower.saturating_sub(2)..=lower.saturating_add(2) {
                let rule = block as u64 + depth as u64 > created_tip as u64;
                assert_eq!(
                    block >= lower,
                    rule,
                    "tip {created_tip} depth {depth} block {block}"
                );
            }
        }
    }
}

#[test]
fn a_path_longer_than_the_circuit_is_refused() {
    let (txids, index, salt) = fixture();
    let tree = synthetic_tree(&tx_merkle_root(&txids));
    let witness = Witness {
        txids: &txids,
        index,
        salt,
        height: 4_000_004,
        tx_depth: 1,
        tree: ClaimTree {
            tree: &tree,
            size: 6,
            leaf: 3,
        },
        lower: 4_000_000,
        expiry: 4_000_017,
    };
    assert!(circuit_input(&witness).is_err());
}

#[test]
fn a_leaf_outside_the_chosen_tree_is_an_error_not_a_panic() {
    let (txids, index, salt) = fixture();
    let tree = synthetic_tree(&tx_merkle_root(&txids));
    for (size, leaf) in [(3usize, 3usize), (7, 3), (0, 0)] {
        let witness = Witness {
            txids: &txids,
            index,
            salt,
            height: 4_000_004,
            tx_depth: 2,
            tree: ClaimTree {
                tree: &tree,
                size,
                leaf,
            },
            lower: 4_000_000,
            expiry: 4_000_017,
        };
        assert!(circuit_input(&witness).is_err(), "size {size} leaf {leaf}");
    }
}
