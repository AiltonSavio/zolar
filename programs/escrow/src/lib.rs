//! Escrow program: SPL tokens a Payee locks for a Payer, released once a shielded Zcash payment is
//! proven final against a Header Relay.
//!
//! Instruction data is a one-byte tag followed by the arguments in the order of the Codama definition
//! (`clients/codama/escrow.mjs`): integers little-endian, byte arrays as they are, a variable path as a
//! `u8` count followed by the 32-byte nodes. The Final Block Tree path may stop short of 20 nodes: the
//! missing upper levels are the empty-subtree constants.
//!
//! | tag | instruction | accounts |
//! |---|---|---|
//! | 0 | CreateEscrow `commitment[32], payout[32], amount u64, bond_required u64, expiry_height u32, claim_mode u8` | payee(ws), escrow(w), payee_token(w), vault(w), relay, token program, system |
//! | 1 | RevealClaim `txid[32], salt[32], block_height u32, tx_index u32, root[32], tree_path, tx_path` | escrow(w), relay, vault(w), payout_token(w), payee(w), token program |
//!
//! Tags 2 (DepositBond), 3 (Refund) and 4 (PrivateClaim) are reserved for tickets 09 and 10.
#![cfg_attr(target_os = "solana", no_std)]

pub mod error;
pub mod processor;
pub mod state;
pub mod zeros;

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod entry {
    pinocchio::program_entrypoint!(crate::processor::process_instruction, 8);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}
