//! Escrow behaviour through the generated clients against the real Header Relay and Escrow programs.

use escrow_client::accounts::Escrow;
use escrow_tests::*;
use relay_tests::synthetic::CHECKPOINT_HEIGHT;
use solana_signer::Signer;
use zolar_claim::tx_merkle_root;

// Error codes of clients/codama/escrow.mjs.
const WRONG_ACCOUNT: u32 = 1;
const BAD_EXPIRY: u32 = 2;
const ZERO_AMOUNT: u32 = 3;
const COMMITMENT_OUT_OF_FIELD: u32 = 4;
const BAD_TOKEN_ACCOUNT: u32 = 5;
const RELAY_HALTED: u32 = 7;
const COMMITMENT_MISMATCH: u32 = 8;
const BLOCK_OUT_OF_RANGE: u32 = 9;
const UNKNOWN_ROOT: u32 = 11;
const POSEIDON_FAILED: u32 = 13;
const CLAIM_MODE_NOT_ALLOWED: u32 = 15;

const FINALITY_DEPTH: u16 = 3;

/// An Escrow created at the current tip, its payment mined in the next block (a block of five
/// transactions, the payment fourth) and that block final.
fn paid() -> (World, Deal, u32) {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(5), 3, 1, tip + 15);
    w.create(&deal);
    let block = w.mine_block_with(&deal.txids);
    w.finalize(block.height);
    (w, deal, block.height)
}

#[test]
fn a_reveal_claim_pays_the_payer_closes_the_escrow_and_returns_the_rent() {
    let (mut w, deal, height) = paid();
    assert_eq!(token_balance(&w.h, &deal.vault), Some(deal.amount));
    assert_eq!(
        token_balance(&w.h, &w.payee_token),
        Some(PAYEE_FUNDS - deal.amount)
    );
    let escrow_lamports = w.h.svm.get_account(&deal.escrow).unwrap().lamports;
    let vault_lamports = w.h.svm.get_account(&deal.vault).unwrap().lamports;
    let payee_before = w.h.svm.get_balance(&w.payee.pubkey()).unwrap();

    let claim = w.claim_ix(&deal, height, None);
    w.claim(claim);

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
fn a_claim_submitted_by_someone_else_still_pays_only_the_payer() {
    let (mut w, deal, height) = paid();
    // The harness fee payer is neither the Payee nor the Payer: a stranger who copied the claim.
    let stranger = w.h.payer.pubkey();
    assert_ne!(stranger, w.payer.pubkey());
    assert_ne!(stranger, w.payee.pubkey());

    // Naming their own token account as the destination is refused.
    let loot = solana_address::Address::new_unique();
    let mint = w.mint;
    w.h.svm
        .set_account(loot, token_account(&mint, &stranger, 0))
        .unwrap();
    let mut theirs = w.claim_ix(&deal, height, None);
    theirs.accounts[3].pubkey = loot;
    assert_eq!(w.claim_err(theirs), BAD_TOKEN_ACCOUNT);
    assert_eq!(token_balance(&w.h, &deal.vault), Some(deal.amount));

    // The copied claim itself succeeds and the tokens reach the Payer.
    let claim = w.claim_ix(&deal, height, None);
    w.claim(claim);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
    assert_eq!(token_balance(&w.h, &loot), Some(0));
}

#[test]
fn a_second_claim_fails() {
    let (mut w, deal, height) = paid();
    let claim = w.claim_ix(&deal, height, None);
    w.claim(claim.clone());
    let err = w.try_send(claim).expect_err("the Escrow is closed");
    assert!(
        matches!(
            err.err,
            solana_transaction_error::TransactionError::AlreadyProcessed
                | solana_transaction_error::TransactionError::InstructionError(..)
        ),
        "{:?}",
        err.err
    );
    // Re-funding the closed address does not revive it.
    w.h.svm.airdrop(&deal.escrow, 5_000_000).unwrap();
    let again = w.claim_ix(&deal, height, None);
    let err = w.try_send(again).expect_err("not an Escrow any more");
    assert!(custom_code(&err) == Some(WRONG_ACCOUNT) || custom_code(&err).is_none());
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}

#[test]
fn a_claim_against_an_older_recent_root_works() {
    let (mut w, deal, height) = paid();
    let size = (height - CHECKPOINT_HEIGHT) as usize;
    w.mine(3);
    let claim = w.claim_ix(&deal, height, Some(size));
    w.claim(claim);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}

#[test]
fn a_wrong_salt_transaction_or_path_is_rejected() {
    let (mut w, deal, height) = paid();

    let mut bad_salt = deal.clone();
    bad_salt.salt[31] ^= 1;
    let ix = w.claim_ix(&bad_salt, height, None);
    assert_eq!(w.claim_err(ix), COMMITMENT_MISMATCH);

    // A salt at or above the BN254 modulus is refused by the Poseidon syscall, not reduced.
    let mut over = deal.clone();
    over.salt = [0xff; 32];
    let ix = w.claim_ix(&over, height, None);
    assert_eq!(w.claim_err(ix), POSEIDON_FAILED);

    let mut other_tx = deal.clone();
    other_tx.index = 2;
    let ix = w.claim_ix(&other_tx, height, None);
    assert_eq!(w.claim_err(ix), COMMITMENT_MISMATCH);

    // The right transaction at the wrong position leads to another Merkle root.
    let ix = w.claim_ix_raw(&deal, height, w.tree().0.root(w.tree().1), vec![], 3);
    assert_eq!(w.claim_err(ix), UNKNOWN_ROOT);

    // A block that is not the payment's block.
    let ix = w.claim_ix(&deal, height - 1, None);
    assert_eq!(w.claim_err(ix), UNKNOWN_ROOT);

    let (tree, size) = w.tree();
    let mut path = tree.compress(tree.path((height - CHECKPOINT_HEIGHT - 1) as usize, size));
    path[1][0] ^= 1;
    let ix = w.claim_ix_raw(&deal, height, tree.root(size), path, deal.index);
    assert_eq!(w.claim_err(ix), UNKNOWN_ROOT);

    // The honest claim still works afterwards.
    let ix = w.claim_ix(&deal, height, None);
    w.claim(ix);
}

#[test]
fn a_self_consistent_forged_tree_is_rejected_because_its_root_is_not_the_relays() {
    let (mut w, deal, height) = paid();
    let (tree, size) = w.tree();
    let index = (height - CHECKPOINT_HEIGHT - 1) as usize;
    let leaf = zolar_claim::block_leaf(height, &tx_merkle_root(&deal.txids)).unwrap();
    // Arbitrary siblings: the path folds to some root of the forger's own making, so the Poseidon
    // check passes and only the membership in the relay's root history can stop it.
    let forged: Vec<[u8; 32]> = (0..20u8).map(|i| [i + 1; 32]).collect();
    let root = zolar_claim::FinalBlockTree::fold(&leaf, index, &forged);
    assert_ne!(root, tree.root(size));
    let before = token_balance(&w.h, &deal.vault);
    let ix = w.claim_ix_raw(&deal, height, root, forged, deal.index);
    assert_eq!(w.claim_err(ix), UNKNOWN_ROOT);
    assert_eq!(token_balance(&w.h, &deal.vault), before);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(0));
}

#[test]
fn a_root_older_than_the_relays_history_is_rejected() {
    let (mut w, deal, height) = paid();
    let old_size = w.tree().1;
    let honest = w.claim_ix(&deal, height, Some(old_size));
    w.mine(130); // the relay keeps the last 128 roots
    let (_, size) = w.tree();
    assert!(size - old_size >= 128);
    // The same claim that was valid is refused: the root aged out.
    let err = w.claim_err(honest);
    assert_eq!(err, UNKNOWN_ROOT);
    // Against a current root it works.
    let ix = w.claim_ix(&deal, height, None);
    w.claim(ix);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}

#[test]
fn a_claim_needs_the_relay_the_escrow_trusts() {
    let (mut w, deal, height) = paid();
    let mut ix = w.claim_ix(&deal, height, None);
    // Another account in the relay's place: not the stored address.
    ix.accounts[1].pubkey = w.payee.pubkey();
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT);
    // The rent destination must be the Payee.
    let mut ix = w.claim_ix(&deal, height, None);
    ix.accounts[4].pubkey = w.h.payer.pubkey();
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT);
    // A forged vault.
    let mut ix = w.claim_ix(&deal, height, None);
    ix.accounts[2].pubkey = w.payee_token;
    assert_eq!(w.claim_err(ix), WRONG_ACCOUNT);
}

#[test]
fn an_escrow_cannot_trust_an_account_the_header_relay_does_not_own() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(2), 0, 2, tip + 5);
    let mut ix = w.create_ix(&deal);
    // The Payee's own wallet is not a Header Relay.
    ix.accounts[4].pubkey = w.payee.pubkey();
    assert_eq!(w.create_err(ix), WRONG_ACCOUNT);
}

#[test]
fn a_halted_relay_stops_creation_and_claims() {
    let mut w = World::new(2);
    let tip = w.tip();
    let deal = w.deal(txids(4), 1, 3, tip + 15);
    w.create(&deal);
    let block = w.mine_block_with(&deal.txids);
    w.finalize(block.height);
    w.mine(2);
    let claim = w.claim_ix(&deal, block.height, None);

    // A heavier branch from the block after the Checkpoint's second block replaces final blocks.
    let mut fork = w.branch.fork_at(CHECKPOINT_HEIGHT + 3);
    let behind = w.branch.tip_height() - (CHECKPOINT_HEIGHT + 3);
    for i in 0..=behind {
        let b = fork.next(9000 + i);
        w.h.relay_synthetic(&w.relay, &b.raw).expect("fork block");
    }
    let data = w.h.relay_data(&w.relay);
    assert!(
        relay_tests::relay_status(&w.relay, &data).is_halted(),
        "the relay halted"
    );

    assert_eq!(w.claim_err(claim), RELAY_HALTED);
    let other = w.deal(txids(2), 0, 4, w.tip() + 5);
    let ix = w.create_ix(&other);
    assert_eq!(w.create_err(ix), RELAY_HALTED);
}

#[test]
fn a_payment_mined_after_the_expiry_cannot_be_claimed() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(3), 1, 5, tip + 2);
    w.create(&deal);
    w.mine(2);
    let block = w.mine_block_with(&deal.txids);
    assert_eq!(block.height, deal.expiry + 1);
    w.finalize(block.height);
    let ix = w.claim_ix(&deal, block.height, None);
    assert_eq!(w.claim_err(ix), BLOCK_OUT_OF_RANGE);
}

#[test]
fn only_a_payment_newer_than_the_replaceable_chain_below_creation_is_accepted() {
    // Finality Depth 3: blocks above the final cursor, the 2 below the tip plus the tip, can still be
    // replaced, so a payment broadcast after the Escrow may land as low as created_tip - 2.
    let mut w = World::new(FINALITY_DEPTH);
    let t = txids(3);
    let old = w.mine_block_with(&t); // height 4,000,003
    let deal = w.deal(t, 1, 6, old.height + 12);
    w.mine(2);
    assert_eq!(w.tip(), old.height + 2);
    // created_tip = old + 2 = old + depth - 1: the lowest height a payment broadcast now can land at.
    w.create(&deal);
    w.finalize(old.height);
    let ix = w.claim_ix(&deal, old.height, None);
    w.claim(ix);

    // One block later the same payment predates the replaceable stretch of chain.
    let mut w = World::new(FINALITY_DEPTH);
    let t = txids(3);
    let old = w.mine_block_with(&t);
    let deal = w.deal(t, 1, 6, old.height + 12);
    w.mine(3);
    assert_eq!(w.tip(), old.height + 3);
    w.create(&deal);
    w.finalize(old.height);
    let ix = w.claim_ix(&deal, old.height, None);
    assert_eq!(w.claim_err(ix), BLOCK_OUT_OF_RANGE);
}

#[test]
fn standard_external_wallet_expiry_is_bounded_without_rewriting_the_payment() {
    for ahead in [40, 120, 144] {
        let mut w = World::new(FINALITY_DEPTH);
        let deal = w.deal(txids(2), 0, 7, w.tip() + ahead);
        w.create(&deal);
        let data = w.h.svm.get_account(&deal.escrow).unwrap().data;
        let escrow = Escrow::from_bytes(&data).unwrap();
        assert_eq!(escrow.expiry_height, w.tip() + ahead);
    }
    let mut w = World::new(FINALITY_DEPTH);
    let deal = w.deal(txids(2), 0, 7, w.tip() + 145);
    let ix = w.create_ix(&deal);
    assert_eq!(w.create_err(ix), BAD_EXPIRY);
}

#[test]
fn create_checks_expiry_amount_bond_mode_commitment_and_the_vault() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();

    let deal = w.deal(txids(2), 0, 7, tip);
    let ix = w.create_ix(&deal);
    assert_eq!(w.create_err(ix), BAD_EXPIRY, "expiry at the tip");

    let deal = w.deal(txids(2), 0, 7, tip + 145);
    let ix = w.create_ix(&deal);
    assert_eq!(w.create_err(ix), BAD_EXPIRY, "more than 144 blocks ahead");

    let deal = w.deal(txids(2), 0, 7, tip + 20);
    let ix = w.create_ix(&deal);
    w.h.ok(&[ix], &[&w.payee.insecure_clone()]);
    let escrow = Escrow::from_bytes(&w.h.svm.get_account(&deal.escrow).unwrap().data).unwrap();
    assert_eq!(escrow.expiry_height, tip + 20);
    assert_eq!(escrow.created_tip, tip);
    assert_eq!(escrow.amount, deal.amount);
    assert_eq!(escrow.payout, w.payer.pubkey());
    assert_eq!(escrow.payee, w.payee.pubkey());
    assert_eq!(escrow.mint, w.mint);
    assert_eq!(escrow.vault, deal.vault);
    assert_eq!(escrow.relay, w.relay);
    assert_eq!(escrow.commitment, deal.commitment);
    assert_eq!(
        (escrow.state, escrow.claim_mode, escrow.bond_required),
        (0, 0, 0)
    );
    assert_eq!(
        (escrow.refund_requested_at, escrow.reserved),
        (0, [0u8; 55])
    );
    assert_eq!(w.h.svm.get_account(&deal.escrow).unwrap().data.len(), 300);

    // The same Payee cannot open a second Escrow for the same commitment.
    let ix = w.create_ix(&deal);
    assert!(w.h.send(&[ix], &[&w.payee.insecure_clone()]).is_err());

    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(2), 0, 8, tip + 5);
    let ix = w.create_ix_with(&deal, 0, 0, 0);
    assert_eq!(w.create_err(ix), ZERO_AMOUNT);
    let ix = w.create_ix_with(&deal, 10, 0, 2);
    assert_eq!(w.create_err(ix), CLAIM_MODE_NOT_ALLOWED);
    let mut over = deal.clone();
    over.commitment = [0xff; 32];
    let ix = w.create_ix_with(&over, 10, 0, 0);
    assert_eq!(w.create_err(ix), COMMITMENT_OUT_OF_FIELD);
}

#[test]
fn the_vault_must_belong_to_the_escrow_and_the_payee_must_hold_the_tokens() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(2), 0, 9, tip + 5);

    // A vault owned by someone else.
    let mint = w.mint;
    w.h.svm
        .set_account(deal.vault, token_account(&mint, &w.payee.pubkey(), 0))
        .unwrap();
    let ix = w.create_ix_with(&deal, deal.amount, 0, 0);
    assert_eq!(w.create_err(ix), BAD_TOKEN_ACCOUNT);

    // A vault with a delegate.
    let mut vault = token_account(&mint, &deal.escrow, 0);
    vault.data[72] = 1;
    w.h.svm.set_account(deal.vault, vault).unwrap();
    let ix = w.create_ix_with(&deal, deal.amount, 0, 0);
    assert_eq!(w.create_err(ix), BAD_TOKEN_ACCOUNT);

    // A vault with a close authority.
    let mut vault = token_account(&mint, &deal.escrow, 0);
    vault.data[129] = 1;
    w.h.svm.set_account(deal.vault, vault).unwrap();
    let ix = w.create_ix_with(&deal, deal.amount, 0, 0);
    assert_eq!(w.create_err(ix), BAD_TOKEN_ACCOUNT);

    // A vault of another mint.
    let other = solana_address::Address::new_unique();
    w.h.svm
        .set_account(deal.vault, token_account(&other, &deal.escrow, 0))
        .unwrap();
    let ix = w.create_ix_with(&deal, deal.amount, 0, 0);
    assert_eq!(w.create_err(ix), BAD_TOKEN_ACCOUNT);

    // More than the Payee holds.
    w.h.svm
        .set_account(deal.vault, token_account(&mint, &deal.escrow, 0))
        .unwrap();
    let ix = w.create_ix_with(&deal, PAYEE_FUNDS + 1, 0, 0);
    assert_eq!(w.create_err(ix), BAD_TOKEN_ACCOUNT);
}

#[test]
fn dust_in_the_escrow_address_and_the_vault_cannot_block_creation() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(5), 3, 10, tip + 15);
    // An observer sends lamports to the predictable Escrow address and tokens to the vault first.
    w.h.svm
        .set_account(
            deal.escrow,
            solana_account::Account {
                lamports: 1_000,
                ..Default::default()
            },
        )
        .unwrap();
    let mint = w.mint;
    w.h.svm
        .set_account(deal.vault, token_account(&mint, &deal.escrow, 7))
        .unwrap();
    let ix = w.create_ix_with(&deal, deal.amount, 0, 0);
    let payee = w.payee.insecure_clone();
    w.h.ok(&[ix], &[&payee]);

    let block = w.mine_block_with(&deal.txids);
    w.finalize(block.height);
    let ix = w.claim_ix(&deal, block.height, None);
    w.claim(ix);
    assert_eq!(
        token_balance(&w.h, &w.payer_token),
        Some(deal.amount + 7),
        "the donation goes to the Payer"
    );
}

#[test]
fn an_escrow_that_only_a_private_claim_may_settle_refuses_a_reveal_claim() {
    let (mut w, deal, height) = paid();
    let mut account = w.h.svm.get_account(&deal.escrow).unwrap();
    account.data[236] = 1;
    w.h.svm.set_account(deal.escrow, account).unwrap();
    let ix = w.claim_ix(&deal, height, None);
    assert_eq!(w.claim_err(ix), CLAIM_MODE_NOT_ALLOWED);
}

#[test]
fn claim_data_with_trailing_or_missing_bytes_is_rejected() {
    let (mut w, deal, height) = paid();
    let mut ix = w.claim_ix(&deal, height, None);
    ix.data.push(0);
    assert!(w.try_send(ix).is_err());
    let mut ix = w.claim_ix(&deal, height, None);
    ix.data.pop();
    assert!(w.try_send(ix).is_err());
    let ix = w.claim_ix(&deal, height, None);
    w.claim(ix);
}

#[test]
fn a_big_block_and_a_deeper_tree_still_fit_one_transaction() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let t = txids(200);
    let deal = w.deal(t, 137, 11, tip + 15);
    w.create(&deal);
    let block = w.mine_block_with(&deal.txids);
    w.mine(130); // the tree outgrows the 128 root history; the claim uses the newest root
    w.finalize(block.height);
    let ix = w.claim_ix(&deal, block.height, None);
    w.claim(ix);
    assert_eq!(token_balance(&w.h, &w.payer_token), Some(deal.amount));
}

/// Prints the compute units per instruction (run with `--nocapture`) and keeps them under a budget.
#[test]
fn compute_units_per_instruction() {
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let deal = w.deal(txids(5), 3, 12, tip + 15);
    let create = w.create(&deal);
    let block = w.mine_block_with(&deal.txids);
    w.finalize(block.height);
    let small = w.claim(w.claim_ix(&deal, block.height, None));

    let tip = w.tip();
    let big = w.deal(txids(200), 137, 13, tip + 15);
    w.create(&big);
    let block = w.mine_block_with(&big.txids);
    w.mine(130);
    w.finalize(block.height);
    let large = w.claim(w.claim_ix(&big, block.height, None));
    println!("CU CreateEscrow {create}");
    println!("CU RevealClaim (block of 5 transactions, young tree) {small}");
    println!(
        "CU RevealClaim (block of 200 transactions, tree of {} leaves) {large}",
        w.tree().1
    );
    assert!(create < 40_000 && small < 60_000 && large < 60_000);
}

#[test]
fn a_payout_account_with_a_delegate_or_close_authority_still_receives_the_claim() {
    for offset in [72usize, 129] {
        let (mut w, deal, height) = paid();
        let mut dest = w.h.svm.get_account(&w.payer_token).unwrap();
        dest.data[offset] = 1;
        dest.data[offset + 4..offset + 36].fill(9);
        w.h.svm.set_account(w.payer_token, dest).unwrap();
        let ix = w.claim_ix(&deal, height, None);
        w.claim(ix);
        assert_eq!(
            token_balance(&w.h, &w.payer_token),
            Some(deal.amount),
            "field at {offset}"
        );
    }
}

#[test]
fn a_frozen_or_foreign_payout_account_is_refused() {
    let (mut w, deal, height) = paid();
    let mut frozen = w.h.svm.get_account(&w.payer_token).unwrap();
    frozen.data[108] = 2;
    w.h.svm.set_account(w.payer_token, frozen).unwrap();
    let ix = w.claim_ix(&deal, height, None);
    assert_eq!(w.claim_err(ix), BAD_TOKEN_ACCOUNT);
}

#[test]
fn the_payee_account_cannot_be_the_vault() {
    // A token account that signs as the Payee and is also named as the vault would make the final
    // CloseAccount close an account into itself and strand the tokens.
    let mut w = World::new(FINALITY_DEPTH);
    let tip = w.tip();
    let mut deal = w.deal(txids(2), 0, 14, tip + 5);
    deal.vault = w.payee.pubkey();
    let mint = w.mint;
    let mut as_vault = token_account(&mint, &deal.escrow, 0);
    as_vault.lamports = 10_000_000_000;
    w.h.svm.set_account(deal.vault, as_vault).unwrap();
    // Rent already in the Escrow address, so no system transfer is needed from the token account.
    w.h.svm
        .set_account(
            deal.escrow,
            solana_account::Account {
                lamports: 5_000_000,
                ..Default::default()
            },
        )
        .unwrap();
    let ix = w.create_ix_with(&deal, deal.amount, 0, 0);
    let payee = w.payee.insecure_clone();
    assert_eq!(w.h.custom_err(&[ix], &[&payee]), BAD_TOKEN_ACCOUNT);
}
