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
//! | 2 | DepositBond | payer(s), escrow(w), payer_token(w), vault(w), token program |
//! | 3 | Refund | escrow(w), relay, vault(w), payee_token(w), payee(w), token program |
//! | 5 | RequestRefund | escrow(w), relay |
//!
//! | 4 | PrivateClaim `proof_a[64] (negated), proof_b[128], proof_c[64], root[32]` | escrow(w), relay, vault(w), payout_token(w), payee(w), token program |
#![cfg_attr(target_os = "solana", no_std)]

pub mod error;
pub mod processor;
pub mod state;
pub mod verifying_key;
mod verifying_key_generated;
pub mod zeros;

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod entry {
    pinocchio::program_entrypoint!(crate::processor::process_instruction, 8);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}
