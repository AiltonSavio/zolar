use base64::{engine::general_purpose::STANDARD, Engine};
use header_relay_interface::RelayView;
use vending_guest::{verify, Error, Witness};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

#[test]
fn real_testnet_payment_binds_payout_block_and_archived_live_relay_root() {
    let w: Witness =
        serde_json::from_str(include_str!("../fixtures/testnet/witness.json")).unwrap();
    let raw = hex::decode(include_str!("../fixtures/testnet/tx.hex").trim()).unwrap();
    assert_eq!(w.tx, raw);
    let tx = Transaction::read(raw.as_slice(), BranchId::Nu7).unwrap();
    assert_eq!(
        tx.txid().to_string(),
        "5f4dd9423abe8530be89d6d904bc5b429a9fba739de49808b072fafe17cab514"
    );
    let header = include_bytes!("../fixtures/testnet/header.bin");
    assert_eq!(w.block_merkle_root, header[36..68]);
    assert_eq!(w.tx_count, 3);
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/testnet/metadata.json")).unwrap();
    let txids: Vec<[u8; 32]> = metadata["txids_internal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            hex::decode(v.as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap()
        })
        .collect();
    assert_eq!(zolar_claim::tx_merkle_root(&txids), w.block_merkle_root);
    assert_eq!(
        zolar_claim::tx_merkle_path(&txids, w.tx_index as usize),
        w.tx_path
    );
    let pv = verify(&w).unwrap();
    assert_eq!(pv.value_zat, 100000);
    assert_eq!(pv.machine, [2; 32]);
    assert_eq!(pv.nonce, [3; 32]);
    assert_eq!(pv.payout, [4; 32]);
    assert_eq!(pv.height, 4543894);
    assert_eq!(hex::encode(pv.encode()), metadata["public_values"]);
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/testnet/relay-snapshot.json")).unwrap();
    let account = &snapshot["result"]["value"];
    let data = STANDARD
        .decode(account["data"][0].as_str().unwrap())
        .unwrap();
    let owner: [u8; 32] = bs58::decode(account["owner"].as_str().unwrap())
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    let addr: [u8; 32] = bs58::decode(metadata["relay"].as_str().unwrap())
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    let relay = RelayView::parse(&owner, &addr, &addr, &data).unwrap();
    assert_eq!(relay.network(), header_relay_interface::NETWORK_TESTNET);
    assert!(relay.is_known_root(&pv.root));
    assert!(pv.height <= relay.finalized_height());
    let mut wrong_payout = w;
    wrong_payout.payout[0] ^= 1;
    assert_eq!(verify(&wrong_payout), Err(Error::Payout));
}
