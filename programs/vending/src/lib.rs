//! Testnet Vending Machine, driven by the Codama interface in clients/codama/vending.mjs.
#![cfg_attr(target_os = "solana", no_std)]
/// Solana devnet deployment. Regenerate Codama clients when changing this address.
pub const ID: pinocchio::Address = pinocchio::Address::new_from_array([
    254, 217, 29, 242, 33, 209, 58, 113, 184, 171, 127, 175, 251, 29, 180, 155, 121, 15, 105, 195,
    155, 4, 108, 104, 108, 148, 242, 122, 178, 160, 48, 222,
]);
pub mod error;
pub mod processor;
pub mod rules;
pub mod state;
#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod entry {
    pinocchio::program_entrypoint!(crate::processor::process_instruction, 12);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}
