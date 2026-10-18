//! Header buffer and multi-transaction Equihash verification.
//!
//! One program-owned account holds a serialised Zcash header followed by the
//! verifier's carried state. Instructions:
//! - `0 | offset: u16 | bytes`: write header bytes (a header does not fit in one transaction).
//! - `1`: reset the state and compute the BLAKE2b midstate.
//! - `2 | leaves: u16`: hash and merge the next `leaves` leaves (up to 512 in total).
#![no_std]

use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};
use zcash_header::equihash::{self, HEADER_LEN, STATE_LEN};

pub const STATE_OFFSET: usize = (HEADER_LEN + 7) & !7;
pub const BUFFER_LEN: usize = STATE_OFFSET + STATE_LEN;

pub const IX_WRITE: u8 = 0;
pub const IX_BEGIN: u8 = 1;
pub const IX_STEP: u8 = 2;

#[cfg(not(feature = "no-entrypoint"))]
mod entry {
    use super::*;
    pinocchio::program_entrypoint!(process_instruction, 1);
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
    let [buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !buffer.owned_by(program_id) || !buffer.is_writable() || buffer.data_len() != BUFFER_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut account = buffer.try_borrow_mut()?;
    let (header, state) = account.split_at_mut(STATE_OFFSET);
    let header = &mut header[..HEADER_LEN];
    match data {
        [IX_WRITE, o0, o1, bytes @ ..] => {
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
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
