//! Bond and refund behaviour through the generated clients against the real Header Relay and Escrow.

use escrow_client::accounts::Escrow;
use escrow_tests::*;
use solana_signer::Signer;

// Error codes of clients/codama/escrow.mjs.
const WRONG_ACCOUNT: u32 = 1;
const BAD_TOKEN_ACCOUNT: u32 = 5;
const NOT_OPEN: u32 = 6;
const RELAY_HALTED: u32 = 7;
const BOND_NOT_DEPOSITED: u32 = 14;
const NO_BOND_REQUIRED: u32 = 16;
const BOND_ALREADY_DEPOSITED: u32 = 17;
const REFUND_NOT_YET: u32 = 18;
const REFUND_NOT_REQUESTED: u32 = 19;
const CLAIM_WINDOW_OPEN: u32 = 20;
const NOT_PAYER: u32 = 21;

const FINALITY_DEPTH: u16 = 3;
const BOND: u64 = 200_000;

/// An Escrow requiring a Bond of `bond` created at the current tip, expiring 15 blocks later.
fn escrow_with_bond(bond: u64) -> (World, Deal) {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(5), 3, 1, tip + 15);
    w.create_with_bond(&deal, bond);
    w.fund_payer(bond);
    (w, deal)
}

/// Relays blocks until the tip is `height`.
fn mine_to(w: &mut World, height: u32) {
    while w.tip() < height {
        w.mine(1);
    }
}

fn gone(w: &World, deal: &Deal) -> bool {
    [deal.escrow, deal.vault]
        .iter()
        .all(|a| w.h.svm.get_account(a).is_none_or(|a| a.lamports == 0))
}

#[test]
fn a_bond_is_locked_by_the_payer_and_returned_with_the_claim() {
    let (mut w, deal) = escrow_with_bond(BOND);
    let escrow = Escrow::from_bytes(&w.h.svm.get_account(&deal.escrow).unwrap().data).unwrap();
    assert_eq!((escrow.bond_required, escrow.bond_deposited), (BOND, 0));
    assert_eq!(token_balance(&w.h, &deal.vault), Some(deal.amount));

    w.deposit_bond(&deal);
    let escrow = Escrow::from_bytes(&w.h.svm.get_account(&deal.escrow).unwrap().data).unwrap();
    assert_eq!(escrow.bond_deposited, BOND);
    assert_eq!(token_balance(&w.h, &deal.vault), Some(deal.amount + BOND));
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(0));

    let block = w.mine_block_with(&deal.txids);
    w.finalize(block.height);
    let claim = w.claim_ix(&deal, block.height, None);
    w.claim(claim);
    assert_eq!(
        token_balance(&w.h, &w.payer_token),
        Some(deal.amount + BOND),
        "the Payer gets the tokens and the Bond back"
    );
    assert_eq!(
        token_balance(&w.h, &w.payee_token),
        Some(PAYEE_FUNDS - deal.amount)
    );
    assert!(gone(&w, &deal));
}

#[test]
fn a_claim_waits_for_the_bond_the_escrow_requires() {
    let (mut w, deal) = escrow_with_bond(BOND);
    let block = w.mine_block_with(&deal.txids);
    w.finalize(block.height);
    let claim = w.claim_ix(&deal, block.height, None);
    assert_eq!(w.claim_err(claim.clone()), BOND_NOT_DEPOSITED);

    w.deposit_bond(&deal);
    w.claim(claim);
    assert_eq!(
        token_balance(&w.h, &w.payer_token),
        Some(deal.amount + BOND)
    );
}

#[test]
fn only_the_payer_wallet_deposits_the_exact_bond_once_and_not_after_a_refund_request() {
    let (mut w, deal) = escrow_with_bond(BOND);

    // A different signer, even holding tokens, is refused.
    let other = solana_keypair::Keypair::new();
    let other_token = solana_address::Address::new_unique();
    w.h.svm
        .set_account(other_token, token_account(&w.mint, &other.pubkey(), BOND))
        .unwrap();
    let mut ix = w.deposit_bond_ix(&deal);
    ix.accounts[0].pubkey = other.pubkey();
    ix.accounts[2].pubkey = other_token;
    assert_eq!(
        w.h.custom_err(&[ix], &[&other]),
        NOT_PAYER,
        "another wallet's Bond would be returned to the Payer"
    );

    // The Payer signing for someone else's tokens, or a wrong vault.
    let mut ix = w.deposit_bond_ix(&deal);
    ix.accounts[2].pubkey = other_token;
    assert_eq!(w.deposit_bond_err(ix), BAD_TOKEN_ACCOUNT);
    let mut ix = w.deposit_bond_ix(&deal);
    ix.accounts[3].pubkey = w.payee_token;
    assert_eq!(w.deposit_bond_err(ix), WRONG_ACCOUNT);

    w.deposit_bond(&deal);
    w.fund_payer(BOND);
    let ix = w.deposit_bond_ix(&deal);
    assert_eq!(w.deposit_bond_err(ix), BOND_ALREADY_DEPOSITED);
    assert_eq!(token_balance(&w.h, &deal.vault), Some(deal.amount + BOND));

    // After a refund request the Bond would only feed the Refund.
    let (mut w, deal) = escrow_with_bond(BOND);
    mine_to(&mut w, deal.expiry + FINALITY_DEPTH as u32 + 1);
    let request = w.request_refund_ix(&deal);
    w.claim(request);
    let ix = w.deposit_bond_ix(&deal);
    assert_eq!(w.deposit_bond_err(ix), NOT_OPEN);

    // An Escrow with no Bond takes none.
    let (mut w, deal) = escrow_with_bond(0);
    w.fund_payer(BOND);
    let ix = w.deposit_bond_ix(&deal);
    assert_eq!(w.deposit_bond_err(ix), NO_BOND_REQUIRED);
}

#[test]
fn a_refund_by_height_waits_for_the_tip_to_pass_the_expiry_plus_finality_depth_and_then_the_window()
{
    let (mut w, deal) = escrow_with_bond(BOND);
    w.deposit_bond(&deal);
    let passes = deal.expiry + FINALITY_DEPTH as u32;
    let payee_before = token_balance(&w.h, &w.payee_token).unwrap();

    mine_to(&mut w, passes - 1);
    let ix = w.request_refund_ix(&deal);
    assert_eq!(w.claim_err(ix), REFUND_NOT_YET, "early refund");
    mine_to(&mut w, passes);
    let ix = w.request_refund_ix(&deal);
    assert_eq!(
        w.claim_err(ix),
        REFUND_NOT_YET,
        "the block at the expiry height only just became claimable"
    );

    mine_to(&mut w, passes + 1);
    let ix = w.request_refund_ix(&deal);
    assert_eq!(w.refund_err(&deal), REFUND_NOT_REQUESTED);
    w.claim(ix.clone());
    let escrow = Escrow::from_bytes(&w.h.svm.get_account(&deal.escrow).unwrap().data).unwrap();
    assert_eq!((escrow.state, escrow.refund_requested_at), (1, w.now()));
    w.advance_time(1);
    assert_eq!(w.claim_err(ix), NOT_OPEN, "a second request");

    assert_eq!(w.refund_err(&deal), CLAIM_WINDOW_OPEN);
    w.advance_time(CLAIM_WINDOW_SECS - 2);
    assert_eq!(w.refund_err(&deal), CLAIM_WINDOW_OPEN, "one second short");
    w.advance_time(1);
    let refund = w.refund_ix(&deal);
    w.claim(refund);

    assert_eq!(
        token_balance(&w.h, &w.payee_token),
        Some(payee_before + deal.amount + BOND),
        "the Payee gets the tokens back and keeps the Bond"
    );
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(0));
    assert!(gone(&w, &deal));
}

#[test]
fn the_backstop_refunds_after_24_hours_with_a_halted_relay() {
    let mut w = World::new(2);
    let tip = w.tip();
    let deal = w.deal(txids(5), 3, 1, tip + 15);
    w.create_with_bond(&deal, 0);
    w.mine(4);
    w.halt();
    let ix = w.request_refund_ix(&deal);
    assert_eq!(
        w.claim_err(ix.clone()),
        RELAY_HALTED,
        "height needs a running relay"
    );
    w.advance_time(BACKSTOP_SECS - (w.now() - START_TIME) - 1);
    assert_eq!(
        w.claim_err(ix.clone()),
        RELAY_HALTED,
        "a second short of 24 hours"
    );

    w.advance_time(1);
    w.claim(ix);
    w.advance_time(CLAIM_WINDOW_SECS);
    let refund = w.refund_ix(&deal);
    w.claim(refund);
    assert_eq!(token_balance(&w.h, &w.payee_token), Some(PAYEE_FUNDS));
    assert!(gone(&w, &deal));
}

#[test]
fn a_halt_after_the_request_sends_the_payee_back_to_the_backstop() {
    let mut w = World::new(2);
    let tip = w.tip();
    let deal = w.deal(txids(5), 3, 1, tip + 15);
    w.create_with_bond(&deal, 0);
    mine_to(&mut w, deal.expiry + 3);
    let request = w.request_refund_ix(&deal);
    w.claim(request);
    w.halt();
    w.advance_time(CLAIM_WINDOW_SECS);
    assert_eq!(
        w.refund_err(&deal),
        RELAY_HALTED,
        "claims are impossible while Halted, so the window proves nothing"
    );
    w.set_time(START_TIME + BACKSTOP_SECS);
    let refund = w.refund_ix(&deal);
    w.claim(refund);
    assert_eq!(token_balance(&w.h, &w.payee_token), Some(PAYEE_FUNDS));
}

#[test]
fn the_backstop_counts_from_creation_not_from_the_start_of_the_test() {
    let (mut w, deal) = escrow_with_bond(0);
    let created = Escrow::from_bytes(&w.h.svm.get_account(&deal.escrow).unwrap().data)
        .unwrap()
        .created_at;
    assert_eq!(created, START_TIME);
    w.set_time(created + BACKSTOP_SECS - 1);
    let ix = w.request_refund_ix(&deal);
    assert_eq!(w.claim_err(ix.clone()), REFUND_NOT_YET);
    w.set_time(created + BACKSTOP_SECS);
    w.claim(ix);
}

#[test]
fn a_payer_whose_payment_finalises_in_the_window_still_claims_and_gets_the_bond_back() {
    let (mut w, deal) = escrow_with_bond(BOND);
    w.deposit_bond(&deal);
    let block = w.mine_block_with(&deal.txids);
    // The relay lags: the block is not final, but the tip is past the expiry when the Payee asks.
    let late = deal.expiry + FINALITY_DEPTH as u32 + 1;
    let claim = {
        mine_to(&mut w, late);
        w.finalize(block.height);
        w.claim_ix(&deal, block.height, None)
    };
    let request = w.request_refund_ix(&deal);
    w.claim(request);
    w.advance_time(CLAIM_WINDOW_SECS - 1);
    w.claim(claim.clone());
    assert_eq!(
        token_balance(&w.h, &w.payer_token),
        Some(deal.amount + BOND)
    );
    assert!(gone(&w, &deal));

    w.advance_time(10);
    assert_eq!(w.refund_err(&deal), WRONG_ACCOUNT, "nothing left to refund");
}

#[test]
fn a_claim_that_lands_after_the_window_but_before_the_refund_still_wins() {
    let (mut w, deal) = escrow_with_bond(0);
    let block = w.mine_block_with(&deal.txids);
    mine_to(&mut w, deal.expiry + FINALITY_DEPTH as u32 + 1);
    let request = w.request_refund_ix(&deal);
    w.claim(request);
    w.advance_time(CLAIM_WINDOW_SECS * 4);
    let claim = w.claim_ix(&deal, block.height, None);
    w.claim(claim);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}

#[test]
fn refund_accounts_must_be_the_escrows_own() {
    let (mut w, deal) = escrow_with_bond(BOND);
    w.deposit_bond(&deal);
    mine_to(&mut w, deal.expiry + FINALITY_DEPTH as u32 + 1);

    // Another relay account, even a real Header Relay look-alike, is not the trusted relay.
    let mut ix = w.request_refund_ix(&deal);
    ix.accounts[1].pubkey = w.payee_token;
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT);

    let ix = w.request_refund_ix(&deal);
    w.claim(ix);
    w.advance_time(CLAIM_WINDOW_SECS);

    // The refund goes only to a token account the Payee owns.
    let mut ix = w.refund_ix(&deal);
    ix.accounts[3].pubkey = w.payer_token;
    assert_eq!(w.claim_err(ix), BAD_TOKEN_ACCOUNT);
    let mut ix = w.refund_ix(&deal);
    ix.accounts[2].pubkey = w.payee_token;
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT);
    let mut ix = w.refund_ix(&deal);
    ix.accounts[4].pubkey = w.payer.pubkey();
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT);
    let mut ix = w.refund_ix(&deal);
    ix.accounts[1].pubkey = w.payee_token;
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT, "not the Escrow's relay");
    let ix = w.refund_ix(&deal);
    w.claim(ix);
}

#[test]
fn compute_units_of_the_bond_and_refund_instructions() {
    let (mut w, deal) = escrow_with_bond(BOND);
    let deposit = w.deposit_bond(&deal);
    mine_to(&mut w, deal.expiry + FINALITY_DEPTH as u32 + 1);
    let request_ix = w.request_refund_ix(&deal);
    let by_height = w.claim(request_ix);
    w.advance_time(CLAIM_WINDOW_SECS);
    let refund_ix = w.refund_ix(&deal);
    let refund = w.claim(refund_ix);

    let (mut w, deal) = escrow_with_bond(0);
    w.advance_time(BACKSTOP_SECS);
    let request_ix = w.request_refund_ix(&deal);
    let by_backstop = w.claim(request_ix);

    println!("CU DepositBond {deposit}");
    println!("CU RequestRefund by height {by_height}");
    println!("CU RequestRefund by backstop {by_backstop}");
    println!("CU Refund {refund}");
    assert!(deposit < 30_000 && by_height < 30_000 && by_backstop < 30_000 && refund < 30_000);
}
