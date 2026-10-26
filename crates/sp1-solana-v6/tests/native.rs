use sp1_solana_v6::{verify_proof, Error};

const PROOF: &[u8; 356] = include_bytes!("fixtures/proof.bin");
const VALUES: &[u8] = include_bytes!("fixtures/public_values.bin");
const VKEY: &[u8; 32] = include_bytes!("fixtures/vkey_hash.bin");

#[test]
fn verifies_published_v6_1_proof_with_v6_10_key() {
    assert_eq!(verify_proof(PROOF, VALUES, VKEY), Ok(()));
}

#[test]
fn rejects_tampered_proof() {
    let mut proof = *PROOF;
    proof[355] ^= 1;
    assert!(verify_proof(&proof, VALUES, VKEY).is_err());
}

#[test]
fn rejects_tampered_public_values() {
    let mut values = VALUES.to_vec();
    values[0] ^= 1;
    assert!(verify_proof(PROOF, &values, VKEY).is_err());
}

#[test]
fn rejects_wrong_vkey_hash() {
    let mut vkey = *VKEY;
    vkey[31] ^= 1;
    assert!(verify_proof(PROOF, VALUES, &vkey).is_err());
}

#[test]
fn does_not_discard_vkey_high_byte() {
    let mut vkey = *VKEY;
    vkey[0] = 1;
    assert!(verify_proof(PROOF, VALUES, &vkey).is_err());
}

#[test]
fn rejects_nonzero_exit_code() {
    let mut proof = *PROOF;
    proof[35] = 1;
    assert_eq!(
        verify_proof(&proof, VALUES, VKEY),
        Err(Error::ExitCodeMismatch)
    );
}

#[test]
fn rejects_untrusted_vk_root() {
    let mut proof = *PROOF;
    proof[67] ^= 1;
    assert_eq!(
        verify_proof(&proof, VALUES, VKEY),
        Err(Error::VkeyRootMismatch)
    );
}

#[test]
fn binds_proof_nonce() {
    let mut proof = *PROOF;
    proof[99] ^= 1;
    assert!(verify_proof(&proof, VALUES, VKEY).is_err());
}

#[test]
fn rejects_wrong_selector() {
    let mut proof = *PROOF;
    proof[0] ^= 1;
    assert_eq!(
        verify_proof(&proof, VALUES, VKEY),
        Err(Error::Groth16VkeyHashMismatch)
    );
}

#[test]
fn rejects_first_party_v6_0_fixture_from_an_earlier_circuit() {
    let proof = include_bytes!("fixtures/v6_0_proof.bin");
    assert_eq!(
        verify_proof(proof, VALUES, VKEY),
        Err(Error::Groth16VkeyHashMismatch)
    );
}

#[test]
fn rejects_every_truncation_and_trailing_byte() {
    for len in 0..PROOF.len() {
        assert_eq!(
            verify_proof(&PROOF[..len], VALUES, VKEY),
            Err(Error::InvalidProofLength)
        );
    }
    let mut proof = PROOF.to_vec();
    proof.push(0);
    assert_eq!(
        verify_proof(&proof, VALUES, VKEY),
        Err(Error::InvalidProofLength)
    );
}

#[test]
fn rejects_noncanonical_vkey_and_nonce_scalars() {
    assert_eq!(
        verify_proof(PROOF, VALUES, &[255; 32]),
        Err(Error::InvalidPublicInput)
    );
    let mut proof = *PROOF;
    proof[68..100].fill(255);
    assert_eq!(
        verify_proof(&proof, VALUES, VKEY),
        Err(Error::InvalidPublicInput)
    );
}

#[test]
fn rejects_noncanonical_coordinates() {
    for offset in (100..356).step_by(32) {
        let mut proof = *PROOF;
        proof[offset..offset + 32].fill(255);
        assert_eq!(verify_proof(&proof, VALUES, VKEY), Err(Error::InvalidPoint));
    }
}

#[test]
fn rejects_points_at_infinity() {
    for (start, end) in [(100, 164), (164, 292), (292, 356)] {
        let mut proof = *PROOF;
        proof[start..end].fill(0);
        assert_eq!(verify_proof(&proof, VALUES, VKEY), Err(Error::InvalidPoint));
    }
}
