//! Final Block Tree, Finality Depth and the read interface, driven through the generated client.

use header_relay_interface::{
    RelayView, HALT_DEEP_REORG, HALT_TREE_FULL, ROOT_HISTORY, R_LEAF_COUNT, TREE_CAPACITY,
};
use relay_tests::{synthetic::*, *};
use solana_address::Address;

const REFERENCE: &str = include_str!("../fixtures/final-block-tree-nu7-4465014-4465270.json");

fn row_hash(rows: &[(u32, Vec<u8>)], height: u32) -> [u8; 32] {
    sha256d(&rows.iter().find(|r| r.0 == height).unwrap().1)
}

fn hex32(s: &str) -> [u8; 32] {
    hex::decode(s).unwrap().try_into().unwrap()
}

/// The root after the 287 live headers equals the root circomlibjs computes independently from the
/// fixture, and so does every root still in the history.
#[test]
fn the_final_block_tree_after_replaying_live_headers_equals_the_reference_root() {
    let rows = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let reference: serde_json::Value = serde_json::from_str(REFERENCE).unwrap();
    let roots: Vec<[u8; 32]> = reference["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| hex32(r.as_str().unwrap()))
        .collect();
    let checkpoint = checkpoint_before(&rows, 113, NU7_HEIGHT, 30, 0);
    let mut h = Harness::new();
    let relay = h.init_relay(&checkpoint);

    let empty = h.relay_data(&relay);
    let view = RelayView::parse(&empty).unwrap();
    assert_eq!(view.latest_root(), roots[0], "empty tree root");
    assert_eq!(view.finality_depth(), 30);

    let mut worst = 0;
    let mut accept_alone = 0;
    let replay = &rows[113..];
    for (i, (_, raw)) in replay.iter().enumerate() {
        if i + 1 == replay.len() {
            // The last header: verify in one transaction, accept alone in the next.
            let mut cu = Cu::default();
            let buffer = h.upload_and_verify(&relay, 0, raw, &mut cu);
            h.ok(&[set_compute_limit(CU_CAP), h.step_ix(&buffer, 256)], &[]);
            accept_alone = h.ok(
                &[set_compute_limit(CU_CAP), h.accept_ix(&buffer, &relay)],
                &[],
            );
            h.ok(&[h.close_ix(&buffer)], &[]);
        } else {
            let cu = h.relay_header(&relay, (i % 2) as u8, raw);
            worst = worst.max(cu.last_leaves_and_accept);
        }
    }
    println!("CU worst transaction (last 256 leaves + AcceptHeader): {worst}");
    println!("CU AcceptHeader alone, NU7 window, one append: {accept_alone}");

    let data = h.relay_data(&relay);
    let view = RelayView::parse(&data).unwrap();
    assert_eq!(view.tip_height(), 4_465_300);
    assert_eq!(view.finalized_height(), 4_465_270);
    assert_eq!(view.finalized_hash(), row_hash(&rows, 4_465_270));
    assert_eq!(view.leaf_count(), 257);
    assert_eq!(view.latest_root(), roots[257]);
    for size in 257 - (ROOT_HISTORY as u32 - 1)..=257 {
        assert_eq!(
            view.root_at(size),
            Some(roots[size as usize]),
            "size {size}"
        );
    }
    assert_eq!(view.root_at(257 - ROOT_HISTORY as u32), None);
    assert!(view.is_known_root(&roots[200]));
    assert!(!view.is_known_root(&roots[100]), "aged out of the history");
    assert!(!view.is_halted());
}

// ---- chain selection, finality and Halt on synthetic branches ----

fn tip_hash(h: &Harness, relay: &Address) -> [u8; 32] {
    let data = h.relay_data(relay);
    RelayView::parse(&data).unwrap().tip_hash()
}

/// Hashes held by the Store's slots.
fn stored(h: &Harness, relay: &Address) -> std::collections::HashSet<[u8; 32]> {
    let (store, data) = h.store(relay);
    (0..store.capacity as usize)
        .filter(|i| {
            let at = 40 + i * 102;
            u32::from_le_bytes(data[at + 92..at + 96].try_into().unwrap()) != 0
        })
        .map(|i| data[40 + i * 102..40 + i * 102 + 32].try_into().unwrap())
        .collect()
}

fn send_all(h: &mut Harness, relay: &Address, blocks: &[Synth]) {
    for b in blocks {
        h.relay_synthetic(relay, &b.raw)
            .unwrap_or_else(|e| panic!("block {} rejected with {e}", b.height));
    }
}

const DUPLICATE_HEADER: u32 = 13;
const RELAY_HALTED: u32 = 16;
const CONTEXT_UNAVAILABLE: u32 = 30;

fn build_trunk(h: &mut Harness, depth: u16, blocks: usize) -> (Address, Branch, Vec<Synth>) {
    let checkpoint = synthetic_checkpoint(depth, 0);
    let relay = h.init_relay(&checkpoint);
    let mut branch = Branch::new(&checkpoint);
    let chain: Vec<Synth> = (0..blocks).map(|i| branch.next(i as u32)).collect();
    send_all(h, &relay, &chain);
    (relay, branch, chain)
}

#[test]
fn competing_branches_resolve_to_the_one_with_most_work() {
    let mut h = Harness::new();
    let (relay, trunk, _) = build_trunk(&mut h, 5, 3);
    let base = trunk.tip_height();

    let mut a = trunk.clone();
    let a_blocks = [a.next(100), a.next(101)];
    let mut b = trunk.clone();
    let b_blocks = [b.next(200), b.next(201), b.next(202)];

    send_all(&mut h, &relay, &a_blocks);
    assert_eq!(tip_hash(&h, &relay), a_blocks[1].hash);
    send_all(&mut h, &relay, &b_blocks[..1]);
    assert_eq!(tip_hash(&h, &relay), a_blocks[1].hash, "less work");
    send_all(&mut h, &relay, &b_blocks[1..2]);
    assert_eq!(
        tip_hash(&h, &relay),
        a_blocks[1].hash,
        "equal work keeps the first seen"
    );
    send_all(&mut h, &relay, &b_blocks[2..]);
    assert_eq!(tip_hash(&h, &relay), b_blocks[2].hash, "more work wins");
    let data = h.relay_data(&relay);
    assert_eq!(RelayView::parse(&data).unwrap().tip_height(), base + 3);

    // Work, not length: three minimum-difficulty blocks (tall, cheap) lose to one ordinary block.
    let mut h = Harness::new();
    let (relay, trunk, _) = build_trunk(&mut h, 5, 3);
    let mut ordinary = trunk.clone();
    let one = ordinary.next(1);
    let mut cheap = trunk.clone();
    let three = [
        cheap.extend(MIN_DIFFICULTY_GAP, 2),
        cheap.extend(MIN_DIFFICULTY_GAP, 3),
        cheap.extend(MIN_DIFFICULTY_GAP, 4),
    ];
    assert!(three.iter().all(|b| b.bits > one.bits), "easier targets");
    send_all(&mut h, &relay, std::slice::from_ref(&one));
    send_all(&mut h, &relay, &three);
    let data = h.relay_data(&relay);
    let view = RelayView::parse(&data).unwrap();
    assert_eq!(view.tip_hash(), one.hash);
    assert_eq!(
        view.tip_height(),
        base_height(3) + 1,
        "the taller branch is not the tip"
    );
}

fn base_height(blocks: u32) -> u32 {
    CHECKPOINT_HEIGHT + blocks
}

#[test]
fn a_reorg_deeper_than_finality_depth_halts_the_relay() {
    let mut h = Harness::new();
    let (relay, trunk, chain) = build_trunk(&mut h, 2, 8);
    let data = h.relay_data(&relay);
    let before = RelayView::parse(&data).unwrap();
    assert_eq!(
        (before.finalized_height(), before.leaf_count()),
        (CHECKPOINT_HEIGHT + 6, 6)
    );
    let root_before = before.latest_root();

    // A branch from block 3 with one block more than the trunk has above it: 6 against 5.
    let mut fork = trunk.fork_at(CHECKPOINT_HEIGHT + 3);
    let blocks: Vec<Synth> = (0..6).map(|i| fork.next(1000 + i)).collect();
    send_all(&mut h, &relay, &blocks[..5]);
    assert_eq!(tip_hash(&h, &relay), chain[7].hash, "five blocks only tie");
    h.relay_synthetic(&relay, &blocks[5].raw)
        .expect("the Halting header itself succeeds");

    let data = h.relay_data(&relay);
    let view = RelayView::parse(&data).unwrap();
    assert!(view.is_halted());
    assert_eq!(view.halt_reason(), HALT_DEEP_REORG);
    assert_eq!(view.halt_hash(), blocks[5].hash);
    assert_eq!(view.halt_height(), CHECKPOINT_HEIGHT + 8);
    assert_eq!(view.tip_hash(), chain[7].hash, "the tip does not move");
    assert_eq!(view.leaf_count(), 6);
    assert_eq!(view.latest_root(), root_before, "the tree is untouched");

    let mut more = trunk.clone();
    assert_eq!(
        h.relay_synthetic(&relay, &more.next(5000).raw),
        Err(RELAY_HALTED)
    );
}

#[test]
fn a_reorg_within_finality_depth_switches_chains_and_keeps_final_blocks() {
    let mut h = Harness::new();
    let (relay, trunk, _) = build_trunk(&mut h, 3, 8);
    let data = h.relay_data(&relay);
    assert_eq!(
        RelayView::parse(&data).unwrap().finalized_height(),
        CHECKPOINT_HEIGHT + 5
    );

    // Fork at block 5 (the last final block) with four blocks against three.
    let mut fork = trunk.fork_at(CHECKPOINT_HEIGHT + 5);
    let blocks: Vec<Synth> = (0..4).map(|i| fork.next(2000 + i)).collect();
    send_all(&mut h, &relay, &blocks);

    let data = h.relay_data(&relay);
    let view = RelayView::parse(&data).unwrap();
    assert!(!view.is_halted());
    assert_eq!(view.tip_hash(), blocks[3].hash);
    assert_eq!(view.finalized_height(), CHECKPOINT_HEIGHT + 6);
    assert_eq!(
        view.finalized_hash(),
        blocks[0].hash,
        "block 6 is now the fork's"
    );
    assert_eq!(view.leaf_count(), 6);
}

#[test]
fn a_flood_of_cheap_siblings_evicts_neither_the_tip_chain_nor_a_harder_branch() {
    let mut h = Harness::new();
    let (relay, trunk, chain) = build_trunk(&mut h, 5, 190);

    // An honest competing branch of ordinary blocks, forking 20 blocks below the tip (190).
    let mut honest = trunk.fork_at(CHECKPOINT_HEIGHT + 170);
    let honest_blocks: Vec<Synth> = (0..3).map(|i| honest.next(9000 + i)).collect();
    send_all(&mut h, &relay, &honest_blocks);

    // 260 minimum-difficulty siblings of blocks near the tip: more than the 207 free slots.
    for i in 0..260u32 {
        let parent_height = CHECKPOINT_HEIGHT + 180 + i % 8;
        let mut fork = trunk.fork_at(parent_height);
        let junk = fork.extend(MIN_DIFFICULTY_GAP, 50_000 + i);
        match h.relay_synthetic(&relay, &junk.raw) {
            Ok(_) => {}
            Err(code) => assert_eq!(
                code, 31,
                "only the admission rule may refuse junk, got {code}"
            ),
        }
    }

    let held = stored(&h, &relay);
    for block in &chain[chain.len() - 179..] {
        assert!(
            held.contains(&block.hash),
            "tip chain block {} evicted",
            block.height
        );
    }
    for block in &honest_blocks {
        assert!(
            held.contains(&block.hash),
            "harder branch block {} evicted",
            block.height
        );
    }
    // Both the tip chain and the honest branch can still be extended.
    let mut tip = trunk.clone();
    h.relay_synthetic(&relay, &tip.next(70_000).raw)
        .expect("tip chain extends");
    h.relay_synthetic(&relay, &honest.next(70_001).raw)
        .expect("honest branch extends");
}

#[test]
fn resubmitting_an_evicted_ancestor_and_its_child_repairs_the_branch() {
    let mut h = Harness::new();
    let (relay, trunk, chain) = build_trunk(&mut h, 3, 8);

    let mut branch = trunk.fork_at(CHECKPOINT_HEIGHT + 1);
    let (first, second) = (branch.next(801), branch.next(802));
    send_all(&mut h, &relay, std::slice::from_ref(&first));
    assert_eq!(
        h.relay_synthetic(&relay, &chain[2].raw),
        Err(DUPLICATE_HEADER)
    );

    // Fill the Store with ordinary siblings of the tip until `first` is the oldest unprotected block.
    let mut junk_branch = trunk.fork_at(CHECKPOINT_HEIGHT + 7);
    let free = 400 - 8 - 1;
    for i in 0..free - 1 {
        let junk = junk_branch.clone().next(10_000 + i as u32);
        h.relay_synthetic(&relay, &junk.raw).unwrap();
    }
    send_all(&mut h, &relay, std::slice::from_ref(&second));
    let junk = junk_branch.next(99_999);
    h.relay_synthetic(&relay, &junk.raw).unwrap();

    let held = stored(&h, &relay);
    assert!(
        !held.contains(&first.hash),
        "the oldest unprotected block went"
    );
    assert!(held.contains(&second.hash));

    let third = branch.next(803);
    assert_eq!(
        h.relay_synthetic(&relay, &third.raw),
        Err(CONTEXT_UNAVAILABLE),
        "the child's pointer leads to a dead slot"
    );
    h.relay_synthetic(&relay, &first.raw)
        .expect("the ancestor is accepted again");
    assert_eq!(
        h.relay_synthetic(&relay, &third.raw),
        Err(CONTEXT_UNAVAILABLE),
        "the second block still points at the old slot"
    );
    h.relay_synthetic(&relay, &second.raw)
        .expect("resubmitting the child repairs its pointer");
    assert_eq!(
        h.relay_synthetic(&relay, &second.raw),
        Err(DUPLICATE_HEADER),
        "now it is a plain duplicate"
    );
    h.relay_synthetic(&relay, &third.raw)
        .expect("the branch extends again");
}

#[test]
fn a_full_final_block_tree_halts_the_relay_instead_of_bricking_it() {
    let mut h = Harness::new();
    let checkpoint = synthetic_checkpoint(1, 0);
    let relay = h.init_relay(&checkpoint);
    let mut account = h.svm.get_account(&relay).unwrap();
    account.data[R_LEAF_COUNT..R_LEAF_COUNT + 4]
        .copy_from_slice(&(TREE_CAPACITY as u32).to_le_bytes());
    h.svm.set_account(relay, account).unwrap();
    let mut branch = Branch::new(&checkpoint);
    send_all(&mut h, &relay, &[branch.next(1)]);
    h.relay_synthetic(&relay, &branch.next(2).raw)
        .expect("the Halting header succeeds");
    let data = h.relay_data(&relay);
    let view = RelayView::parse(&data).unwrap();
    assert_eq!(view.halt_reason(), HALT_TREE_FULL);
    assert_eq!(view.tip_height(), CHECKPOINT_HEIGHT + 1);
    assert_eq!(
        h.relay_synthetic(&relay, &branch.next(3).raw),
        Err(RELAY_HALTED)
    );
}

#[test]
fn compute_units_of_accept_header_alone_on_a_synthetic_chain() {
    let mut h = Harness::new();
    let checkpoint = synthetic_checkpoint(5, 0);
    let relay = h.init_relay(&checkpoint);
    let mut branch = Branch::new(&checkpoint);
    let mut no_append = 0;
    let mut append = 0;
    for i in 0..8 {
        let cu = h.relay_synthetic(&relay, &branch.next(i).raw).unwrap();
        if i == 3 {
            no_append = cu;
        }
        if i == 7 {
            append = cu;
        }
    }
    let mut fork_branch = branch.fork_at(CHECKPOINT_HEIGHT + 3);
    let non_tip = h
        .relay_synthetic(&relay, &fork_branch.next(77).raw)
        .unwrap();
    println!("CU AcceptHeader alone (17-block window): new tip no append {no_append}, new tip with append {append}, non-tip fork {non_tip}");
}
