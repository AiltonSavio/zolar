use base64::{engine::general_purpose::STANDARD, Engine};
use header_relay_interface::RelayView;
use vending::{
    error::VendingError,
    rules::{validate_claim, ClaimAccounts},
    state::{Machine, Reservation, MACHINE_LEN},
};
use vending_guest::{verify, Error, Witness};
use vending_types::PublicValues;
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

#[test]
fn real_testnet_payment_roundtrips_through_program_claim_policy() {
    let w: Witness =
        serde_json::from_str(include_str!("../fixtures/testnet/witness.json")).unwrap();
    let pv = verify(&w).unwrap();
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
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/testnet/metadata.json")).unwrap();
    let addr: [u8; 32] = bs58::decode(metadata["relay"].as_str().unwrap())
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    // The guest's actual output passes the program's decoder and Claim policy.
    // This is a policy check; it does not bypass the instruction's Groth16 verifier.
    let encoded = pv.encode();
    let decoded = PublicValues::decode(&encoded).unwrap();
    assert_eq!(decoded, pv);
    assert_eq!(decoded.encode(), encoded);
    let machine_bytes = include_bytes!("../../vending-prover/tests/fixtures/machine-revision2.bin");
    let machine = Machine::read(machine_bytes).unwrap();
    let mut written = [0; MACHINE_LEN];
    machine.write(&mut written);
    assert_eq!(&written, machine_bytes);
    assert_eq!(&written[227..291], w.shop_viewing_key.as_slice());
    assert_eq!(machine.svk_hash, pv.svk_hash);
    assert_eq!(machine.diversifier, pv.diversifier);
    assert_eq!(machine.relay, addr);
    let mut reservation = Reservation {
        bump: 1,
        machine: pv.machine,
        nonce: pv.nonce,
        payout: pv.payout,
        price_zat: machine.price_zat,
        expiry_height: pv.height,
        created_at: 1_000,
        release_requested: true,
        release_requested_at: 2_000,
    };
    let check = |r: &Reservation, tag_exists| {
        validate_claim(
            &decoded,
            ClaimAccounts {
                machine_key: &pv.machine,
                machine: &machine,
                reservation: Some(r),
                relay_owner: &owner,
                relay_key: &addr,
                relay_data: &data,
                tag_exists,
            },
        )
    };
    let plan = check(&reservation, false).unwrap();
    assert_eq!(plan.payout, pv.payout);
    assert_eq!(plan.payment_tag, pv.payment_tag);
    assert_eq!(plan.tickets, 1);
    assert!(plan.close_reservation);
    assert_eq!(check(&reservation, true), Err(VendingError::TagUsed));
    reservation.payout[0] ^= 1;
    assert_eq!(check(&reservation, false), Err(VendingError::WrongPayout));
    reservation.payout = pv.payout;
    reservation.price_zat += 1;
    assert_eq!(check(&reservation, false), Err(VendingError::Underpaid));
    reservation.price_zat = machine.price_zat;
    reservation.expiry_height -= 1;
    assert_eq!(
        check(&reservation, false),
        Err(VendingError::PaymentExpired)
    );
}
