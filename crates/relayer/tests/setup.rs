mod common;

use std::{cell::RefCell, rc::Rc};

use common::Lite;
use header_relay_client::accounts::{Relay, Store};
use relay_tests::{
    checkpoint_before, digest_of, fixture_rows, Harness, NU7_ACTIVATION_FIXTURE, NU7_HEIGHT,
};
use solana_keypair::Keypair;
use solana_signer::Signer;
use zolar_relayer::{
    chain::{Cluster, RelayClient},
    follower::{Follower, RelayChain, Step},
    p2p::RawHeader,
    setup::{checkpoint_digest, checkpoint_from, create_relay, CONTEXT},
};

fn rows() -> Vec<(u32, Vec<u8>)> {
    fixture_rows(NU7_ACTIVATION_FIXTURE)
}

#[test]
fn a_checkpoint_built_from_live_headers_matches_the_one_the_relay_tests_build() {
    let all = rows();
    let end = 200;
    let headers: Vec<RawHeader> = all[end - CONTEXT..end]
        .iter()
        .map(|(_, h)| h.clone().try_into().unwrap())
        .collect();

    let built = checkpoint_from(&headers, all[end - 1].0, 10, 77).unwrap();

    let expected = checkpoint_before(&all, end, NU7_HEIGHT, 10, 77);
    assert_eq!(built, expected);
    assert_eq!(checkpoint_digest(&built), digest_of(&expected));
}

#[test]
fn headers_that_do_not_chain_cannot_make_a_checkpoint() {
    let all = rows();
    let mut headers: Vec<RawHeader> = all[10..10 + CONTEXT]
        .iter()
        .map(|(_, h)| h.clone().try_into().unwrap())
        .collect();
    headers.swap(5, 6);
    assert!(checkpoint_from(&headers, 0, 10, 0).is_err());
    assert!(checkpoint_from(&headers[..50], 0, 10, 0).is_err());
}

#[test]
fn a_relay_created_by_the_relayer_accepts_real_headers() {
    let all = rows();
    let end = 150;
    let headers: Vec<RawHeader> = all[end - CONTEXT..end]
        .iter()
        .map(|(_, h)| h.clone().try_into().unwrap())
        .collect();
    let checkpoint = checkpoint_from(&headers, all[end - 1].0, 10, 5).unwrap();

    let svm = Harness::new().svm;
    let mut svm = svm;
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 20_000_000_000).unwrap();
    let svm = Rc::new(RefCell::new(svm));
    let mut cluster = Lite {
        svm: svm.clone(),
        sends: 0,
        fail_sends: vec![],
    };

    let (relay, signatures) = create_relay(&mut cluster, &payer, &checkpoint).unwrap();

    assert_eq!(signatures.len(), 1 + 2 + 1 + 3);
    let account = cluster.account(&relay).unwrap().unwrap();
    let state = Relay::from_bytes(&account).unwrap();
    assert_eq!(state.state, 1, "sealed");
    assert_eq!(state.tip_height, checkpoint.height);
    let store = cluster
        .account(&Store::find_pda(&relay).0)
        .unwrap()
        .unwrap();
    assert_eq!(Store::from_bytes(&store[..]).unwrap().capacity, 400);

    let source = common::Rows {
        rows: all[end..end + 2]
            .iter()
            .map(|(_, h)| h.clone().try_into().unwrap())
            .collect(),
        forged: None,
    };
    let payer2 = Keypair::try_from(payer.to_bytes().as_slice()).unwrap();
    let mut follower = Follower::new(
        RelayClient::new(
            Lite {
                svm,
                sends: 0,
                fail_sends: vec![],
            },
            payer2,
            relay,
        ),
        source,
        |_| {},
    );
    assert_eq!(follower.step().unwrap(), Step::Progress { accepted: 2 });
    assert_eq!(follower.chain_mut().tip().unwrap().height, all[end + 1].0);
}
