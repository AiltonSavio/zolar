//! Account layouts, packed and little-endian so a borsh decoder reads them as is.
//!
//! All hashes are in internal byte order (raw SHA-256d output; explorers print
//! them reversed). Every account starts with a discriminator and a version byte.
//!
//! - **Relay** (PDA `["relay", checkpoint_digest]`): the fixed Checkpoint, the tip
//!   and the Halt flag. `reserved` is zeroed space for later tickets' fields.
//! - **Store** (PDA `["store", relay]`): a 40-byte header, then `capacity` header
//!   slots written in insertion order and overwritten oldest first once full.
//! - **Buffer** (PDA `["buffer", relay, authority, nonce]`): one header being
//!   uploaded and verified by its authority.

pub const DISC_RELAY: u8 = 1;
pub const DISC_STORE: u8 = 2;
pub const DISC_BUFFER: u8 = 3;
pub const VERSION: u8 = 1;

pub const SEED_RELAY: &[u8] = b"relay";
pub const SEED_STORE: &[u8] = b"store";
pub const SEED_BUFFER: &[u8] = b"buffer";

// Checkpoint: the immutable starting point. Its SHA-256 is the Relay's address seed.
/// Times and compact targets kept for the newest 113 checkpoint blocks: the 102-block
/// NU7 averaging window plus the 11-block median.
pub const CONTEXT_LEN: usize = 113;
pub const CP_VERSION: usize = 0;
pub const CP_NETWORK: usize = 1;
pub const CP_FINALITY_DEPTH: usize = 2;
pub const CP_NU7_HEIGHT: usize = 4;
pub const CP_HEIGHT: usize = 8;
pub const CP_NONCE: usize = 12;
pub const CP_HASH: usize = 20;
pub const CP_TIMES: usize = 52;
pub const CP_BITS: usize = CP_TIMES + CONTEXT_LEN * 4;
pub const CHECKPOINT_LEN: usize = CP_BITS + CONTEXT_LEN * 4;

pub const NETWORK_MAINNET: u8 = 0;
pub const NETWORK_TESTNET: u8 = 1;

// Relay
pub const R_DISC: usize = 0;
pub const R_VERSION: usize = 1;
pub const R_STATE: usize = 2;
pub const R_HALTED: usize = 3;
pub const R_CREATOR: usize = 4;
pub const R_TIP_HEIGHT: usize = 36;
pub const R_TIP_SLOT: usize = 40;
pub const R_TIP_SEQ: usize = 42;
pub const R_TIP_WORK: usize = 46;
pub const R_TIP_HASH: usize = 62;
pub const R_CHECKPOINT: usize = 94;
pub const R_RESERVED: usize = R_CHECKPOINT + CHECKPOINT_LEN;
pub const RESERVED_LEN: usize = 128;
pub const RELAY_LEN: usize = R_RESERVED + RESERVED_LEN;

pub const STATE_PENDING: u8 = 0;
pub const STATE_ACTIVE: u8 = 1;

// Store
pub const S_DISC: usize = 0;
pub const S_VERSION: usize = 1;
pub const S_RELAY: usize = 2;
pub const S_CAPACITY: usize = 34;
pub const S_COUNT: usize = 36;
pub const STORE_HEADER_LEN: usize = 40;
pub const INITIAL_STORE_LEN: usize = 10_240;
pub const GROW_BYTES: usize = 10_240;
pub const MAX_CAPACITY: usize = 400;

// Header slot
pub const SL_HASH: usize = 0;
pub const SL_MERKLE_ROOT: usize = 32;
pub const SL_WORK: usize = 64;
pub const SL_HEIGHT: usize = 80;
pub const SL_TIME: usize = 84;
pub const SL_BITS: usize = 88;
pub const SL_SEQ: usize = 92;
pub const SL_PARENT_SLOT: usize = 96;
pub const SL_PARENT_SEQ: usize = 98;
pub const SLOT_LEN: usize = 102;
/// Parent slot value meaning "the Checkpoint block".
pub const CHECKPOINT_SLOT: u16 = u16::MAX;

// Buffer
pub const B_DISC: usize = 0;
pub const B_VERSION: usize = 1;
pub const B_ACCEPTED: usize = 2;
pub const B_RELAY: usize = 3;
pub const B_AUTHORITY: usize = 35;
pub const B_HEADER: usize = 67;
pub const B_VERIFIER: usize = B_HEADER + zcash_header::equihash::HEADER_LEN;
pub const BUFFER_LEN: usize = B_VERIFIER + zcash_header::equihash::STATE_LEN;

/// Cumulative work and height are measured from the Checkpoint block, which has work 0.
pub const fn initial_capacity() -> usize {
    (INITIAL_STORE_LEN - STORE_HEADER_LEN) / SLOT_LEN
}
