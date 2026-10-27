//! Pure post-verification policy. No verifier bypass exists in the instruction processor.
use crate::{error::VendingError as E, state::*};
use header_relay_interface::{RelayView, ViewError, NETWORK_TESTNET};
use vending_types::PublicValues;

pub struct ClaimAccounts<'a> {
    pub machine_key: &'a [u8; 32],
    pub machine: &'a Machine,
    pub reservation: Option<&'a Reservation>,
    pub relay_owner: &'a [u8; 32],
    pub relay_key: &'a [u8; 32],
    pub relay_data: &'a [u8],
    pub tag_exists: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub struct ClaimPlan {
    pub payout: [u8; 32],
    pub payment_tag: [u8; 32],
    pub tickets: u64,
    pub close_reservation: bool,
}
pub fn view_error(e: ViewError) -> E {
    match e {
        ViewError::WrongOwner | ViewError::WrongAddress => E::WrongAccount,
        ViewError::Halted(_) => E::RelayHalted,
        _ => E::RelayNotReadable,
    }
}
pub fn validate_claim(p: &PublicValues, a: ClaimAccounts<'_>) -> Result<ClaimPlan, E> {
    let m = a.machine;
    if &p.machine != a.machine_key {
        return Err(E::WrongMachine);
    }
    if p.diversifier != m.diversifier {
        return Err(E::WrongDiversifier);
    }
    if p.svk_hash != m.svk_hash {
        return Err(E::WrongViewingKey);
    }
    let relay =
        RelayView::parse(a.relay_owner, a.relay_key, &m.relay, a.relay_data).map_err(view_error)?;
    if relay.network() != NETWORK_TESTNET || p.branch_id != NU7_BRANCH_ID {
        return Err(E::WrongBranch);
    }
    if !relay.is_known_root(&p.root) {
        return Err(E::UnknownRoot);
    }
    let r = a.reservation.ok_or(E::MissingReservation)?;
    if &r.machine != a.machine_key {
        return Err(E::WrongMachine);
    }
    if p.nonce != r.nonce {
        return Err(E::MissingReservation);
    }
    if p.payout != r.payout {
        return Err(E::WrongPayout);
    }
    if p.height > r.expiry_height {
        return Err(E::PaymentExpired);
    }
    if p.value_zat < r.price_zat {
        return Err(E::Underpaid);
    }
    if a.tag_exists {
        return Err(E::TagUsed);
    }
    Ok(ClaimPlan {
        payout: r.payout,
        payment_tag: p.payment_tag,
        tickets: 1,
        close_reservation: true,
    })
}
pub fn can_request_release(
    tip: u32,
    depth: u16,
    halted: bool,
    expiry: u32,
    created: i64,
    now: i64,
) -> Result<(), E> {
    let allowed = u64::from(tip) > u64::from(expiry) + u64::from(depth)
        || (halted
            && now
                .checked_sub(created)
                .is_some_and(|elapsed| elapsed >= BACKSTOP_SECS));
    if allowed {
        Ok(())
    } else {
        Err(E::ReleaseTooEarly)
    }
}

pub fn can_release(reservation: &Reservation, now: i64) -> Result<(), E> {
    if !reservation.release_requested {
        return Err(E::ReleaseNotRequested);
    }
    let end = reservation
        .release_requested_at
        .checked_add(CLAIM_WINDOW_SECS)
        .ok_or(E::ArithmeticOverflow)?;
    if now < end {
        return Err(E::ClaimWindowOpen);
    }
    Ok(())
}
