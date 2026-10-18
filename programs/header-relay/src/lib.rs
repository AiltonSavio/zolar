//! Header buffer and multi-transaction Equihash verification.
//!
//! One program-owned account holds an authority, a serialised Zcash header and
//! the verifier's carried state. Every instruction takes `[buffer, authority]`
//! and the authority must sign; the first instruction on a fresh buffer claims it.
//! - `0 | offset: u16 | bytes`: write header bytes (a header does not fit in one
//!   transaction). Only allowed while the state is clear.
//! - `1`: compute the BLAKE2b midstate and start verifying; the header is frozen.
//! - `2 | leaves: u16`: hash and merge the next `leaves` leaves (512 in total).
//! - `3`: clear the state so the buffer can take another header.
#![no_std]

use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};
use zcash_header::equihash::{self, HEADER_LEN, STATE_LEN};

pub const AUTHORITY_LEN: usize = 32;
pub const STATE_OFFSET: usize = (AUTHORITY_LEN + HEADER_LEN + 7) & !7;
pub const BUFFER_LEN: usize = STATE_OFFSET + STATE_LEN;

pub const IX_WRITE: u8 = 0;
pub const IX_BEGIN: u8 = 1;
pub const IX_STEP: u8 = 2;
pub const IX_RESET: u8 = 3;

#[cfg(not(feature = "no-entrypoint"))]
mod entry {
    use super::*;
    pinocchio::program_entrypoint!(process_instruction, 2);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}

fn verifier_error(e: equihash::Error) -> ProgramError {
    use equihash::Error::*;
    ProgramError::Custom(match e {
        MalformedHeader => 1,
        MalformedState => 2,
        WrongStage => 3,
        Collision => 4,
        OutOfOrder => 5,
        DuplicateIndex => 6,
        NonZeroRoot => 7,
    })
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let [buffer, authority] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !buffer.owned_by(program_id) || !buffer.is_writable() || buffer.data_len() != BUFFER_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut account = buffer.try_borrow_mut()?;
    let (front, state) = account.split_at_mut(STATE_OFFSET);
    let (owner, header) = front.split_at_mut(AUTHORITY_LEN);
    let header = &mut header[..HEADER_LEN];
    if owner.iter().all(|b| *b == 0) {
        owner.copy_from_slice(authority.address().as_ref());
    } else if owner != authority.address().as_ref() {
        return Err(ProgramError::IncorrectAuthority);
    }
    match data {
        [IX_WRITE, o0, o1, bytes @ ..] => {
            if !equihash::is_clear(state) {
                return Err(ProgramError::Custom(8));
            }
            let offset = u16::from_le_bytes([*o0, *o1]) as usize;
            let end = offset
                .checked_add(bytes.len())
                .filter(|end| *end <= HEADER_LEN)
                .ok_or(ProgramError::InvalidInstructionData)?;
            header[offset..end].copy_from_slice(bytes);
            Ok(())
        }
        [IX_BEGIN] => equihash::begin(header, state).map_err(verifier_error),
        [IX_STEP, l0, l1] => equihash::step(header, state, u16::from_le_bytes([*l0, *l1]))
            .map(|_| ())
            .map_err(verifier_error),
        [IX_RESET] => {
            state.fill(0);
            Ok(())
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
