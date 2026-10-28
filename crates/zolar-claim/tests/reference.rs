//! The Final Block Tree and the Payment Commitment against independent references: the circomlibjs
//! output committed for the Header Relay tests, and the empty-tree root the program stores.

use zolar_claim::{block_leaf, tx_merkle_path, tx_merkle_root, FinalBlockTree, DEPTH};

const FIXTURE: &str =
    include_str!("../../relay-tests/fixtures/final-block-tree-nu7-4465014-4465270.json");

fn h32(s: &str) -> [u8; 32] {
    hex::decode(s).unwrap().try_into().unwrap()
}

struct Reference {
    leaves: Vec<(u32, [u8; 32], [u8; 32])>,
    roots: Vec<[u8; 32]>,
    zeros: Vec<[u8; 32]>,
}

fn reference() -> Reference {
    let json: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let leaves = json["leaves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["height"].as_u64().unwrap() as u32,
                h32(l["merkleRoot"].as_str().unwrap()),
                h32(l["leaf"].as_str().unwrap()),
            )
        })
        .collect();
    let roots = json["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| h32(r.as_str().unwrap()))
        .collect();
    let zeros = json["zeros"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| h32(r.as_str().unwrap()))
        .collect();
    Reference {
        leaves,
        roots,
        zeros,
    }
}

#[test]
fn block_leaves_equal_circomlibjs() {
    for (height, merkle_root, leaf) in reference().leaves {
        assert_eq!(block_leaf(height, &merkle_root).unwrap(), leaf);
    }
}

#[test]
fn tree_roots_equal_circomlibjs_at_every_size() {
    let r = reference();
    let leaves: Vec<[u8; 32]> = r.leaves.iter().map(|l| l.2).collect();
    let tree = FinalBlockTree::new(leaves.clone());
    assert_eq!(tree.zeros(), &r.zeros[..]);
    for size in [0, 1, 2, 3, 100, 128, leaves.len()] {
        assert_eq!(tree.root(size), r.roots[size], "size {size}");
    }
}

#[test]
fn every_path_folds_to_the_root_of_its_size() {
    let r = reference();
    let leaves: Vec<[u8; 32]> = r.leaves.iter().map(|l| l.2).collect();
    let tree = FinalBlockTree::new(leaves.clone());
    let size = leaves.len();
    for index in [0, 1, 2, 77, size - 1] {
        let path = tree.path(index, size);
        assert_eq!(path.len(), DEPTH);
        let root = FinalBlockTree::fold(&leaves[index], index, &path);
        assert_eq!(root, r.roots[size], "index {index}");
    }
    // An older size gives an older root: the path is for that tree, not the newest one.
    let path = tree.path(3, 50);
    assert_eq!(FinalBlockTree::fold(&leaves[3], 3, &path), r.roots[50]);
}

#[test]
fn transaction_merkle_tree_follows_the_block_rules() {
    // One transaction: the root is the txid. Two: SHA-256d of both. Three: the odd one pairs with itself.
    let a = [1u8; 32];
    let b = [2u8; 32];
    let c = [3u8; 32];
    assert_eq!(tx_merkle_root(&[a]), a);
    let ab = sha256d_pair(&a, &b);
    assert_eq!(tx_merkle_root(&[a, b]), ab);
    let cc = sha256d_pair(&c, &c);
    assert_eq!(tx_merkle_root(&[a, b, c]), sha256d_pair(&ab, &cc));
    for index in 0..3 {
        let path = tx_merkle_path(&[a, b, c], index);
        assert_eq!(
            zolar_claim::fold_tx_path(&[a, b, c][index], index, &path),
            tx_merkle_root(&[a, b, c])
        );
    }
}

fn sha256d_pair(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut data = [0u8; 64];
    data[..32].copy_from_slice(l);
    data[32..].copy_from_slice(r);
    Sha256::digest(Sha256::digest(data)).into()
}

#[test]
fn salt_must_be_a_field_element() {
    let txid = [7u8; 32];
    assert!(zolar_claim::payment_commitment(&txid, &[0xff; 32]).is_err());
    let salt = [9u8; 32];
    let c = zolar_claim::payment_commitment(&txid, &salt).unwrap();
    assert_ne!(
        c,
        zolar_claim::payment_commitment(&[8u8; 32], &salt).unwrap()
    );
}

#[test]
fn a_young_tree_needs_few_siblings_and_the_missing_levels_are_empty_subtrees() {
    let r = reference();
    let leaves: Vec<[u8; 32]> = r.leaves.iter().map(|l| l.2).collect();
    let tree = FinalBlockTree::new(leaves.clone());
    let size = leaves.len(); // 257 leaves: nine levels hold real nodes
    let index = size - 1;
    let short = tree.compress(tree.path(index, size));
    assert_eq!(short.len(), 9);
    let mut full = short.clone();
    full.extend_from_slice(&tree.zeros()[short.len()..DEPTH]);
    assert_eq!(
        FinalBlockTree::fold(&leaves[index], index, &full),
        r.roots[size]
    );
}
