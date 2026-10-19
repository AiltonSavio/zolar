//! Private Claim through the generated client against the real Header Relay and Escrow programs, with a
//! real Groth16 proof (`circuits/fixtures/swap2.*`) of a payment of the ticket 08 devnet swap.

use escrow_tests::*;
use header_relay_interface::R_ROOTS;

// Error codes of clients/codama/escrow.mjs.
const BAD_TOKEN_ACCOUNT: u32 = 5;
const UNKNOWN_ROOT: u32 = 11;
const CLAIM_MODE_NOT_ALLOWED: u32 = 15;
const INVALID_PROOF: u32 = 16;

const FINALITY_DEPTH: u16 = 3;

/// An Escrow for the fixture payment, created when the relay tip is the one the proof was made for
/// (window lower bound 4,000,000, expiry 4,000,017), and the relay's newest root set to the proof's root.
fn funded(claim_mode: u8) -> (World, Deal, PrivateFixture) {
    let fx = PrivateFixture::load();
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    assert_eq!(
        tip, 4_000_002,
        "the proof was made for an Escrow created at tip 4,000,002"
    );
    let deal = w.deal_with_salt(fx.txids.clone(), fx.index, fx.salt, tip + 15);
    w.create_with_mode(&deal, claim_mode);
    w.forge_latest_root(fx.root());
    assert_eq!(
        deal.commitment, fx.public[1],
        "fixture commitment must equal the Escrow's"
    );
    (w, deal, fx)
}

#[test]
fn a_real_proof_pays_the_payer_and_closes_the_escrow() {
    let (mut w, deal, fx) = funded(1);
    let escrow_lamports = w.h.svm.get_account(&deal.escrow).unwrap().lamports;
    let vault_lamports = w.h.svm.get_account(&deal.vault).unwrap().lamports;
    let payee_before = w.h.svm.get_balance(&w.payee.pubkey()).unwrap();

    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    let cu = w.claim(ix);
    println!("CU PrivateClaim {cu}");

    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
    assert!(w
        .h
        .svm
        .get_account(&deal.escrow)
        .is_none_or(|a| a.lamports == 0));
    assert!(w
        .h
        .svm
        .get_account(&deal.vault)
        .is_none_or(|a| a.lamports == 0));
    let payee_after = w.h.svm.get_balance(&w.payee.pubkey()).unwrap();
    assert_eq!(payee_after - payee_before, escrow_lamports + vault_lamports);
}

#[test]
fn a_private_only_escrow_refuses_a_reveal_claim() {
    let (mut w, deal, _) = funded(1);
    let reveal = w.claim_ix(&deal, 4_000_004, None);
    assert_eq!(w.claim_err(reveal), CLAIM_MODE_NOT_ALLOWED);
}

#[test]
fn an_escrow_that_allows_both_settles_by_private_claim() {
    let (mut w, deal, fx) = funded(0);
    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    w.claim(ix);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}

#[test]
fn a_tampered_proof_is_rejected() {
    let (mut w, deal, fx) = funded(1);
    for part in 0..3 {
        for byte in [0usize, 31, 40] {
            let mut proof = fx.proof.clone();
            match part {
                0 => proof.a[byte] ^= 1,
                1 => proof.b[byte] ^= 1,
                _ => proof.c[byte] ^= 1,
            }
            let ix = w.private_claim_ix(&deal, &proof, fx.root());
            assert_eq!(w.claim_err(ix), INVALID_PROOF, "part {part} byte {byte}");
        }
    }
    // A proof that was not negated is also wrong.
    let mut proof = fx.proof.clone();
    proof.a = {
        let mut a = fx.proof.a;
        a[32..].copy_from_slice(&[0u8; 32]);
        a
    };
    let ix = w.private_claim_ix(&deal, &proof, fx.root());
    assert_eq!(w.claim_err(ix), INVALID_PROOF);
    assert_eq!(token_balance(&w.h, &deal.vault), Some(deal.amount));
}

#[test]
fn a_proof_for_another_commitment_is_rejected() {
    let fx = PrivateFixture::load();
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let mut other_salt = fx.salt;
    other_salt[31] ^= 1;
    let deal = w.deal_with_salt(fx.txids.clone(), fx.index, other_salt, tip + 15);
    w.create_with_mode(&deal, 1);
    w.forge_latest_root(fx.root());
    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    assert_eq!(w.claim_err(ix), INVALID_PROOF);
}

#[test]
fn a_proof_for_another_height_window_is_rejected() {
    // Created three blocks later: the program derives a different lower bound and expiry.
    let fx = PrivateFixture::load();
    let mut w = World::new(FINALITY_DEPTH);
    w.mine(3);
    let tip = w.tip();
    let deal = w.deal_with_salt(fx.txids.clone(), fx.index, fx.salt, tip + 15);
    w.create_with_mode(&deal, 1);
    w.forge_latest_root(fx.root());
    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    assert_eq!(w.claim_err(ix), INVALID_PROOF);
}

#[test]
fn a_root_the_relay_does_not_know_is_rejected_before_the_proof_is_checked() {
    let (mut w, deal, fx) = funded(1);
    let mut unknown = fx.root();
    unknown[31] ^= 1;
    let ix = w.private_claim_ix(&deal, &fx.proof, unknown);
    assert_eq!(w.claim_err(ix), UNKNOWN_ROOT);
}

#[test]
fn a_root_that_aged_out_of_the_history_is_rejected() {
    let (mut w, deal, fx) = funded(1);
    // 128 further appends overwrite the slot of the forged root.
    let appended = 128;
    let start = w.tip();
    w.finalize(start + appended);
    let data = w.h.relay_data(&w.relay);
    assert!(
        !data[R_ROOTS..].windows(32).any(|r| r == fx.root()),
        "root still stored"
    );
    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    assert_eq!(w.claim_err(ix), UNKNOWN_ROOT);
}

#[test]
fn the_proof_pays_only_the_stored_payout_whoever_submits_it() {
    let (mut w, deal, fx) = funded(1);
    let stranger = w.h.payer.pubkey();
    let loot = solana_address::Address::new_unique();
    let mint = w.mint;
    w.h.svm
        .set_account(loot, token_account(&mint, &stranger, 0))
        .unwrap();
    let mut theirs = w.private_claim_ix(&deal, &fx.proof, fx.root());
    theirs.accounts[3].pubkey = loot;
    assert_eq!(w.claim_err(theirs), BAD_TOKEN_ACCOUNT);
    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    w.claim(ix);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
    assert_eq!(token_balance(&w.h, &loot), Some(0));
}

#[test]
fn a_second_claim_with_the_same_proof_fails() {
    let (mut w, deal, fx) = funded(1);
    let ix = w.private_claim_ix(&deal, &fx.proof, fx.root());
    w.claim(ix.clone());
    assert!(w.try_send(ix).is_err());
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}
