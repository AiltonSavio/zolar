use sha2::{Digest, Sha256};
use zcash_header::{
    header,
    pow::{check_pow, header_work, target_from_compact, work, Network, PowError, U256},
};

const HEADER_4483600: &str = include_str!("../fixtures/testnet-4483600.hex");

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn fixture() -> [u8; header::HEADER_LEN] {
    hex::decode(HEADER_4483600.trim())
        .unwrap()
        .try_into()
        .unwrap()
}

/// Values below were computed with Python big integers, independent of this crate:
/// `target = mantissa << 8 * (size - 3)`, `work = 2**256 // (target + 1)`.
#[test]
fn work_matches_big_integer_reference() {
    for (bits, expected) in [
        (0x200165f8u32, 183u128),
        (0x1e3100c9, 342_370),
        (0x2007ffff, 32),
        (0x1f07ffff, 8192),
        (0x1d00ffff, 4_295_032_833),
        (0x1c1d6f3a, 37_354_547_255),
    ] {
        let target = target_from_compact(bits).unwrap();
        assert_eq!(work(&target), Some(expected), "bits {bits:08x}");
    }
}

#[test]
fn compact_edge_cases_are_rejected_like_consensus() {
    assert_eq!(target_from_compact(0), None);
    assert_eq!(target_from_compact(0x01003456), None, "zero after shift");
    assert_eq!(target_from_compact(0x04923456), None, "negative");
    assert_eq!(target_from_compact(0x23000001), None, "overflow");
    assert_eq!(target_from_compact(0x01123456), Some(U256([0x12, 0, 0, 0])));
}

#[test]
fn real_testnet_header_meets_its_target() {
    let header = fixture();
    let hash = header::hash(sha256, &header);
    assert_eq!(header::bits(&header), 0x200165f8);
    assert!(check_pow(Network::Testnet, 0x200165f8, &hash).is_ok());
    assert_eq!(header_work(sha256, Network::Testnet, &header), Ok(183));
}

#[test]
fn hash_above_target_is_rejected() {
    let hash = header::hash(sha256, &fixture());
    // The real hash begins with zero bytes at the top; a target below it must reject.
    let hash_as_target = U256::from_le_bytes(&hash);
    assert_ne!(hash_as_target, U256::ZERO);
    assert_eq!(
        check_pow(Network::Testnet, 0x1d00ffff, &hash),
        Err(PowError::HashAboveTarget)
    );
}

#[test]
fn target_easier_than_the_network_limit_is_rejected() {
    let hash = [0u8; 32];
    assert_eq!(
        check_pow(Network::Mainnet, 0x2007ffff, &hash),
        Err(PowError::AboveLimit)
    );
    assert!(check_pow(Network::Testnet, 0x2007ffff, &hash).is_ok());
    assert_eq!(
        check_pow(Network::Testnet, 0, &hash),
        Err(PowError::InvalidBits)
    );
}

#[test]
fn work_that_does_not_fit_128_bits_is_refused() {
    // Target 1 would mean 2^255 hashes.
    assert_eq!(work(&U256([1, 0, 0, 0])), None);
    assert_eq!(
        work(&target_from_compact(0x1d00ffff).unwrap()),
        Some(4_295_032_833)
    );
}

#[test]
fn the_fixture_has_a_valid_version() {
    assert_eq!(header::version(&fixture()), 4);
    assert!(header::version(&fixture()) >= header::MIN_VERSION);
}
