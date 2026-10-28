//! The Private Claim verifying key, generated from the circuit's setup by
//! `cargo run -p zolar-claim --example gen_vk` (see `circuits/README.md`); never edit by hand.

pub use crate::verifying_key_generated::VERIFYINGKEY;

/// Public inputs of the circuit: root, commitment, lowest height, expiry height.
pub const PUBLIC_INPUTS: usize = 4;

const _: () = assert!(VERIFYINGKEY.nr_pubinputs == PUBLIC_INPUTS);
