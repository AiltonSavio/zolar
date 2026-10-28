//! The Header Relay enforces Zcash's difficulty adjustment and median-time rules over live NU7
//! testnet headers (see `scripts/export-testnet-p2p.py`).
use relay_tests::*;

const BAD_DIFFICULTY: u32 = 27;
const TIME_NOT_AFTER_MEDIAN: u32 = 28;
const TIME_TOO_FAR_AHEAD: u32 = 29;

fn index_of(rows: &[(u32, Vec<u8>)], height: u32) -> usize {
    rows.iter().position(|r| r.0 == height).unwrap()
}

#[test]
fn the_relay_accepts_287_live_headers_across_the_nu7_activation() {
    let rows = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let mut h = Harness::new();
    let checkpoint = checkpoint_before(&rows, 113, NU7_HEIGHT, 10, 0);
    assert_eq!(checkpoint.height, 4_465_013);
    let relay = h.init_relay(&checkpoint);
    let mut worst = 0;
    for (n, (height, raw)) in rows[113..].iter().enumerate() {
        let cu = h.relay_header(&relay, (n % 2) as u8, raw);
        worst = worst.max(cu.last_leaves_and_accept);
        assert_eq!(h.relay(&relay).tip_height, *height);
    }
    println!("worst accept transaction: {worst} CU");
    assert_eq!(h.relay(&relay).tip_height, 4_465_300);
    assert_eq!(h.store(&relay).0.count, 287);
}

#[test]
fn the_relay_accepts_a_real_minimum_difficulty_block_and_the_blocks_after_it() {
    let rows = fixture_rows(MIN_DIFFICULTY_FIXTURE);
    let at = index_of(&rows, 4_495_487);
    let mut h = Harness::new();
    let relay = h.init_relay(&checkpoint_before(&rows, at - 112, NU7_HEIGHT, 10, 0));
    for (n, (_, raw)) in rows[at - 112..at + 15].iter().enumerate() {
        h.relay_header(&relay, (n % 2) as u8, raw);
    }
    assert_eq!(h.relay(&relay).tip_height, 4_495_501);
}

/// Relay for the forged-header tests: the real chain up to 4,465,099, the parent of the forged blocks.
fn relay_before_4465100(h: &mut Harness) -> solana_address::Address {
    let rows = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let end = index_of(&rows, 4_465_100);
    h.init_relay(&checkpoint_before(&rows, end, NU7_HEIGHT, 10, 0))
}

fn rejected_with(forged: &str) -> u32 {
    let mut h = Harness::new();
    let relay = relay_before_4465100(&mut h);
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay, 0, &header(forged), &mut cu);
    let last = h.finish_ixs(&buffer, &relay);
    let code = h.custom_err(&last, &[]);
    assert_eq!(h.store(&relay).0.count, 0);
    code
}

#[test]
fn a_cheap_low_difficulty_header_is_rejected() {
    // Valid Equihash and work at the testnet limit, but 4,465,100 must carry the adjusted target.
    assert_eq!(rejected_with(FORGED_LOW_DIFFICULTY), BAD_DIFFICULTY);
}

#[test]
fn minimum_difficulty_needs_a_gap_over_450_seconds() {
    assert_eq!(rejected_with(FORGED_MIN_DIFFICULTY_450), BAD_DIFFICULTY);

    let mut h = Harness::new();
    let relay = relay_before_4465100(&mut h);
    h.relay_header(&relay, 0, &header(FORGED_MIN_DIFFICULTY_451));
    assert_eq!(h.relay(&relay).tip_height, 4_465_100);
}

#[test]
fn a_time_not_after_the_median_of_the_previous_eleven_is_rejected() {
    assert_eq!(rejected_with(FORGED_TIME_AT_MEDIAN), TIME_NOT_AFTER_MEDIAN);
}

#[test]
fn a_time_more_than_ninety_minutes_past_the_median_is_rejected() {
    // Its 5,263 s gap makes the minimum-difficulty target the right one, so only the time rule fails.
    assert_eq!(rejected_with(FORGED_TIME_TOO_FAR), TIME_TOO_FAR_AHEAD);
}

#[test]
fn a_checkpoint_with_an_invalid_target_in_its_context_cannot_be_sealed() {
    let rows = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let mut checkpoint = checkpoint_before(&rows, 200, NU7_HEIGHT, 10, 0);
    checkpoint.bits[60] = 0x0480_0001; // negative compact target
    let mut h = Harness::new();
    let create = h.create_relay_ix(&checkpoint);
    h.ok(&[create], &[]);
    for ix in h.write_checkpoint_ixs(&checkpoint) {
        h.ok(&[ix], &[]);
    }
    let seal = h.seal_ix(&checkpoint);
    assert_eq!(h.custom_err(&[seal], &[]), 18);
}

/// Compute units of `AcceptHeader` alone, before and after the activation (17 and 102 blocks averaged).
#[test]
fn accept_header_stays_well_inside_the_compute_budget() {
    let rows = fixture_rows(NU7_ACTIVATION_FIXTURE);
    let end = index_of(&rows, 4_465_025);
    let mut h = Harness::new();
    let relay = h.init_relay(&checkpoint_before(&rows, end, NU7_HEIGHT, 10, 0));
    for (height, raw) in &rows[end..end + 2] {
        let mut cu = Cu::default();
        let buffer = h.upload_and_verify(&relay, 0, raw, &mut cu);
        h.ok(&[set_compute_limit(CU_CAP), h.step_ix(&buffer, 256)], &[]);
        let accept = h.ok(
            &[set_compute_limit(CU_CAP), h.accept_ix(&buffer, &relay)],
            &[],
        );
        h.ok(&[h.close_ix(&buffer)], &[]);
        println!("AcceptHeader at {height}: {accept} CU");
        assert!(accept < 200_000);
    }
}
