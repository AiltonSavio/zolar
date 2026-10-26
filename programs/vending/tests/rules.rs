use header_relay_interface::*;
use vending::{error::VendingError as E, rules::*, state::*};
use vending_types::PublicValues;

struct Fixture {
    m: Machine,
    r: Reservation,
    p: PublicValues,
    relay: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let m = Machine {
            bump: 1,
            mint_bump: 2,
            payee: [1; 32],
            seed: [2; 32],
            mint: [3; 32],
            relay: [4; 32],
            price_zat: 50,
            available: 1,
            stock: 2,
            diversifier: [5; 11],
            svk_hash: [6; 32],
            vkey_hash: [7; 32],
            reservation_blocks: 20,
        };
        let r = Reservation {
            bump: 1,
            machine: [8; 32],
            nonce: [9; 32],
            payout: [10; 32],
            price_zat: 50,
            expiry_height: 120,
            created_at: 1000,
        };
        let p = PublicValues {
            version: 1,
            branch_id: NU7_BRANCH_ID,
            machine: r.machine,
            nonce: r.nonce,
            value: 50,
            payment_tag: [11; 32],
            root: [12; 32],
            height: 120,
            diversifier: m.diversifier,
            svk_hash: m.svk_hash,
            pool: 3,
        };
        let mut relay = vec![0; RELAY_LEN];
        relay[R_DISC] = DISC_RELAY;
        relay[R_VERSION] = 1;
        relay[R_STATE] = STATE_ACTIVE;
        relay[R_CHECKPOINT + CP_NETWORK] = NETWORK_TESTNET;
        relay[R_CHECKPOINT + CP_FINALITY_DEPTH..R_CHECKPOINT + CP_FINALITY_DEPTH + 2]
            .copy_from_slice(&10u16.to_le_bytes());
        relay[R_TIP_HEIGHT..R_TIP_HEIGHT + 4].copy_from_slice(&130u32.to_le_bytes());
        relay[R_ROOTS..R_ROOTS + 32].copy_from_slice(&p.root);
        Self { m, r, p, relay }
    }
    fn check(&self) -> Result<ClaimPlan, E> {
        self.check_with(Some(&self.r), false)
    }
    fn check_with(&self, r: Option<&Reservation>, tag_exists: bool) -> Result<ClaimPlan, E> {
        validate_claim(
            &self.p,
            ClaimAccounts {
                machine_key: &[8; 32],
                machine: &self.m,
                reservation: r,
                relay_owner: &HEADER_RELAY_PROGRAM,
                relay_key: &self.m.relay,
                relay_data: &self.relay,
                tag_exists,
            },
        )
    }
}
#[test]
fn valid_claim_plans_one_ticket_and_closure_to_stored_payout() {
    let f = Fixture::new();
    let p = f.check().unwrap();
    assert_eq!(p.payout, f.r.payout);
    assert_eq!(p.payment_tag, f.p.payment_tag);
    assert_eq!(p.tickets, 1);
    assert!(p.close_reservation);
}
#[test]
fn wrong_machine() {
    let mut f = Fixture::new();
    f.p.machine[0] ^= 1;
    assert_eq!(f.check(), Err(E::WrongMachine));
}
#[test]
fn wrong_reservation_machine() {
    let mut f = Fixture::new();
    f.r.machine[0] ^= 1;
    assert_eq!(f.check(), Err(E::WrongMachine));
}
#[test]
fn wrong_nonce() {
    let mut f = Fixture::new();
    f.p.nonce[0] ^= 1;
    assert_eq!(f.check(), Err(E::MissingReservation));
}
#[test]
fn missing_reservation() {
    let f = Fixture::new();
    assert_eq!(f.check_with(None, false), Err(E::MissingReservation));
}
#[test]
fn wrong_diversifier() {
    let mut f = Fixture::new();
    f.p.diversifier[0] ^= 1;
    assert_eq!(f.check(), Err(E::WrongDiversifier));
}
#[test]
fn wrong_viewing_key() {
    let mut f = Fixture::new();
    f.p.svk_hash[0] ^= 1;
    assert_eq!(f.check(), Err(E::WrongViewingKey));
}
#[test]
fn wrong_branch() {
    let mut f = Fixture::new();
    f.p.branch_id ^= 1;
    assert_eq!(f.check(), Err(E::WrongBranch));
}
#[test]
fn unknown_root() {
    let mut f = Fixture::new();
    f.p.root[0] ^= 1;
    assert_eq!(f.check(), Err(E::UnknownRoot));
}
#[test]
fn aged_root() {
    let mut f = Fixture::new();
    f.relay[R_LEAF_COUNT..R_LEAF_COUNT + 4].copy_from_slice(&128u32.to_le_bytes());
    f.relay[R_ROOTS..R_ROOTS + 32].fill(0);
    assert_eq!(f.check(), Err(E::UnknownRoot));
}
#[test]
fn halted_relay() {
    let mut f = Fixture::new();
    f.relay[R_HALTED] = 1;
    assert_eq!(f.check(), Err(E::RelayHalted));
}
#[test]
fn wrong_relay_owner_and_address() {
    let f = Fixture::new();
    for (owner, key) in [([0; 32], f.m.relay), (HEADER_RELAY_PROGRAM, [0; 32])] {
        assert_eq!(
            validate_claim(
                &f.p,
                ClaimAccounts {
                    machine_key: &[8; 32],
                    machine: &f.m,
                    reservation: Some(&f.r),
                    relay_owner: &owner,
                    relay_key: &key,
                    relay_data: &f.relay,
                    tag_exists: false
                }
            ),
            Err(E::WrongAccount)
        );
    }
}
#[test]
fn underpaid() {
    let mut f = Fixture::new();
    f.p.value = 49;
    assert_eq!(f.check(), Err(E::Underpaid));
    f.p.value = 51;
    assert!(f.check().is_ok());
}
#[test]
fn payment_after_expiry() {
    let mut f = Fixture::new();
    f.p.height = 121;
    assert_eq!(f.check(), Err(E::PaymentExpired));
}
#[test]
fn reused_tag() {
    let f = Fixture::new();
    assert_eq!(f.check_with(Some(&f.r), true), Err(E::TagUsed));
}
#[test]
fn version_and_pool() {
    let mut f = Fixture::new();
    f.p.version = 2;
    assert_eq!(f.check(), Err(E::BadPublicValues));
    f.p.version = 1;
    f.p.pool = 2;
    assert_eq!(f.check(), Err(E::BadPublicValues));
}
#[test]
fn release_boundaries_and_overflow() {
    assert_eq!(
        can_release(130, 10, false, 120, 1000, 1000),
        Err(E::ReleaseTooEarly)
    );
    assert!(can_release(131, 10, false, 120, 1000, 1000).is_ok());
    assert_eq!(
        can_release(500, 10, true, 120, 1000, 87399),
        Err(E::ReleaseTooEarly)
    );
    assert!(can_release(0, 10, true, 120, 1000, 87400).is_ok());
    assert_eq!(
        can_release(u32::MAX, 10, false, u32::MAX, 0, 100000),
        Err(E::ReleaseTooEarly)
    );
    assert_eq!(
        can_release(0, 10, true, 120, i64::MAX, i64::MAX),
        Err(E::ReleaseTooEarly)
    );
}
#[test]
fn account_layout_roundtrip_and_bad_lengths() {
    let f = Fixture::new();
    let mut m = [0; MACHINE_LEN];
    f.m.write(&mut m);
    assert_eq!(Machine::read(&m), Some(f.m));
    assert!(Machine::read(&m[..226]).is_none());
    let mut r = [0; RESERVATION_LEN];
    f.r.write(&mut r);
    assert_eq!(Reservation::read(&r), Some(f.r));
    r[0] = 0;
    assert!(Reservation::read(&r).is_none());
}

#[test]
fn contract_nonce_recreation_allows_old_unclaimed_payment_for_new_payout() {
    // Regression witness for the shared-contract issue: closing is not permanent nonce consumption.
    let mut f = Fixture::new();
    f.r.created_at += BACKSTOP_SECS;
    f.r.expiry_height += 100;
    f.r.payout = [99; 32];
    assert_eq!(f.check().unwrap().payout, [99; 32]);
}
