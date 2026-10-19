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

/// Headers further than this below the tip are refused. A genuine competing branch that
/// forks deeper is beyond what the relay can follow; with `MAX_FINALITY_DEPTH` half of
/// this, any fork deeper than Finality Depth is still seen while it is shallow enough to
/// be stored. It also stops replays of old, genuine headers from refilling the Store.
pub const REORG_WINDOW: u32 = 60;
/// Largest Finality Depth a Checkpoint may fix.
pub const MAX_FINALITY_DEPTH: u16 = (REORG_WINDOW / 2) as u16;
/// Headers are accepted only once the Store holds the 113 blocks of difficulty context
/// plus `REORG_WINDOW` and some margin. A new Store holds 100, so `GrowStore` comes first.
pub const MIN_CAPACITY: usize = 200;

/// Whether a header at `height` is too far below the tip to keep.
pub fn is_too_old(tip_height: u32, height: u32) -> bool {
    tip_height.saturating_sub(height) > REORG_WINDOW
}

/// Marks the slots holding the last `CONTEXT_LEN` blocks of the tip's chain (the tip and its
/// ancestors, following parent pointers while each slot still holds the block pointed to).
/// `link(slot)` returns that slot's `(seq, parent_slot, parent_seq)`. These blocks are the
/// difficulty context of the next header on the tip, so a flood of siblings must not evict them.
pub fn mark_tip_chain(
    tip_slot: u16,
    tip_seq: u32,
    link: impl Fn(usize) -> (u32, u16, u32),
    marked: &mut [bool],
) {
    let (mut slot, mut seq) = (tip_slot, tip_seq);
    for _ in 0..zcash_header::difficulty::CONTEXT_LEN {
        let index = slot as usize;
        if slot == CHECKPOINT_SLOT || index >= marked.len() {
            return;
        }
        let (stored_seq, parent_slot, parent_seq) = link(index);
        if stored_seq != seq {
            return;
        }
        marked[index] = true;
        (slot, seq) = (parent_slot, parent_seq);
    }
}

/// Picks the slot a new header is written to: the first never-used slot (sequence 0),
/// otherwise the oldest slot that is not `protected` (the tip's chain and the new header's parent).
pub fn choose_slot(
    capacity: usize,
    seq_of: impl Fn(usize) -> u32,
    protected: impl Fn(usize) -> bool,
) -> Option<usize> {
    let mut oldest: Option<(u32, usize)> = None;
    for i in 0..capacity {
        let seq = seq_of(i);
        if seq == 0 {
            return Some(i);
        }
        if protected(i) {
            continue;
        }
        if oldest.is_none_or(|(s, _)| seq < s) {
            oldest = Some((seq, i));
        }
    }
    oldest.map(|(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_beyond_the_reorg_window_are_too_old() {
        assert!(!is_too_old(1_000, 1_000 - REORG_WINDOW));
        assert!(is_too_old(1_000, 1_000 - REORG_WINDOW - 1));
        assert!(
            !is_too_old(5, 1_000),
            "a header above the tip is never too old"
        );
    }

    #[test]
    fn uses_free_slots_before_evicting() {
        let seqs = [3, 4, 0, 0];
        assert_eq!(choose_slot(4, |i| seqs[i], |_| false), Some(2));
    }

    #[test]
    fn evicts_the_oldest_unprotected_slot() {
        let seqs = [7, 5, 9, 6];
        assert_eq!(choose_slot(4, |i| seqs[i], |_| false), Some(1));
        assert_eq!(choose_slot(4, |i| seqs[i], |i| i == 1), Some(3));
        assert_eq!(choose_slot(4, |i| seqs[i], |i| i == 1 || i == 3), Some(0));
    }

    #[test]
    fn never_evicts_the_tip_or_the_parent_so_insertion_cannot_deadlock() {
        // The tip sits in the oldest slot (as after a heavier branch): the next oldest goes.
        let seqs = [1, 2, 3];
        assert_eq!(choose_slot(3, |i| seqs[i], |i| i < 2), Some(2));
        assert_eq!(choose_slot(2, |i| seqs[i], |_| true), None);
    }

    /// A chain of three blocks (slots 2, 0, 1 in insertion order) behind a tip, plus a sibling in slot 3.
    #[test]
    fn the_tip_chain_survives_a_flood_of_siblings() {
        // slot: (seq, parent_slot, parent_seq)
        let slots = [
            (2, 2, 1),               // slot 0: second block, child of slot 2
            (3, 0, 2),               // slot 1: the tip, child of slot 0
            (1, CHECKPOINT_SLOT, 0), // slot 2: first block, child of the checkpoint
            (9, 2, 1),               // slot 3: a sibling of the second block
        ];
        let mut marked = [false; 4];
        mark_tip_chain(1, 3, |i| slots[i], &mut marked);
        assert_eq!(marked, [true, true, true, false]);
        let seqs = [2, 3, 1, 9];
        assert_eq!(
            choose_slot(4, |i| seqs[i], |i| marked[i]),
            Some(3),
            "only the sibling is evictable"
        );
    }

    #[test]
    fn a_stale_pointer_ends_the_marked_chain() {
        // slot 2 was overwritten by seq 7 after slot 0 pointed at seq 1.
        let slots = [(2, 2, 1), (3, 0, 2), (7, CHECKPOINT_SLOT, 0)];
        let mut marked = [false; 3];
        mark_tip_chain(1, 3, |i| slots[i], &mut marked);
        assert_eq!(marked, [true, true, false]);
    }

    #[test]
    fn marking_stops_after_the_difficulty_context() {
        // 120 blocks in a line: slot i holds seq i + 1, child of slot i - 1.
        let link = |i: usize| (i as u32 + 1, i.wrapping_sub(1) as u16, i as u32);
        let mut marked = [false; 120];
        mark_tip_chain(119, 120, link, &mut marked);
        assert_eq!(marked.iter().filter(|m| **m).count(), 113);
        assert!(marked[119] && marked[7] && !marked[6]);
    }
}
