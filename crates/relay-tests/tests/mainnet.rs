//! A second Header Relay with a mainnet Checkpoint replays 287 recorded mainnet headers at full
//! difficulty (3,522,314 to 3,522,600, after a 113-block context), rejects forged ones, and runs
//! next to a testnet Relay in the same program.
use relay_tests::{mainnet::*, *};
use solana_signer::Signer;

const BAD_DIFFICULTY: u32 = 27;

/// Relay on mainnet block `rows[end - 1]`.
fn mainnet_relay(h: &mut Harness, rows: &[(u32, Vec<u8>)], end: usize) -> solana_address::Address {
    h.init_relay(&mainnet_checkpoint(rows, end, 10, 0))
}

#[test]
fn the_relay_accepts_287_recorded_mainnet_headers() {
    let rows = fixture_rows(MAINNET_FIXTURE);
    let mut h = Harness::new();
    let checkpoint = mainnet_checkpoint(&rows, 113, 10, 0);
    assert_eq!((checkpoint.network, checkpoint.nu7_height), (0, NEVER));
    assert_eq!(checkpoint.height, 3_522_313);
    let relay = h.init_relay(&checkpoint);
    let (mut worst_accept, mut worst_tx) = (0, 0);
    for (n, (height, raw)) in rows[113..].iter().enumerate() {
        let cu = h.relay_header(&relay, (n % 2) as u8, raw);
        worst_accept = worst_accept.max(cu.last_leaves_and_accept);
        worst_tx = worst_tx.max(cu.upload_rest_and_first_leaves);
        assert_eq!(h.relay(&relay).tip_height, *height);
    }
    println!("mainnet worst finishing transaction (last leaves + AcceptHeader): {worst_accept} CU");
    println!("mainnet worst first-leaves transaction: {worst_tx} CU");
    assert_eq!(h.relay(&relay).tip_height, 3_522_600);
    assert_eq!(h.store(&relay).0.count, 287);
}

#[test]
fn a_mainnet_relay_logs_the_accept_header_compute_units() {
    let rows = fixture_rows(MAINNET_FIXTURE);
    let mut h = Harness::new();
    let relay = mainnet_relay(&mut h, &rows, 113);
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay, 0, &rows[113].1, &mut cu);
    let accept = h.ok(&[set_compute_limit(CU_CAP), h.step_ix(&buffer, 256)], &[]);
    let accept_header = h.ok(
        &[set_compute_limit(CU_CAP), h.accept_ix(&buffer, &relay)],
        &[],
    );
    println!("mainnet step: {accept} CU, AcceptHeader alone: {accept_header} CU");
    assert!(accept_header < 100_000);
}

#[test]
fn a_mainnet_header_with_a_bad_equihash_solution_is_rejected() {
    let rows = fixture_rows(MAINNET_FIXTURE);
    let at = rows.iter().position(|r| r.0 == 3_522_500).unwrap();
    let mut h = Harness::new();
    let relay = mainnet_relay(&mut h, &rows, at);
    let mut bad = rows[at].1.clone();
    bad[600] ^= 1;
    let mut cu = Cu::default();
    let code = h
        .try_upload_and_verify(&relay, 0, &bad, &mut cu)
        .expect_err("a corrupted solution must not verify");
    assert!((4..=7).contains(&code), "verifier error, got {code}");
    let buffer = Harness::buffer_addr(&relay, &h.payer.pubkey(), 0);
    assert_eq!(h.custom_err(&[h.accept_ix(&buffer, &relay)], &[]), 14);
    assert_eq!(h.store(&relay).0.count, 0);
}

#[test]
fn a_mainnet_header_with_a_target_above_the_real_difficulty_is_rejected() {
    // Real Equihash and real work at the real difficulty, but the chain this Relay follows was 256
    // times harder: only the exact difficulty rule can refuse it.
    let rows = fixture_rows(MAINNET_FIXTURE);
    let at = rows.iter().position(|r| r.0 == 3_522_500).unwrap();
    let mut h = Harness::new();
    let relay = h.init_relay(&harder_mainnet_checkpoint(&rows, at));
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay, 0, &rows[at].1, &mut cu);
    assert_eq!(
        h.custom_err(&h.finish_ixs(&buffer, &relay), &[]),
        BAD_DIFFICULTY
    );
    assert_eq!(h.store(&relay).0.count, 0);
}

#[test]
fn the_real_header_at_that_height_is_accepted() {
    let rows = fixture_rows(MAINNET_FIXTURE);
    let at = rows.iter().position(|r| r.0 == 3_522_500).unwrap();
    let mut h = Harness::new();
    let relay = mainnet_relay(&mut h, &rows, at);
    h.relay_header(&relay, 0, &rows[at].1);
    assert_eq!(h.relay(&relay).tip_height, 3_522_500);
}

#[test]
fn a_testnet_and_a_mainnet_relay_run_side_by_side_in_one_program() {
    let testnet_rows = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let mainnet_rows = fixture_rows(MAINNET_FIXTURE);
    let mut h = Harness::new();
    let testnet = h.init_relay(&checkpoint_before(&testnet_rows, 113, NU7_HEIGHT, 10, 0));
    let mainnet = h.init_relay(&mainnet_checkpoint(&mainnet_rows, 113, 10, 0));
    assert_ne!(testnet, mainnet);
    for n in 0..60 {
        h.relay_header(&testnet, (n % 2) as u8, &testnet_rows[113 + n].1);
        h.relay_header(&mainnet, (n % 2) as u8, &mainnet_rows[113 + n].1);
    }
    assert_eq!(h.relay(&testnet).checkpoint.network, 1);
    assert_eq!(h.relay(&mainnet).checkpoint.network, 0);
    assert_eq!(h.relay(&testnet).tip_height, testnet_rows[172].0);
    assert_eq!(h.relay(&mainnet).tip_height, mainnet_rows[172].0);
    assert_eq!(h.store(&testnet).0.count, 60);
    assert_eq!(h.store(&mainnet).0.count, 60);

    // Neither relay takes the other network's next header: the parent is unknown to it, and a
    // testnet header also fails the mainnet limit.
    let next_testnet = &testnet_rows[173].1;
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&mainnet, 0, next_testnet, &mut cu);
    let code = h.custom_err(&h.finish_ixs(&buffer, &mainnet), &[]);
    assert!(code == 10 || code == 12, "got {code}");
    let next_mainnet = &mainnet_rows[173].1;
    let buffer = h.upload_and_verify(&testnet, 1, next_mainnet, &mut cu);
    assert_eq!(h.custom_err(&h.finish_ixs(&buffer, &testnet), &[]), 12);
}
