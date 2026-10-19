use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use zolar_relayer::{
    follower::{
        Batch, ChainError, Event, Follower, Hash, HeaderSource, Outcome, RelayChain, Stage, Step,
        Tip,
    },
    p2p::{sha256d, RawHeader, HEADER_LEN},
};

/// A header that only carries what following needs: a parent link and a distinguishing nonce byte.
fn header(parent: Hash, salt: u8) -> RawHeader {
    let mut h = [0u8; HEADER_LEN];
    h[4..36].copy_from_slice(&parent);
    h[100] = salt;
    h
}

fn chain_from(parent: Hash, salts: &[u8]) -> Vec<RawHeader> {
    let mut parent = parent;
    salts
        .iter()
        .map(|s| {
            let h = header(parent, *s);
            parent = sha256d(&h);
            h
        })
        .collect()
}

const CHECKPOINT: Hash = [7; 32];

#[derive(Default)]
struct State {
    tip: Option<Tip>,
    held: Vec<(Hash, u32)>,
    submitted: Vec<Hash>,
    fail_next: VecDeque<String>,
    reject_at: Option<Hash>,
    halt_after: Option<Hash>,
    penalized: Vec<String>,
}

#[derive(Clone)]
struct FakeRelay(Rc<RefCell<State>>);

impl RelayChain for FakeRelay {
    fn tip(&mut self) -> Result<Tip, ChainError> {
        Ok(self.0.borrow().tip.unwrap())
    }
    fn known_blocks(&mut self) -> Result<Vec<(Hash, u32)>, ChainError> {
        Ok(self.0.borrow().held.clone())
    }
    fn submit(
        &mut self,
        header: &RawHeader,
        stage: &mut dyn FnMut(Stage),
    ) -> Result<Outcome, ChainError> {
        let mut s = self.0.borrow_mut();
        if let Some(e) = s.fail_next.pop_front() {
            return Err(ChainError::Transient(e));
        }
        let hash = sha256d(header);
        let parent: Hash = header[4..36].try_into().unwrap();
        stage(Stage::Uploaded);
        if s.reject_at == Some(hash) {
            return Ok(Outcome::Rejected {
                code: 27,
                reason: "BadDifficulty".into(),
            });
        }
        stage(Stage::Verified);
        if s.held.iter().any(|(h, _)| *h == hash) {
            return Ok(Outcome::AlreadyKnown);
        }
        let parent_height = s.held.iter().find(|(h, _)| *h == parent).unwrap().1;
        s.held.insert(0, (hash, parent_height + 1));
        s.submitted.push(hash);
        let tip = s.tip.unwrap();
        if parent_height + 1 > tip.height {
            s.tip = Some(Tip {
                height: parent_height + 1,
                hash,
                halted: u8::from(s.halt_after == Some(hash)),
            });
        }
        Ok(Outcome::Accepted {
            tip: s.tip.unwrap(),
            signatures: vec!["sig".into()],
        })
    }
}

struct FakeSource {
    /// What the peer knows: full chains from the checkpoint.
    chains: Vec<Vec<RawHeader>>,
    state: Rc<RefCell<State>>,
}

impl HeaderSource for FakeSource {
    fn headers_after(&mut self, locator: &[Hash]) -> Result<Batch, String> {
        for chain in &self.chains {
            let hashes: Vec<Hash> = chain.iter().map(|h| sha256d(h)).collect();
            for want in locator {
                if *want == CHECKPOINT {
                    return Ok(Batch {
                        peer: "p".into(),
                        headers: chain.clone(),
                    });
                }
                if let Some(i) = hashes.iter().position(|h| h == want) {
                    return Ok(Batch {
                        peer: "p".into(),
                        headers: chain[i + 1..].to_vec(),
                    });
                }
            }
        }
        Ok(Batch {
            peer: "p".into(),
            headers: vec![],
        })
    }
    fn penalize(&mut self, peer: &str) {
        self.state.borrow_mut().penalized.push(peer.into());
    }
}

fn setup(chains: Vec<Vec<RawHeader>>) -> (Rc<RefCell<State>>, FakeRelay, FakeSource) {
    let state = Rc::new(RefCell::new(State {
        tip: Some(Tip {
            height: 100,
            hash: CHECKPOINT,
            halted: 0,
        }),
        held: vec![(CHECKPOINT, 100)],
        ..State::default()
    }));
    (
        state.clone(),
        FakeRelay(state.clone()),
        FakeSource { chains, state },
    )
}

type Log = Rc<RefCell<Vec<Event>>>;

fn collect() -> (Log, Box<dyn FnMut(Event)>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    (log, Box::new(move |e| sink.borrow_mut().push(e)))
}

#[test]
fn it_submits_every_new_header_in_order_and_reports_the_tip() {
    let chain = chain_from(CHECKPOINT, &[1, 2, 3]);
    let (state, relay, source) = setup(vec![chain.clone()]);
    let (log, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);

    assert_eq!(follower.step().unwrap(), Step::Progress { accepted: 3 });
    assert_eq!(follower.step().unwrap(), Step::Idle);

    let hashes: Vec<Hash> = chain.iter().map(|h| sha256d(h)).collect();
    assert_eq!(state.borrow().submitted, hashes);
    let log = log.borrow();
    assert!(log.contains(&Event::Uploaded {
        height: 101,
        hash: hashes[0]
    }));
    assert!(log.contains(&Event::Verified {
        height: 101,
        hash: hashes[0]
    }));
    assert!(log.contains(&Event::Accepted {
        height: 103,
        hash: hashes[2],
        tip_height: 103,
        signatures: vec!["sig".into()]
    }));
    assert!(log.contains(&Event::Tip {
        height: 103,
        hash: hashes[2]
    }));
}

#[test]
fn a_transient_failure_loses_and_repeats_nothing() {
    let chain = chain_from(CHECKPOINT, &[1, 2, 3]);
    let (state, relay, source) = setup(vec![chain.clone()]);
    state.borrow_mut().fail_next.push_back("rpc timeout".into());
    let (_, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);

    assert!(matches!(follower.step(), Err(ChainError::Transient(_))));
    assert_eq!(follower.step().unwrap(), Step::Progress { accepted: 3 });

    let hashes: Vec<Hash> = chain.iter().map(|h| sha256d(h)).collect();
    assert_eq!(state.borrow().submitted, hashes);
}

#[test]
fn a_rejected_header_drops_the_rest_of_the_batch_and_penalizes_the_peer() {
    let chain = chain_from(CHECKPOINT, &[1, 2, 3]);
    let hashes: Vec<Hash> = chain.iter().map(|h| sha256d(h)).collect();
    let (state, relay, source) = setup(vec![chain]);
    state.borrow_mut().reject_at = Some(hashes[1]);
    let (log, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);

    assert_eq!(follower.step().unwrap(), Step::Rejected);

    assert_eq!(state.borrow().submitted, vec![hashes[0]]);
    assert_eq!(state.borrow().penalized, vec!["p".to_string()]);
    assert!(log.borrow().iter().any(|e| matches!(
        e,
        Event::Rejected {
            height: 102,
            code: 27,
            ..
        }
    )));
}

#[test]
fn a_peer_on_another_branch_is_penalized_and_nothing_is_submitted() {
    let stranger = chain_from([9; 32], &[1, 2]);
    let (state, relay, _) = setup(vec![]);
    // The peer answers from a chain that does not contain the relay's tip.
    struct Stranger(Vec<RawHeader>, Rc<RefCell<State>>);
    impl HeaderSource for Stranger {
        fn headers_after(&mut self, _: &[Hash]) -> Result<Batch, String> {
            Ok(Batch {
                peer: "stale".into(),
                headers: self.0.clone(),
            })
        }
        fn penalize(&mut self, peer: &str) {
            self.1.borrow_mut().penalized.push(peer.into());
        }
    }
    let (log, sink) = collect();
    let mut follower = Follower::new(relay, Stranger(stranger, state.clone()), sink);

    assert_eq!(follower.step().unwrap(), Step::Idle);

    assert!(state.borrow().submitted.is_empty());
    assert_eq!(state.borrow().penalized, vec!["stale".to_string()]);
    assert!(log.borrow().contains(&Event::Unlinked {
        peer: "stale".into()
    }));
}

#[test]
fn a_reorg_is_followed_from_the_fork_point() {
    let main = chain_from(CHECKPOINT, &[1, 2, 3]);
    let (state, relay, source) = setup(vec![main.clone()]);
    let (_, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);
    follower.step().unwrap();

    // The network now has a longer branch that forks after block 101.
    let fork_parent = sha256d(&main[0]);
    let branch = chain_from(fork_parent, &[20, 21, 22]);
    let mut full = vec![main[0]];
    full.extend(branch.clone());
    follower = Follower::new(
        FakeRelay(state.clone()),
        FakeSource {
            chains: vec![full],
            state: state.clone(),
        },
        Box::new(|_| {}),
    );

    assert_eq!(follower.step().unwrap(), Step::Progress { accepted: 3 });
    let branch_tip = sha256d(&branch[2]);
    assert_eq!(state.borrow().tip.unwrap().hash, branch_tip);
    assert_eq!(state.borrow().tip.unwrap().height, 104);
}

#[test]
fn a_halted_relay_gets_nothing() {
    let chain = chain_from(CHECKPOINT, &[1]);
    let (state, relay, source) = setup(vec![chain]);
    state.borrow_mut().tip.as_mut().unwrap().halted = 1;
    let (log, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);

    assert_eq!(follower.step().unwrap(), Step::Halted);

    assert!(state.borrow().submitted.is_empty());
    assert!(log.borrow().contains(&Event::Halted { reason: 1 }));
}

#[test]
fn a_halt_caused_by_a_header_stops_the_batch() {
    let chain = chain_from(CHECKPOINT, &[1, 2, 3]);
    let first = sha256d(&chain[0]);
    let (state, relay, source) = setup(vec![chain]);
    state.borrow_mut().halt_after = Some(first);
    let (_, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);

    assert_eq!(follower.step().unwrap(), Step::Halted);
    assert_eq!(state.borrow().submitted, vec![first]);
}

#[test]
fn headers_the_relay_already_holds_are_not_resubmitted() {
    let chain = chain_from(CHECKPOINT, &[1, 2, 3]);
    let (state, relay, source) = setup(vec![chain.clone()]);
    let (_, sink) = collect();
    let mut follower = Follower::new(relay, source, sink);
    follower.step().unwrap();
    let submitted = state.borrow().submitted.len();

    // A peer that lags behind answers with the whole chain again.
    struct Replay(Vec<RawHeader>);
    impl HeaderSource for Replay {
        fn headers_after(&mut self, _: &[Hash]) -> Result<Batch, String> {
            Ok(Batch {
                peer: "lagging".into(),
                headers: self.0.clone(),
            })
        }
        fn penalize(&mut self, _: &str) {}
    }
    let mut follower = Follower::new(FakeRelay(state.clone()), Replay(chain), |_| {});
    follower.step().unwrap();
    // Unlinked from the checkpoint's point of view is fine; what matters is no new submissions.
    assert_eq!(state.borrow().submitted.len(), submitted);
}

#[test]
fn the_locator_reaches_back_to_the_oldest_block_the_relay_holds() {
    let chain = chain_from(CHECKPOINT, &(1..=150).collect::<Vec<u8>>());
    let (state, relay, _) = setup(vec![]);
    // The relay holds 150 blocks already.
    {
        let mut s = state.borrow_mut();
        for (i, h) in chain.iter().enumerate() {
            let hash = sha256d(h);
            s.held.insert(0, (hash, 101 + i as u32));
        }
        let top = s.held[0];
        s.tip = Some(Tip {
            height: top.1,
            hash: top.0,
            halted: 0,
        });
    }
    struct Spy(Rc<RefCell<Vec<Vec<Hash>>>>);
    impl HeaderSource for Spy {
        fn headers_after(&mut self, locator: &[Hash]) -> Result<Batch, String> {
            self.0.borrow_mut().push(locator.to_vec());
            Ok(Batch {
                peer: "p".into(),
                headers: vec![],
            })
        }
        fn penalize(&mut self, _: &str) {}
    }
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut follower = Follower::new(relay, Spy(seen.clone()), |_| {});

    follower.step().unwrap();

    let locator = seen.borrow()[0].clone();
    assert!(locator.len() <= 64, "{} hashes", locator.len());
    assert_eq!(locator[0], sha256d(&chain[149]), "starts at the tip");
    assert_eq!(locator[1], sha256d(&chain[148]), "dense near the tip");
    assert!(locator.contains(&CHECKPOINT), "reaches the oldest block");
}
