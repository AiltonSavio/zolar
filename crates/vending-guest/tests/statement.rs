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
