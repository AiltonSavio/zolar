//! Header Relay program: a Zcash light client on Solana.
//!
//! Instruction data is a one-byte tag followed by borsh-encoded arguments (`u16`/`u32`
//! little-endian, `Vec<u8>` as `u32` length plus bytes). The interface is described for
//! Codama in `clients/codama/program.mjs`; account layouts are in [`state`].
//!
//! | tag | instruction | accounts |
//! |---|---|---|
//! | 0 | CreateRelay `digest[32]` | payer(ws), relay(w), store(w), system |
//! | 1 | WriteCheckpoint `offset u16, bytes` | creator(s), relay(w) |
//! | 2 | SealRelay | relay(w) |
//! | 3 | GrowStore | payer(ws), store(w), system |
//! | 4 | CreateBuffer `nonce u8` | authority(ws), relay, buffer(w), system |
//! | 5 | WriteBuffer `offset u16, bytes` | authority(s), buffer(w) |
//! | 6 | BeginVerify | authority(s), buffer(w) |
//! | 7 | StepVerify `leaves u16` | authority(s), buffer(w) |
//! | 8 | AcceptHeader | authority(s), buffer(w), relay(w), store(w) |
//! | 9 | ResetBuffer | authority(s), buffer(w) |
//! | 10 | CloseBuffer | authority(ws), buffer(w) |
#![no_std]

pub mod error;
pub mod state;

#[cfg(target_os = "solana")]
mod processor;

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod entry {
    pinocchio::program_entrypoint!(crate::processor::process_instruction, 4);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}
