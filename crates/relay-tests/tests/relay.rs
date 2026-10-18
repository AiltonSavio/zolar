use relay_tests::*;
use sha2::{Digest, Sha256};

#[test]
fn checkpoint_initialises_a_relay_at_the_checkpoint_block() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let relay_addr = h.init_relay(&checkpoint);

    let relay = h.relay(&relay_addr);
    assert_eq!(relay.state, 1, "sealed relay is active");
    assert_eq!(relay.halted, 0);
    assert_eq!(relay.tip_height, 4_483_599);
    assert_eq!(relay.tip_work, 0);
    assert_eq!(relay.tip_hash, checkpoint.hash);
    assert_eq!(relay.checkpoint.finality_depth, 10);
    assert_eq!(relay.checkpoint.times[112], checkpoint.times[112]);
}

fn sha256d(data: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(data)).into()
}

#[test]
fn a_real_testnet_header_is_accepted_through_the_generated_client() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let relay_addr = h.init_relay(&checkpoint);
    let raw = header(HEADER_4483600);
    assert_eq!(
        raw[4..36],
        checkpoint.hash,
        "the fixture header builds on the checkpoint block"
    );

    let cu = h.relay_header(&relay_addr, 0, &raw);
    println!("CU {cu:?}");
    for used in [cu.upload_rest_and_first_leaves, cu.last_leaves_and_accept] {
        assert!(used < CU_CAP as u64);
    }

    let relay = h.relay(&relay_addr);
    assert_eq!(relay.tip_height, 4_483_600);
    assert_eq!(relay.tip_work, 183);
    assert_eq!(relay.tip_hash, sha256d(&raw));
    let (store, data) = h.store(&relay_addr);
    assert_eq!(store.count, 1);
    let slot = &data[40..40 + 102];
    assert_eq!(slot[..32], sha256d(&raw));
    assert_eq!(slot[32..64], raw[36..68], "merkle root");
}

use header_relay_client::accounts::HeaderBuffer;
use solana_signer::Signer;

const UNKNOWN_PARENT: u32 = 12;
const DUPLICATE_HEADER: u32 = 13;
const HEADER_FROZEN: u32 = 8;
const RELAY_NOT_ACTIVE: u32 = 15;
const DIGEST_MISMATCH: u32 = 17;

#[test]
fn a_header_chains_onto_an_accepted_parent() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    h.relay_header(&relay_addr, 0, &header(HEADER_4483600));
    h.relay_header(&relay_addr, 1, &header(HEADER_4483601));

    let relay = h.relay(&relay_addr);
    assert_eq!(relay.tip_height, 4_483_601);
    assert_eq!(relay.tip_hash, sha256d(&header(HEADER_4483601)));
    let (store, data) = h.store(&relay_addr);
    assert_eq!(store.count, 2);
    let second = &data[40 + 102..40 + 204];
    assert_eq!(
        u16::from_le_bytes([second[96], second[97]]),
        0,
        "parent is slot 0"
    );
}

#[test]
fn a_header_with_an_unknown_parent_is_rejected() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    // 4,483,601 builds on 4,483,600, which this relay has not accepted.
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483601), &mut cu);
    let last = h.finish_ixs(&buffer, &relay_addr);
    assert_eq!(h.custom_err(&last, &[]), UNKNOWN_PARENT);
    assert_eq!(h.store(&relay_addr).0.count, 0);
}

#[test]
fn a_bad_equihash_solution_is_rejected() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut raw = header(HEADER_4483600);
    raw[600] ^= 1;
    let mut cu = Cu::default();
    let code = h
        .try_upload_and_verify(&relay_addr, 0, &raw, &mut cu)
        .expect_err("a corrupted solution must not verify");
    assert!((4..=7).contains(&code), "verifier error, got {code}");
    // The header was never verified, so it cannot be accepted.
    let buffer = Harness::buffer_addr(&relay_addr, &h.payer.pubkey(), 0);
    assert_eq!(h.custom_err(&[h.accept_ix(&buffer, &relay_addr)], &[]), 14);
    assert_eq!(h.store(&relay_addr).0.count, 0);
}

#[test]
fn a_header_cannot_change_once_verification_began() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483600), &mut cu);
    let code = h.custom_err(&[h.write_ix(&buffer, 0, &[0u8; 4])], &[]);
    assert_eq!(code, HEADER_FROZEN);
}

#[test]
fn an_accepted_header_cannot_be_accepted_again() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let raw = header(HEADER_4483600);
    h.relay_header(&relay_addr, 0, &raw);
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 1, &raw, &mut cu);
    let last = h.finish_ixs(&buffer, &relay_addr);
    assert_eq!(h.custom_err(&last, &[]), DUPLICATE_HEADER);
}

#[test]
fn closing_the_buffer_returns_its_rent() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let authority = h.payer.pubkey();
    let before = h.svm.get_balance(&authority).unwrap();
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483600), &mut cu);
    let rent = h.svm.get_balance(&buffer).unwrap();
    assert!(rent > 0);
    let last = h.finish_ixs(&buffer, &relay_addr);
    h.ok(&last, &[]);
    let mid = h.svm.get_balance(&authority).unwrap();
    h.ok(&[h.close_ix(&buffer)], &[]);
    let after = h.svm.get_balance(&authority).unwrap();
    assert!(h.svm.get_account(&buffer).is_none_or(|a| a.lamports == 0));
    assert_eq!(
        after - mid,
        rent - 5_000,
        "rent comes back, minus the 5,000 lamport fee"
    );
    assert!(
        before - after < rent,
        "relaying a header costs only fees, not rent"
    );
}

#[test]
fn another_relayer_cannot_use_or_close_my_buffer() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483600), &mut cu);
    let thief = solana_keypair::Keypair::new();
    h.svm.airdrop(&thief.pubkey(), 1_000_000_000).unwrap();
    let steal = header_relay_client::instructions::CloseBufferBuilder::new()
        .authority(thief.pubkey())
        .buffer(buffer)
        .instruction();
    // 0x...: IncorrectAuthority is a builtin error, not a custom one.
    let err = h.send(&[steal], &[&thief]).unwrap_err().err;
    assert_eq!(
        err,
        solana_transaction_error::TransactionError::InstructionError(
            0,
            solana_instruction::error::InstructionError::IncorrectAuthority
        )
    );
    assert!(h.svm.get_account(&buffer).is_some());
}

#[test]
fn nobody_can_create_a_buffer_in_another_relayers_name() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let victim = solana_keypair::Keypair::new();
    // The victim does not sign, so the payer cannot occupy the victim's buffer address.
    let mut ix = h.create_buffer_for(&victim.pubkey(), &relay_addr, 0);
    ix.accounts[0].is_signer = false;
    let err = h.send(&[ix], &[]).unwrap_err().err;
    assert_eq!(
        err,
        solana_transaction_error::TransactionError::InstructionError(
            0,
            solana_instruction::error::InstructionError::MissingRequiredSignature
        )
    );
    assert!(h
        .svm
        .get_account(&Harness::buffer_addr(&relay_addr, &victim.pubkey(), 0))
        .is_none());
}

#[test]
fn a_prefunded_address_cannot_block_creation() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let relay = Harness::relay_addr(&h.payer.pubkey(), &checkpoint);
    let store = Harness::store_addr(&relay);
    for target in [relay, store] {
        let ix = solana_system_transfer(&h.payer.pubkey(), &target, 1_000_000);
        h.ok(&[ix], &[]);
    }
    let relay_addr = h.init_relay(&checkpoint);
    assert_eq!(h.relay(&relay_addr).state, 1);
    let buffer = Harness::buffer_addr(&relay_addr, &h.payer.pubkey(), 0);
    let ix = solana_system_transfer(&h.payer.pubkey(), &buffer, 1_000_000);
    h.ok(&[ix], &[]);
    h.ok(&[h.create_buffer_ix(&relay_addr, 0)], &[]);
    assert_eq!(
        HeaderBuffer::from_bytes(&h.svm.get_account(&buffer).unwrap().data)
            .unwrap()
            .authority,
        h.payer.pubkey()
    );
}

#[test]
fn a_relay_must_be_sealed_with_the_checkpoint_its_address_commits_to() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let create = h.create_relay_ix(&checkpoint);
    h.ok(&[create], &[]);
    // Write a different checkpoint (finality depth 11) under the address of the first.
    let other = testnet_checkpoint(11, 0);
    let relay = Harness::relay_addr(&h.payer.pubkey(), &checkpoint);
    let bytes = borsh::to_vec(&other).unwrap();
    for (i, chunk) in bytes.chunks(480).enumerate() {
        let ix = header_relay_client::instructions::WriteCheckpointBuilder::new()
            .creator(h.payer.pubkey())
            .relay(relay)
            .offset((i * 480) as u16)
            .bytes(chunk.to_vec())
            .instruction();
        h.ok(&[ix], &[]);
    }
    assert_eq!(
        h.custom_err(&[h.seal_ix(&checkpoint)], &[]),
        DIGEST_MISMATCH
    );
    // And a pending relay accepts no buffers.
    assert_eq!(
        h.custom_err(&[h.create_buffer_ix(&relay, 0)], &[]),
        RELAY_NOT_ACTIVE
    );
}

#[test]
fn only_the_creator_writes_the_checkpoint_and_only_before_sealing() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let relay = Harness::relay_addr(&h.payer.pubkey(), &checkpoint);
    let create = h.create_relay_ix(&checkpoint);
    h.ok(&[create], &[]);
    let other = solana_keypair::Keypair::new();
    h.svm.airdrop(&other.pubkey(), 1_000_000_000).unwrap();
    let ix = header_relay_client::instructions::WriteCheckpointBuilder::new()
        .creator(other.pubkey())
        .relay(relay)
        .offset(0)
        .bytes(vec![1])
        .instruction();
    assert!(h.send(&[ix], &[&other]).is_err());
    for ix in h.write_checkpoint_ixs(&checkpoint) {
        h.ok(&[ix], &[]);
    }
    h.ok(&[h.seal_ix(&checkpoint)], &[]);
    let late = h.write_checkpoint_ixs(&checkpoint).remove(0);
    assert_eq!(h.custom_err(&[late], &[]), 22);
}

#[test]
fn relays_with_different_nonces_are_independent() {
    let mut h = Harness::new();
    let a = h.init_relay(&testnet_checkpoint(10, 0));
    let b = h.init_relay(&testnet_checkpoint(10, 1));
    assert_ne!(a, b);
    h.relay_header(&a, 0, &header(HEADER_4483600));
    assert_eq!(h.relay(&a).tip_height, 4_483_600);
    assert_eq!(h.relay(&b).tip_height, 4_483_599);
    assert_eq!(h.store(&b).0.count, 0);
}

#[test]
fn the_store_grows_in_steps_to_a_maximum() {
    let mut h = Harness::new();
    let relay = h.init_relay_ungrown(&testnet_checkpoint(10, 0));
    let store = Harness::store_addr(&relay);
    assert_eq!(h.store(&relay).0.capacity, 100);
    for capacity in [200, 300, 400] {
        h.grow_ix_ok(&relay);
        assert_eq!(h.store(&relay).0.capacity, capacity);
        assert_eq!(
            h.svm.get_account(&store).unwrap().data.len(),
            40 + capacity as usize * 102
        );
    }
    let grow = header_relay_client::instructions::GrowStoreBuilder::new()
        .payer(h.payer.pubkey())
        .store(store)
        .system_program(SYSTEM_PROGRAM)
        .instruction();
    assert_eq!(h.custom_err(&[grow], &[]), 21);
}

#[test]
fn headers_wait_for_a_store_big_enough_for_the_difficulty_context() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay_ungrown(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483600), &mut cu);
    let last = h.finish_ixs(&buffer, &relay_addr);
    assert_eq!(h.custom_err(&last, &[]), 24);
    h.grow_ix_ok(&relay_addr);
    h.ok(&last, &[]);
    assert_eq!(h.relay(&relay_addr).tip_height, 4_483_600);
}

#[test]
fn a_finality_depth_the_store_cannot_follow_is_refused() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(31, 0);
    let create = h.create_relay_ix(&checkpoint);
    h.ok(&[create], &[]);
    for ix in h.write_checkpoint_ixs(&checkpoint) {
        h.ok(&[ix], &[]);
    }
    assert_eq!(h.custom_err(&[h.seal_ix(&checkpoint)], &[]), 18);
}

#[test]
fn nobody_can_squat_another_deployers_relay_address() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let squatter = solana_keypair::Keypair::new();
    h.svm.airdrop(&squatter.pubkey(), 10_000_000_000).unwrap();
    // The squatter creates a relay for the very same Checkpoint first and never finishes it.
    let digest = digest_of(&checkpoint);
    let squat_relay = Harness::relay_addr(&squatter.pubkey(), &checkpoint);
    let squat = header_relay_client::instructions::CreateRelayBuilder::new()
        .payer(squatter.pubkey())
        .relay(squat_relay)
        .store(Harness::store_addr(&squat_relay))
        .system_program(SYSTEM_PROGRAM)
        .checkpoint_digest(digest)
        .instruction();
    h.ok(&[squat], &[&squatter]);
    // The deployer's relay lives at a different address and initialises normally.
    let mine = h.init_relay(&checkpoint);
    assert_ne!(mine, squat_relay);
    assert_eq!(h.relay(&mine).state, 1);
    assert_eq!(h.relay(&squat_relay).state, 0);
}

#[test]
fn a_header_whose_hash_is_above_its_target_is_rejected() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    // Equihash accepts the forged solution; only the target check can reject it.
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(FORGED_HASH_ABOVE_TARGET), &mut cu);
    let last = h.finish_ixs(&buffer, &relay_addr);
    assert_eq!(h.custom_err(&last, &[]), 11);
    assert_eq!(h.store(&relay_addr).0.count, 0);
}

#[test]
fn a_target_easier_than_the_network_limit_is_rejected() {
    // A mainnet Relay refuses the testnet header: its target (0x200165f8) is easier than mainnet's limit.
    let mut h = Harness::new();
    let mut checkpoint = testnet_checkpoint(10, 0);
    checkpoint.network = 0;
    let relay_addr = h.init_relay(&checkpoint);
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483600), &mut cu);
    let last = h.finish_ixs(&buffer, &relay_addr);
    assert_eq!(h.custom_err(&last, &[]), 10);
}

#[test]
fn reset_unfreezes_a_buffer_for_another_header() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(HEADER_4483600), &mut cu);
    assert_eq!(
        h.custom_err(&[h.write_ix(&buffer, 0, &[0u8; 4])], &[]),
        HEADER_FROZEN
    );
    let reset = header_relay_client::instructions::ResetBufferBuilder::new()
        .authority(h.payer.pubkey())
        .buffer(buffer)
        .instruction();
    h.ok(&[reset], &[]);
    h.ok(&[h.write_ix(&buffer, 0, &[0u8; 4])], &[]);
}

#[test]
fn an_empty_buffer_does_not_verify_and_header_writes_stay_in_bounds() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let buffer = Harness::buffer_addr(&relay_addr, &h.payer.pubkey(), 0);
    h.ok(&[h.create_buffer_ix(&relay_addr, 0)], &[]);
    assert_eq!(
        h.custom_err(&[h.step_ix(&buffer, 256)], &[]),
        1,
        "step on an empty buffer"
    );
    let err = h
        .send(&[h.write_ix(&buffer, 1480, &[0u8; 8])], &[])
        .unwrap_err()
        .err;
    assert_eq!(
        err,
        solana_transaction_error::TransactionError::InstructionError(
            0,
            solana_instruction::error::InstructionError::InvalidInstructionData
        )
    );
}

#[test]
fn a_block_version_below_four_is_rejected() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    let mut cu = Cu::default();
    // Equihash and the target both accept this forged header; only the version is wrong.
    let buffer = h.upload_and_verify(&relay_addr, 0, &header(FORGED_VERSION_3), &mut cu);
    let last = h.finish_ixs(&buffer, &relay_addr);
    assert_eq!(h.custom_err(&last, &[]), 25);
}

#[test]
fn accounts_of_different_relays_cannot_be_mixed() {
    let mut h = Harness::new();
    let a = h.init_relay(&testnet_checkpoint(10, 0));
    let b = h.init_relay(&testnet_checkpoint(10, 1));
    let mut cu = Cu::default();
    let buffer_a = h.upload_and_verify(&a, 0, &header(HEADER_4483600), &mut cu);
    // Buffer of relay A, relay B, store of relay A.
    let mut ix = h.accept_ix(&buffer_a, &b);
    ix.accounts[3].pubkey = Harness::store_addr(&a);
    let mut tx = vec![set_compute_limit(CU_CAP), h.step_ix(&buffer_a, 256)];
    tx.push(ix);
    assert_eq!(h.custom_err(&tx, &[]), 19);
    // Using relay A's buffer with relay B and B's own store fails too.
    let mut tx = vec![set_compute_limit(CU_CAP), h.step_ix(&buffer_a, 256)];
    tx.push(h.accept_ix(&buffer_a, &b));
    assert_eq!(h.custom_err(&tx, &[]), 19);
    assert_eq!(h.store(&a).0.count, 0);
    assert_eq!(h.store(&b).0.count, 0);
}

#[test]
fn a_foreign_account_is_not_accepted_as_a_buffer() {
    let mut h = Harness::new();
    let relay_addr = h.init_relay(&testnet_checkpoint(10, 0));
    // The relay account itself is program-owned but is not a buffer.
    let ix = h.write_ix(&relay_addr, 0, &[1u8; 4]);
    assert_eq!(h.custom_err(&[ix], &[]), 19);
    // A system-owned account is not either.
    let ix = h.write_ix(&h.payer.pubkey(), 0, &[1u8; 4]);
    assert!(h.send(&[ix], &[]).is_err());
}
