//! Difficulty and median-time rules replayed over 400 consecutive headers of the live NU7 testnet
//! chain (4,464,901 to 4,465,300, exported from P2P peers by `scripts/export-testnet-p2p.py`),
//! which span the NU7 activation at 4,465,026. Expected values come from the chain itself: every
//! header's own `nBits` is the answer.
use zcash_header::{
    difficulty::{check_time, expected_bits, Rules, TimeError, CONTEXT_LEN},
    header,
    pow::Network,
};

const NU7_FIXTURE: &str = include_str!("../fixtures/testnet-nu7-4464901-4465300.txt");
const MIN_DIFFICULTY_FIXTURE: &str = include_str!("../fixtures/testnet-nu7-4495261-4495660.txt");
const NU7_HEIGHT: u32 = 4_465_026;

const RULES: Rules = Rules {
    network: Network::Testnet,
    nu7_height: NU7_HEIGHT,
};

struct Block {
    height: u32,
    time: u32,
    bits: u32,
}

fn blocks() -> Vec<Block> {
    parse(NU7_FIXTURE)
}

fn parse(fixture: &str) -> Vec<Block> {
    fixture
        .lines()
        .map(|line| {
            let (height, hex_str) = line.split_once(' ').unwrap();
            let raw: [u8; header::HEADER_LEN] = hex::decode(hex_str).unwrap().try_into().unwrap();
            Block {
                height: height.parse().unwrap(),
                time: header::time(&raw),
                bits: header::bits(&raw),
            }
        })
        .collect()
}

/// Times and bits of the `CONTEXT_LEN` blocks before `blocks[i]`, newest first.
fn context(blocks: &[Block], i: usize) -> (Vec<u32>, Vec<u32>) {
    let before = &blocks[i - CONTEXT_LEN..i];
    (
        before.iter().rev().map(|b| b.time).collect(),
        before.iter().rev().map(|b| b.bits).collect(),
    )
}

#[test]
fn every_fixture_header_has_exactly_the_target_the_rule_computes() {
    let blocks = blocks();
    assert_eq!(blocks.len(), 400);
    let mut checked = 0;
    for i in CONTEXT_LEN..blocks.len() {
        let b = &blocks[i];
        let (times, bits) = context(&blocks, i);
        let got = expected_bits(&RULES, b.height, b.time, &times, &bits).unwrap();
        assert_eq!(got, b.bits, "height {}", b.height);
        checked += 1;
    }
    assert_eq!(checked, 287);
}

/// Block 4,495,487 is the one minimum-difficulty block in 40,000 NU7 testnet blocks (it came 451 s
/// after its parent). The fixture keeps it plus the 170 blocks that follow, whose averaging window
/// still contains its 2^251 target.
#[test]
fn a_real_minimum_difficulty_block_and_the_blocks_that_average_it_in() {
    let blocks = parse(MIN_DIFFICULTY_FIXTURE);
    let at = blocks.iter().position(|b| b.height == 4_495_487).unwrap();
    assert_eq!(blocks[at].bits, 0x2007ffff);
    assert_eq!(blocks[at].time - blocks[at - 1].time, 451);
    for i in CONTEXT_LEN..blocks.len() {
        let b = &blocks[i];
        let (times, bits) = context(&blocks, i);
        assert_eq!(
            expected_bits(&RULES, b.height, b.time, &times, &bits),
            Ok(b.bits),
            "height {}",
            b.height
        );
        assert_eq!(check_time(&RULES, b.height, b.time, &times), Ok(()));
    }
    // One second less and the same block would have to carry the adjusted target.
    let (times, bits) = context(&blocks, at);
    let b = &blocks[at];
    assert_ne!(
        expected_bits(&RULES, b.height, b.time - 1, &times, &bits),
        Ok(0x2007ffff)
    );
}

#[test]
fn the_window_follows_the_candidate_height_not_the_context() {
    // 4,465,025 is the last block under the 17-block window and 75 s spacing, 4,465,026 the first
    // under the 102-block window and 25 s: their bits differ in character (0x1e7e92ff, 0x1f0162f6),
    // so applying the wrong window to either fails.
    let blocks = blocks();
    let at = |h: u32| blocks.iter().position(|b| b.height == h).unwrap();
    let before_activation = Rules {
        nu7_height: NU7_HEIGHT + 1,
        ..RULES
    };
    let i = at(NU7_HEIGHT);
    let (times, bits) = context(&blocks, i);
    let b = &blocks[i];
    assert_eq!(
        expected_bits(&RULES, b.height, b.time, &times, &bits),
        Ok(b.bits)
    );
    assert_ne!(
        expected_bits(&before_activation, b.height, b.time, &times, &bits),
        Ok(b.bits),
        "the 17-block window gives a different target at the first NU7 height"
    );
}

#[test]
fn every_fixture_header_satisfies_the_time_rules() {
    let blocks = blocks();
    for i in CONTEXT_LEN..blocks.len() {
        let (times, _) = context(&blocks, i);
        assert_eq!(
            check_time(&RULES, blocks[i].height, blocks[i].time, &times),
            Ok(())
        );
    }
}

#[test]
fn time_must_exceed_the_median_of_the_previous_eleven_and_stay_within_ninety_minutes() {
    let blocks = blocks();
    let i = 200;
    let (times, _) = context(&blocks, i);
    let mut last_eleven: Vec<u32> = times[..11].to_vec();
    last_eleven.sort();
    let mtp = last_eleven[5];
    let h = blocks[i].height;
    assert_eq!(
        check_time(&RULES, h, mtp, &times),
        Err(TimeError::NotAfterMedian)
    );
    assert_eq!(
        check_time(&RULES, h, mtp - 1, &times),
        Err(TimeError::NotAfterMedian)
    );
    assert_eq!(check_time(&RULES, h, mtp + 1, &times), Ok(()));
    assert_eq!(check_time(&RULES, h, mtp + 5400, &times), Ok(()));
    assert_eq!(
        check_time(&RULES, h, mtp + 5401, &times),
        Err(TimeError::TooFarAhead)
    );
}

/// `scripts/gen-difficulty-vectors.py` runs an independent big-integer model over synthetic
/// contexts: minimum-difficulty blocks (450 s does not qualify, 451 s does), a minimum-difficulty
/// block inside the mean, both clamps, mainnet, and a mean whose sum overflows 256 bits.
#[test]
fn matches_the_independent_model_on_synthetic_contexts() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/difficulty-vectors.json")).unwrap();
    let cases = vectors.as_array().unwrap();
    assert!(cases.len() >= 20);
    for c in cases {
        let u32s = |key: &str| -> Vec<u32> {
            c[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect()
        };
        let rules = Rules {
            network: match c["network"].as_str().unwrap() {
                "mainnet" => Network::Mainnet,
                _ => Network::Testnet,
            },
            nu7_height: c["nu7_height"].as_u64().unwrap() as u32,
        };
        let got = expected_bits(
            &rules,
            c["height"].as_u64().unwrap() as u32,
            c["time"].as_u64().unwrap() as u32,
            &u32s("times"),
            &u32s("bits"),
        );
        assert_eq!(
            got,
            Ok(c["expected_bits"].as_u64().unwrap() as u32),
            "{}",
            c["name"]
        );
    }
}

#[test]
fn a_context_shorter_than_the_window_is_an_error_not_the_pow_limit() {
    let blocks = blocks();
    let (times, bits) = context(&blocks, 200);
    assert_eq!(
        expected_bits(&RULES, 4_465_100, 0, &times[..112], &bits),
        Err(zcash_header::difficulty::DifficultyError::ContextTooShort)
    );
}
