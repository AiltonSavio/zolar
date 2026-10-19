//! Mainnet replay: 400 consecutive recorded Zcash mainnet headers (3,522,201 to 3,522,600, exported
//! from public P2P peers and cross-checked against Blockchair by `scripts/export-mainnet-headers.sh`).
//! Mainnet has no NU7 height yet, so the 17-block window and 75 s spacing apply throughout, and the
//! testnet minimum-difficulty rule never does. Expected values come from the chain itself.
use sha2::{Digest, Sha256};
use zcash_header::{
    difficulty::{check_time, expected_bits, Rules, CONTEXT_LEN},
    equihash::{self, Progress, STATE_LEN},
    header,
    pow::{check_pow, Network, PowError},
};

const FIXTURE: &str = include_str!("../fixtures/mainnet/mainnet-3522201-3522600.txt");
const MAINNET_POW_LIMIT_BITS: u32 = 0x1f07ffff;

const RULES: Rules = Rules {
    network: Network::Mainnet,
    nu7_height: u32::MAX,
};

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn rows() -> Vec<(u32, Vec<u8>)> {
    FIXTURE
        .lines()
        .map(|l| {
            let (h, raw) = l.split_once(' ').unwrap();
            (h.parse().unwrap(), hex::decode(raw).unwrap())
        })
        .collect()
}

fn fixed(raw: &[u8]) -> [u8; header::HEADER_LEN] {
    raw.try_into().unwrap()
}

fn solution_verifies(raw: &[u8]) -> bool {
    let mut state = vec![0u8; STATE_LEN];
    if equihash::begin(raw, &mut state).is_err() {
        return false;
    }
    loop {
        match equihash::step(raw, &mut state, 256) {
            Ok(Progress::Verified) => return true,
            Ok(Progress::Pending(_)) => {}
            Err(_) => return false,
        }
    }
}

/// Times and bits of the `CONTEXT_LEN` blocks before `rows[i]`, newest first.
fn context(rows: &[(u32, Vec<u8>)], i: usize) -> (Vec<u32>, Vec<u32>) {
    let before = &rows[i - CONTEXT_LEN..i];
    (
        before
            .iter()
            .rev()
            .map(|r| header::time(&fixed(&r.1)))
            .collect(),
        before
            .iter()
            .rev()
            .map(|r| header::bits(&fixed(&r.1)))
            .collect(),
    )
}

#[test]
fn every_mainnet_header_links_meets_its_target_and_has_a_valid_solution() {
    let rows = rows();
    assert_eq!(rows.len(), 400);
    for (i, (height, raw)) in rows.iter().enumerate() {
        let h = fixed(raw);
        assert_eq!(header::version(&h), 4, "height {height}");
        let hash = header::hash(sha256, &h);
        assert!(
            check_pow(Network::Mainnet, header::bits(&h), &hash).is_ok(),
            "height {height}"
        );
        assert!(solution_verifies(raw), "height {height}");
        if i > 0 {
            assert_eq!(
                header::prev_hash(&h),
                header::hash(sha256, &fixed(&rows[i - 1].1))
            );
        }
    }
}

#[test]
fn every_mainnet_header_has_exactly_the_target_the_rule_computes() {
    let rows = rows();
    let mut checked = 0;
    for i in CONTEXT_LEN..rows.len() {
        let (height, raw) = &rows[i];
        let h = fixed(raw);
        let (times, bits) = context(&rows, i);
        assert_eq!(
            expected_bits(&RULES, *height, header::time(&h), &times, &bits),
            Ok(header::bits(&h)),
            "height {height}"
        );
        assert_eq!(
            check_time(&RULES, *height, header::time(&h), &times),
            Ok(())
        );
        checked += 1;
    }
    assert_eq!(checked, 400 - CONTEXT_LEN);
}

#[test]
fn a_solution_with_a_flipped_bit_is_rejected() {
    let (_, raw) = &rows()[200];
    assert!(solution_verifies(raw));
    let mut bad = raw.clone();
    bad[600] ^= 1;
    assert!(!solution_verifies(&bad));
}

/// Real blocks carry about 2^215; a forger who claims the mainnet limit (2^243) mines 8,192 times
/// faster, and the exact-match rule is what turns that header away.
#[test]
fn a_target_above_the_real_difficulty_is_not_the_expected_target() {
    let rows = rows();
    let i = 200;
    let (height, raw) = &rows[i];
    let h = fixed(raw);
    let (times, bits) = context(&rows, i);
    let expected = expected_bits(&RULES, *height, header::time(&h), &times, &bits).unwrap();
    assert_ne!(expected, MAINNET_POW_LIMIT_BITS);
    assert!(expected < MAINNET_POW_LIMIT_BITS);
}

/// Testnet lets a block more than 450 s after its parent carry the limit target. Mainnet never does:
/// the same context and a late time still give the adjusted target.
#[test]
fn the_testnet_minimum_difficulty_rule_does_not_apply_on_mainnet() {
    let rows = rows();
    let i = 200;
    let (height, raw) = &rows[i];
    let h = fixed(raw);
    let (times, bits) = context(&rows, i);
    let late = times[0] + 451;
    let mainnet = expected_bits(&RULES, *height, late, &times, &bits).unwrap();
    assert_ne!(mainnet, MAINNET_POW_LIMIT_BITS);
    assert_eq!(check_time(&RULES, *height, late, &times), Ok(()));
    let testnet = Rules {
        network: Network::Testnet,
        ..RULES
    };
    assert_eq!(
        expected_bits(&testnet, *height, late, &times, &bits),
        Ok(0x2007ffff)
    );
    assert_eq!(
        check_pow(Network::Mainnet, 0x2007ffff, &header::hash(sha256, &h)),
        Err(PowError::AboveLimit)
    );
}
