use zcash_header::equihash::{self, Error, Progress, STATE_LEN};

const HEADER_HEX: &str = include_str!("../fixtures/testnet-4483600.hex");

fn header() -> Vec<u8> {
    hex::decode(HEADER_HEX.trim()).unwrap()
}

fn verify(header: &[u8], leaves_per_step: u16) -> Result<usize, Error> {
    let mut state = vec![0u8; STATE_LEN];
    equihash::begin(header, &mut state)?;
    let mut steps = 0;
    loop {
        steps += 1;
        if let Progress::Verified = equihash::step(header, &mut state, leaves_per_step)? {
            return Ok(steps);
        }
    }
}

#[test]
fn real_testnet_header_verifies() {
    assert_eq!(verify(&header(), 512).unwrap(), 1);
}

fn oracle_accepts(header: &[u8]) -> bool {
    ::equihash::is_valid_solution(200, 9, &header[..108], &header[108..140], &header[143..]).is_ok()
}

#[test]
fn oracle_accepts_the_fixture() {
    assert!(oracle_accepts(&header()));
}

#[test]
fn verdict_does_not_depend_on_step_size() {
    for per_step in [1, 7, 171, 256, 511, 512, 1000] {
        let steps = verify(&header(), per_step).unwrap();
        assert_eq!(steps, 512usize.div_ceil(per_step as usize));
    }
}

#[test]
fn any_flipped_solution_bit_is_rejected_like_the_stock_crate() {
    let good = header();
    let mut rejected = 0;
    for bit in (0..1344 * 8).step_by(61) {
        let mut bad = good.clone();
        bad[143 + bit / 8] ^= 1 << (bit % 8);
        assert!(!oracle_accepts(&bad));
        assert!(verify(&bad, 512).is_err(), "bit {bit} accepted");
        rejected += 1;
    }
    assert!(rejected > 150);
}

#[test]
fn flipped_nonce_or_header_bit_is_rejected() {
    let good = header();
    for byte in [0usize, 4, 36, 100, 108, 120, 139] {
        let mut bad = good.clone();
        bad[byte] ^= 0x01;
        assert!(verify(&bad, 512).is_err(), "byte {byte} accepted");
    }
}

#[test]
fn swapped_halves_are_out_of_order() {
    let mut bad = header();
    let (left, right) = bad[143..].split_at_mut(672);
    left.swap_with_slice(right);
    assert_eq!(verify(&bad, 512).unwrap_err(), Error::OutOfOrder);
}

#[test]
fn duplicated_index_is_rejected() {
    let mut bad = header();
    let first_four: Vec<u8> = bad[143..143 + 8].to_vec();
    bad[143 + 8..143 + 16].copy_from_slice(&first_four);
    assert!(verify(&bad, 512).is_err());
}

#[test]
fn malformed_header_and_state_are_rejected() {
    let mut state = vec![0u8; STATE_LEN];
    assert_eq!(
        equihash::begin(&header()[..1486], &mut state),
        Err(Error::MalformedHeader)
    );
    let mut bad_prefix = header();
    bad_prefix[140] = 0xfc;
    assert_eq!(
        equihash::begin(&bad_prefix, &mut state),
        Err(Error::MalformedHeader)
    );
    assert_eq!(
        equihash::begin(&header(), &mut state[..10]),
        Err(Error::MalformedState)
    );
    assert_eq!(
        equihash::step(&header(), &mut state, 1),
        Err(Error::WrongStage)
    );
}

#[test]
fn step_after_verified_is_rejected() {
    let h = header();
    let mut state = vec![0u8; STATE_LEN];
    equihash::begin(&h, &mut state).unwrap();
    assert_eq!(equihash::step(&h, &mut state, 512), Ok(Progress::Verified));
    assert!(equihash::is_verified(&state));
    assert_eq!(equihash::step(&h, &mut state, 1), Err(Error::WrongStage));
}

#[test]
fn leaf_hash_matches_blake2b_simd() {
    use zcash_header::blake2b;
    let h = header();
    let input: [u8; 140] = h[..140].try_into().unwrap();
    let mid = blake2b::midstate(&input, 200, 9);
    let mut personal = b"ZcashPoW".to_vec();
    personal.extend(200u32.to_le_bytes());
    personal.extend(9u32.to_le_bytes());
    for index in [0u32, 1, 2, 777, 1_048_575, 2_097_151] {
        let mut st = blake2b_simd::Params::new()
            .hash_length(50)
            .personal(&personal)
            .to_state();
        st.update(&input);
        st.update(&(index / 2).to_le_bytes());
        let full = st.finalize();
        let start = (index as usize % 2) * 25;
        assert_eq!(
            blake2b::leaf_hash(&mid, &input, index)[..],
            full.as_bytes()[start..start + 25]
        );
    }
}

#[test]
fn fixture_is_the_block_the_source_reported() {
    use sha2::{Digest, Sha256};
    let h = header();
    let mut hash: Vec<u8> = Sha256::digest(Sha256::digest(&h[..140 + 3 + 1344])).to_vec();
    hash.reverse();
    assert_eq!(
        hex::encode(hash),
        "004c99de90db50c3e994bf15103e0330701c35ddf87a55a875397e7a5d28e840"
    );
}

#[test]
fn every_recorded_testnet_header_verifies() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "hex") {
            continue;
        }
        let header = hex::decode(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert!(oracle_accepts(&header), "{path:?}");
        assert_eq!(verify(&header, 256).unwrap(), 2, "{path:?}");
        count += 1;
    }
    assert!(count >= 2);
}
