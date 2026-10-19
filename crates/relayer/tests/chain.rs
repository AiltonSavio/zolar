//! The Relayer against the real Header Relay program in LiteSVM, with real NU7 testnet headers.
mod common;

use std::{cell::RefCell, rc::Rc};

use common::{Lite, Rows};
use litesvm::LiteSVM;
use relay_tests::{
    checkpoint_before, fixture_rows, Harness, FORGED_LOW_DIFFICULTY, NU7_ACTIVATION_FIXTURE,
    NU7_HEIGHT,
};
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use zolar_relayer::{
    chain::RelayClient,
    follower::{ChainError, Event, Follower, RelayChain, Step},
    p2p::{sha256d, RawHeader},
};

struct World {
    svm: Rc<RefCell<LiteSVM>>,
    relay: Address,
    payer_key: Keypair,
    rows: Vec<RawHeader>,
    /// Height of the first fixture row.
    first_height: u32,
    checkpoint_height: u32,
}

/// A relay checkpointed at fixture index `end - 1` of the NU7 activation fixture.
fn world(end: usize) -> World {
    let all = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let checkpoint = checkpoint_before(&all, end, NU7_HEIGHT, 30, 1);
    let mut harness = Harness::new();
    let relay = harness.init_relay(&checkpoint);
    let payer_key = Keypair::new();
    harness
        .svm
        .airdrop(&payer_key.pubkey(), 10_000_000_000)
        .unwrap();
    let rows = all[end..]
        .iter()
        .map(|(_, h)| h.clone().try_into().unwrap())
        .collect();
    World {
        svm: Rc::new(RefCell::new(harness.svm)),
        relay,
        payer_key,
        rows,
        first_height: all[end].0,
        checkpoint_height: checkpoint.height,
    }
}

fn client(w: &World, fail_sends: Vec<usize>) -> RelayClient<Lite> {
    let lite = Lite {
        svm: w.svm.clone(),
        sends: 0,
        fail_sends,
    };
    let payer = Keypair::try_from(w.payer_key.to_bytes().as_slice()).unwrap();
    RelayClient::new(lite, payer, w.relay)
}

fn source(w: &World, count: usize) -> Rows {
    Rows {
        rows: w.rows[..count].to_vec(),
        forged: None,
    }
}

fn buffer_exists(w: &World) -> bool {
    let buffer =
        header_relay_client::accounts::HeaderBuffer::find_pda(&w.relay, &w.payer_key.pubkey(), 0).0;
    w.svm.borrow().get_account(&buffer).is_some()
}

#[test]
fn it_relays_real_headers_end_to_end_and_leaves_no_buffer_behind() {
    let w = world(150);
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let mut follower = Follower::new(client(&w, vec![]), source(&w, 3), move |e| {
        sink.borrow_mut().push(e)
    });

    assert_eq!(follower.step().unwrap(), Step::Progress { accepted: 3 });

    let tip = follower.chain_mut().tip().unwrap();
    assert_eq!(tip.height, w.first_height + 2);
    assert_eq!(tip.hash, sha256d(&w.rows[2]));
    assert_eq!(tip.halted, 0);
    assert!(!buffer_exists(&w));
    let hash = sha256d(&w.rows[0]);
    let events = events.borrow();
    assert!(events.contains(&Event::Uploaded {
        height: w.first_height,
        hash
    }));
    assert!(events.contains(&Event::Verified {
        height: w.first_height,
        hash
    }));
    // Nothing newer: the next poll is idle.
    drop(events);
    assert_eq!(follower.step().unwrap(), Step::Idle);
}

#[test]
fn a_restarted_relayer_continues_from_the_relays_tip() {
    let w = world(150);
    let mut first = Follower::new(client(&w, vec![]), source(&w, 2), |_| {});
    first.step().unwrap();

    let mut restarted = Follower::new(client(&w, vec![]), source(&w, 4), |_| {});
    assert_eq!(restarted.step().unwrap(), Step::Progress { accepted: 2 });
    assert_eq!(
        restarted.chain_mut().tip().unwrap().height,
        w.first_height + 3
    );
}

#[test]
fn a_dropped_connection_midway_is_retried_without_leaking_the_buffer() {
    let w = world(150);
    // Sends per header: create, second, accept, close. The third send (accept) of the first header fails.
    let mut follower = Follower::new(client(&w, vec![3]), source(&w, 2), |_| {});

    assert!(matches!(follower.step(), Err(ChainError::Transient(_))));
    assert!(buffer_exists(&w), "the interrupted attempt left its buffer");

    assert_eq!(follower.step().unwrap(), Step::Progress { accepted: 2 });
    assert!(!buffer_exists(&w));
    assert_eq!(
        follower.chain_mut().tip().unwrap().height,
        w.first_height + 1
    );
}

#[test]
fn a_forged_header_is_rejected_by_the_relay_and_its_buffer_is_closed() {
    // Checkpoint at 4,465,096; the forged header replaces real block 4,465,100 (parent 4,465,099).
    let w = world(196);
    assert_eq!(w.checkpoint_height, 4_465_096);
    let forged: RawHeader = relay_tests::header(FORGED_LOW_DIFFICULTY)
        .try_into()
        .unwrap();
    let mut rows = Rows {
        rows: w.rows[..6].to_vec(),
        forged: Some((3, forged)),
    };
    rows.rows.truncate(6);
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let mut follower = Follower::new(client(&w, vec![]), rows, move |e| sink.borrow_mut().push(e));

    assert_eq!(follower.step().unwrap(), Step::Rejected);

    assert_eq!(follower.chain_mut().tip().unwrap().height, 4_465_099);
    assert!(!buffer_exists(&w));
    assert!(events.borrow().iter().any(|e| matches!(
        e,
        Event::Rejected {
            height: 4_465_100,
            code: 27,
            ..
        }
    )));
}
