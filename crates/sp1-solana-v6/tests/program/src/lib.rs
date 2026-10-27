//! Test-only, account-free SBF adapter: `vkey_hash[32] || proof[356] || public_values`.
#![cfg_attr(target_os = "solana", no_std)]

use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};

pub fn process_instruction(
    _program_id: &Address,
    _accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (vkey, rest) = data
        .split_at_checked(32)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let (proof, values) = rest
        .split_at_checked(sp1_solana_v6::PROOF_LEN)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let vkey = vkey
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    sp1_solana_v6::verify_proof(proof, values, vkey)
        .map_err(|error| ProgramError::Custom(error as u32 + 1))
}

#[cfg(target_os = "solana")]
mod entry {
    pinocchio::program_entrypoint!(crate::process_instruction, 0);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}
