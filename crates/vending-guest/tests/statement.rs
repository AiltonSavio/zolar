mod common;
use common::{fixture, rebind};
use vending_guest::{verify, Error};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

#[test]
fn complete_statement_commits_only_the_contract_values() {
    let w = fixture("valid");
    let pv = verify(&w).unwrap();
    assert_eq!(pv.machine, w.machine);
    assert_eq!(pv.nonce, w.nonce);
    assert_eq!(pv.payout, w.payout);
    assert_eq!(pv.value_zat, 2_500_000);
    assert_eq!(pv.root, w.root);
    assert_eq!(pv.height, w.height);
    assert_eq!(pv.diversifier, w.diversifier);
    assert_eq!(pv.branch_id, 0x77190ad9);
    assert_eq!(pv.encode().len(), 249);
}
#[test]
fn changed_plaintexts_with_valid_encryption_but_old_cmx_are_rejected() {
    for kind in ["value", "rseed", "rho"] {
        assert_eq!(verify(&fixture(kind)), Err(Error::Decryption), "{kind}");
    }
}
#[test]
fn wrong_diversifier_key_memo_machine_nonce_are_rejected() {
    let mut w = fixture("valid");
    w.diversifier[0] ^= 1;
    assert_eq!(verify(&w), Err(Error::Diversifier));
    let mut w = fixture("valid");
    w.shop_viewing_key[32] ^= 1;
    assert!(verify(&w).is_err());
    assert_eq!(verify(&fixture("memo")), Err(Error::Memo));
    let mut w = fixture("valid");
    w.machine[0] ^= 1;
    assert_eq!(verify(&w), Err(Error::Machine));
    let mut w = fixture("valid");
    w.nonce[0] ^= 1;
    assert_eq!(verify(&w), Err(Error::Nonce));
}
#[test]
fn paths_roots_indices_and_trailing_tx_data_are_bound() {
    let mut w = fixture("valid");
    w.tx_count = 2;
    w.tx_path.push([7; 32]);
    assert_eq!(verify(&w), Err(Error::TxPath));
    let mut w = fixture("valid");
    w.root[0] ^= 1;
    assert_eq!(verify(&w), Err(Error::TreePath));
    let mut w = fixture("valid");
    w.tree_path[0] = [255; 32];
    assert_eq!(verify(&w), Err(Error::TreePath));
    let mut w = fixture("valid");
    w.tree_index = 1 << 20;
    assert_eq!(verify(&w), Err(Error::TreePath));
    let mut w = fixture("valid");
    w.tx_index = 1;
    assert_eq!(verify(&w), Err(Error::TxPath));
    let mut w = fixture("valid");
    w.tx.push(0);
    assert_eq!(verify(&w), Err(Error::Transaction));
}
#[test]
fn recorded_payment_txid_and_decryption_hold_but_text_memo_is_rejected() {
    let mut w = fixture("valid");
    w.tx = common::real_tx();
    rebind(&mut w);
    let tx = Transaction::read(w.tx.as_slice(), BranchId::Nu7).unwrap();
    assert_eq!(
        tx.txid().to_string(),
        "df172741ca487bd157b4fd2f31bef6168707944fc56ff885a520eea62128d9a7"
    );
    assert_eq!(verify(&w), Err(Error::Memo));
}
#[test]
fn leaf_hashes_match_the_existing_independent_circomlibjs_fixture() {
    let v: serde_json::Value = serde_json::from_str(include_str!(
        "../../relay-tests/fixtures/final-block-tree-nu7-4465014-4465270.json"
    ))
    .unwrap();
    for l in v["leaves"].as_array().unwrap() {
        let root: [u8; 32] = hex::decode(l["merkleRoot"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            hex::encode(
                zolar_claim::block_leaf(l["height"].as_u64().unwrap() as u32, &root).unwrap()
            ),
            l["leaf"].as_str().unwrap()
        );
    }
}

#[test]
fn guest_folds_nonzero_tree_indices_with_the_shared_tree_builder() {
    let mut w = fixture("valid");
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../relay-tests/fixtures/final-block-tree-nu7-4465014-4465270.json"
    ))
    .unwrap();
    let mut leaves: Vec<[u8; 32]> = reference["leaves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            hex::decode(l["leaf"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap()
        })
        .collect();
    w.tree_index = 77;
    leaves[77] = zolar_claim::block_leaf(w.height, &w.block_merkle_root).unwrap();
    let size = leaves.len();
    let tree = zolar_claim::FinalBlockTree::new(leaves);
    w.tree_path = tree.path(77, size).try_into().unwrap();
    w.root = tree.root(size);
    assert_eq!(verify(&w).unwrap().root, w.root);
    w.tree_index = 76;
    assert_eq!(verify(&w), Err(Error::TreePath));
}

#[test]
fn transaction_paths_bind_right_directions_and_odd_duplication() {
    let mut w = fixture("valid");
    let tx = Transaction::read(w.tx.as_slice(), BranchId::Nu7).unwrap();
    let txid = *tx.txid().as_ref();
    let txids = [[1; 32], [2; 32], txid];
    w.tx_index = 2;
    w.tx_count = 3;
    w.tx_path = zolar_claim::tx_merkle_path(&txids, 2);
    w.block_merkle_root = zolar_claim::tx_merkle_root(&txids);
    assert!(vending_guest::check_tx_path(&txid, &w).is_ok());
    w.tx_path[0] = [9; 32];
    assert_eq!(vending_guest::check_tx_path(&txid, &w), Err(Error::TxPath));
}

#[test]
fn payout_is_bound_and_noncanonical_text_memos_are_rejected() {
    let mut w = fixture("valid");
    w.payout[0] ^= 1;
    assert_eq!(verify(&w), Err(Error::Payout));
    for kind in ["uppercase", "spaces", "base58"] {
        assert_eq!(verify(&fixture(kind)), Err(Error::Memo), "{kind}");
    }
}
