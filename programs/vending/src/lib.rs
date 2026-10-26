//! Testnet Vending Machine, driven by the Codama interface in clients/codama/vending.mjs.
#![cfg_attr(target_os = "solana", no_std)]
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
