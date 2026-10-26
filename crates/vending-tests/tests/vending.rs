use solana_signer::Signer;
use vending::{error::VendingError as E, state::*};
use vending_client::{
    accounts::{Machine as ClientMachine, Reservation as ClientReservation},
    VENDING_ID,
};
use vending_tests::*;

#[test]
fn init_config_is_immutable_and_mint_authority_is_pda() {
    let mut w = World::new();
    w.init(2);
    let m = w.machine_state();
    assert_eq!(m.available, 2);
    assert_eq!(m.price_zat, 2_500_000);
    assert_eq!(m.diversifier, [5; 11]);
    assert_eq!(m.svk_hash, [6; 32]);
    assert_eq!(m.vkey_hash, [7; 32]);
    assert_eq!(m.relay, *w.relay.as_array());
    let mint = w.h.svm.get_account(&w.mint.pubkey()).unwrap();
    assert_eq!(mint.owner, TOKEN);
    assert_eq!(mint.data.len(), 82);
    assert_eq!(&mint.data[..4], &1u32.to_le_bytes());
    assert_eq!(&mint.data[4..36], w.authority.as_ref());
    assert_eq!(&mint.data[36..44], &0u64.to_le_bytes());
    assert_eq!(mint.data[44], 0);
    assert_eq!(mint.data[45], 1);
    assert_eq!(&mint.data[46..50], &[0; 4]);
    let before = w.h.svm.get_account(&w.machine).unwrap();
    let ix = w.init_ix(999);
    assert_error(w.h.send(&[ix], &[&w.payee, &w.mint]), E::AlreadyInitialized);
    assert_eq!(w.h.svm.get_account(&w.machine).unwrap(), before);
    let mut unknown = w.init_ix(1);
    unknown.data = vec![4];
    assert!(w.h.send(&[unknown], &[&w.payee, &w.mint]).is_err());
    let decoded = ClientMachine::from_bytes(&before.data).unwrap();
    assert_eq!(decoded.price_zat, m.price_zat);
    assert_eq!(decoded.mint, w.mint.pubkey());
    assert_eq!(before.owner, VENDING_ID);
    assert_eq!(before.data.len(), MACHINE_LEN);
}
#[test]
fn reserve_decrements_stock_rejects_duplicate_nonce_and_sold_out() {
    let mut w = World::new();
    w.init(2);
    w.reserve([1; 32]).unwrap();
    assert_eq!(w.machine_state().available, 1);
    let r = w.reservation_state([1; 32]);
    assert_eq!(r.expiry_height, w.tip() + 2);
    assert_eq!(r.payout, *w.payer.pubkey().as_array());
    assert_eq!(r.created_at, START);
    assert_eq!(r.price_zat, 2_500_000);
    let a = w.h.svm.get_account(&w.reservation([1; 32])).unwrap();
    let decoded = ClientReservation::from_bytes(&a.data).unwrap();
    assert_eq!(decoded.nonce, [1; 32]);
    assert_eq!(decoded.expiry_height, r.expiry_height);
    assert_error(w.reserve([1; 32]), E::AlreadyInitialized);
    assert_eq!(w.machine_state().available, 1);
    w.reserve([2; 32]).unwrap();
    assert_error(w.reserve([3; 32]), E::SoldOut);
    assert_eq!(w.machine_state().available, 0);
}
#[test]
fn release_exact_boundary_returns_rent_and_stock_and_closes() {
    let mut w = World::new();
    w.init(1);
    w.reserve([1; 32]).unwrap();
    let r = w.reservation([1; 32]);
    let rent = w.h.svm.get_account(&r).unwrap().lamports;
    let payout_before = w.h.svm.get_account(&w.payer.pubkey()).unwrap().lamports;
    assert_error(w.release([1; 32]), E::ReleaseTooEarly);
    w.mine(3);
    assert_error(w.release([1; 32]), E::ReleaseTooEarly);
    w.mine(1);
    w.release([1; 32]).unwrap();
    assert_eq!(w.machine_state().available, 1);
    assert!(w.h.svm.get_account(&r).is_none());
    assert_eq!(
        w.h.svm.get_account(&w.payer.pubkey()).unwrap().lamports,
        payout_before + rent
    );
    assert_error(w.release([1; 32]), E::MissingReservation);
    assert_eq!(w.machine_state().available, 1);
}
#[test]
fn halt_requires_full_24_hours_even_if_height_allows_release() {
    let mut w = World::new();
    w.init(1);
    w.reserve([1; 32]).unwrap();
    w.mine(4);
    w.halt();
    w.time(START + BACKSTOP_SECS - 1);
    assert_error(w.release([1; 32]), E::ReleaseTooEarly);
    w.time(START + BACKSTOP_SECS);
    w.release([1; 32]).unwrap();
    assert_eq!(w.machine_state().available, 1);
}
#[test]
fn claim_rejects_every_proof_without_minting_or_closing() {
    let mut w = World::new();
    w.init(1);
    w.reserve([1; 32]).unwrap();
    let ma = w.h.svm.get_account(&w.machine).unwrap();
    let ra = w.h.svm.get_account(&w.reservation([1; 32])).unwrap();
    let mint = w.h.svm.get_account(&w.mint.pubkey()).unwrap();
    let ix = w.claim_ix([1; 32]);
    let tag = ix.accounts[8].pubkey;
    let ata = ix.accounts[7].pubkey;
    let result = w.h.send(&[ix], &[]);
    println!(
        "CU Claim invalid proof {}",
        result.as_ref().unwrap_err().meta.compute_units_consumed
    );
    assert_error(result, E::InvalidProof);
    assert_eq!(w.h.svm.get_account(&w.machine).unwrap(), ma);
    assert_eq!(w.h.svm.get_account(&w.reservation([1; 32])).unwrap(), ra);
    assert_eq!(w.h.svm.get_account(&w.mint.pubkey()).unwrap(), mint);
    assert!(w.h.svm.get_account(&tag).is_none());
    assert!(w.h.svm.get_account(&ata).is_none());
}
#[test]
fn dust_prefunded_pdas_do_not_block_init_or_reserve() {
    let mut w = World::new();
    w.h.svm.airdrop(&w.machine, 890_880).unwrap();
    w.init(1);
    let ra = w.reservation([1; 32]);
    w.h.svm.airdrop(&ra, 890_880).unwrap();
    w.reserve([1; 32]).unwrap();
    assert_eq!(w.reservation_state([1; 32]).nonce, [1; 32]);
    assert_eq!(w.machine_state().available, 0);
}
#[test]
fn release_rejects_account_substitutions_and_reserve_rejects_halt() {
    let mut w = World::new();
    w.init(1);
    w.reserve([1; 32]).unwrap();
    w.mine(4);
    let mut ix = w.release_ix([1; 32]);
    ix.accounts[4].pubkey = w.payee.pubkey();
    assert_error(w.h.send(&[ix], &[]), E::WrongAccount);
    let mut ix = w.release_ix([1; 32]);
    ix.accounts[3].pubkey = w.machine;
    assert_error(w.h.send(&[ix], &[]), E::WrongAccount);
    w.release([1; 32]).unwrap();
    w.halt();
    assert_error(w.reserve([2; 32]), E::RelayHalted);
}
#[test]
fn machine_and_reservation_pda_substitution_and_readonly_rejected() {
    let mut w = World::new();
    w.init(2);
    let mut ix = w.reserve_ix([1; 32]);
    ix.accounts[2].pubkey = w.reservation([2; 32]);
    assert_error(w.h.send(&[ix], &[&w.payer]), E::WrongAccount);
    let mut ix = w.reserve_ix([1; 32]);
    ix.accounts[1].is_writable = false;
    assert!(w.h.send(&[ix], &[&w.payer]).is_err());
    assert_eq!(w.machine_state().available, 2);
}
#[test]
fn claim_rejects_noncanonical_lengths() {
    let mut w = World::new();
    w.init(1);
    w.reserve([1; 32]).unwrap();
    for mode in 0..3 {
        let mut ix = w.claim_ix([1; 32]);
        match mode {
            0 => {
                ix.data.pop();
            }
            1 => ix.data.push(0),
            _ => ix.data[1..3].copy_from_slice(&0u16.to_le_bytes()),
        };
        assert!(w.h.send(&[ix], &[]).is_err());
    }
}
#[test]
fn claim_wire_size_with_compute_budget_fits_packet() {
    use solana_address::Address;
    use solana_instruction::Instruction;
    use solana_message::Message;
    let mut w = World::new();
    w.init(1);
    let ix = w.claim_ix([1; 32]);
    assert_eq!(ix.data.len(), 640);
    assert_eq!(&ix.data[1..3], &388u16.to_le_bytes());
    let compute = Address::from_str_const("ComputeBudget111111111111111111111111111111");
    let limit = relay_tests::set_compute_limit(1_400_000);
    let price = Instruction {
        program_id: compute,
        accounts: vec![],
        data: [vec![3], 1u64.to_le_bytes().to_vec()].concat(),
    };
    let message = Message::new(&[limit, price, ix], Some(&w.h.payer.pubkey()));
    let wire = 1 + 64 + message.serialize().len();
    println!("Claim transaction bytes {wire}");
    assert!(wire <= 1232);
    assert_eq!(wire, 1226);
}

#[test]
fn claim_checks_all_destination_and_program_bindings_before_verification() {
    use solana_instruction::error::InstructionError;
    use solana_transaction_error::TransactionError;
    let mut w = World::new();
    w.init(1);
    w.reserve([1; 32]).unwrap();
    for index in 4..=8 {
        let mut ix = w.claim_ix([1; 32]);
        ix.accounts[index].pubkey = w.payee.pubkey();
        assert_error(w.h.send(&[ix], &[]), E::WrongAccount);
    }
    for index in 9..=11 {
        let mut ix = w.claim_ix([1; 32]);
        ix.accounts[index].pubkey = w.machine;
        let err = w.h.send(&[ix], &[]).unwrap_err();
        assert_eq!(
            err.err,
            TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
        );
    }
    assert_eq!(w.machine_state().available, 0);
    assert_eq!(
        w.reservation_state([1; 32]).payout,
        *w.payer.pubkey().as_array()
    );
}
