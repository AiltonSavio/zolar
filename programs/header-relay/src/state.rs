//! Account layouts, packed and little-endian so a borsh decoder reads them as is.
//!
//! All hashes are in internal byte order (raw SHA-256d output; explorers print
//! them reversed). Every account starts with a discriminator and a version byte.
//!
//! - **Relay** (PDA `["relay", creator, checkpoint_digest]`): the fixed Checkpoint, the tip, the
//!   Halt reason, the Final Block Tree cursor, its filled subtrees and its root history. The layout
//!   lives in the `header-relay-interface` crate, which other programs read it with.
//! - **Store** (PDA `["store", relay]`): a 40-byte header, then `capacity` header
//!   slots written in insertion order and overwritten oldest first once full.
//! - **Buffer** (PDA `["buffer", relay, authority, nonce]`): one header being
//!   uploaded and verified by its authority.

pub use header_relay_interface::*;

pub const DISC_STORE: u8 = 2;
pub const DISC_BUFFER: u8 = 3;

pub const SEED_RELAY: &[u8] = b"relay";
pub const SEED_STORE: &[u8] = b"store";
pub const SEED_BUFFER: &[u8] = b"buffer";

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
/// Headers are accepted only once the Store has its full 400 slots: the protected tip chain
/// (`protected_tip_depth`, at most 204), a competing branch's context (113) and room to evict. A new
/// Store holds 100, so `GrowStore` runs three times first.
pub const MIN_CAPACITY: usize = MAX_CAPACITY;

// Worst case, a header that is not the tip: the tip chain, a fork's context blocks, headroom.
const _: () = assert!(
    MIN_CAPACITY
        >= protected_tip_depth(MAX_FINALITY_DEPTH) + zcash_header::difficulty::CONTEXT_LEN + 80
);
// A header that becomes the tip protects its own chain only.
const _: () = assert!(MIN_CAPACITY > protected_tip_depth(MAX_FINALITY_DEPTH) + 100);

/// Whether a header at `height` is too far below the tip to keep.
pub fn is_too_old(tip_height: u32, height: u32) -> bool {
    tip_height.saturating_sub(height) > REORG_WINDOW
}

/// Blocks of the tip's chain the Store never evicts. A competing branch may fork `REORG_WINDOW + 1`
/// blocks below the tip; its next header needs `CONTEXT_LEN` blocks of difficulty context below that
/// fork point, all on the tip's chain. A heavier chain that keeps the finalised block can also lower the
/// tip by up to `finality_depth - 1` blocks, so the protected span starts that much lower.
pub const fn protected_tip_depth(finality_depth: u16) -> usize {
    REORG_WINDOW as usize + 1 + zcash_header::difficulty::CONTEXT_LEN + finality_depth as usize
}

/// Marks the slots holding the last `depth` blocks of the chain that ends at `(slot, seq)` (following
/// parent pointers while each slot still holds the block pointed to). `link(slot)` returns that
/// slot's `(seq, parent_slot, parent_seq)`.
pub fn mark_chain(
    start_slot: u16,
    start_seq: u32,
    depth: usize,
    link: impl Fn(usize) -> (u32, u16, u32),
    marked: &mut [bool],
) {
    let (mut slot, mut seq) = (start_slot, start_seq);
    for _ in 0..depth {
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

/// How hard a block was to mine: `floor(log2(target))` of its `nBits`, so a bigger class is an
/// easier block and one class is a factor of two. An invalid encoding counts as the easiest.
pub fn difficulty_class(bits: u32) -> u16 {
    let size = (bits >> 24) as i32;
    let mantissa = bits & 0x007f_ffff;
    if mantissa == 0 || bits & 0x0080_0000 != 0 {
        return u16::MAX;
    }
    (8 * (size - 3) + 31 - mantissa.leading_zeros() as i32).max(0) as u16
}

/// What the Store knows about one slot, for choosing which to overwrite.
#[derive(Clone, Copy, Debug)]
pub struct SlotInfo {
    /// 0 for a never-used slot.
    pub seq: u32,
    /// Too far below the tip to get another child.
    pub stale: bool,
    pub class: u16,
}

/// The slot a new header goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Victim {
    pub index: usize,
    pub free: bool,
    pub stale: bool,
    pub class: u16,
}

/// Picks the slot a new header is written to: a never-used slot; otherwise, among slots that are not
/// `protected`, a stale one first, then the easiest block, oldest first among equals.
pub fn choose_slot(
    capacity: usize,
    info: impl Fn(usize) -> SlotInfo,
    protected: impl Fn(usize) -> bool,
) -> Option<Victim> {
    let mut best: Option<(SlotInfo, usize)> = None;
    for i in 0..capacity {
        let slot = info(i);
        if slot.seq == 0 {
            return Some(Victim {
                index: i,
                free: true,
                stale: false,
                class: 0,
            });
        }
        if protected(i) {
            continue;
        }
        let better = best.is_none_or(|(b, _)| {
            (slot.stale, slot.class, core::cmp::Reverse(slot.seq))
                > (b.stale, b.class, core::cmp::Reverse(b.seq))
        });
        if better {
            best = Some((slot, i));
        }
    }
    best.map(|(s, index)| Victim {
        index,
        free: false,
        stale: s.stale,
        class: s.class,
    })
}

/// Whether a header of difficulty `class` may take the victim's slot. A header that becomes the new
/// tip always may; any other must not be easier than a live block it would push out, so cheap
/// headers cannot dislodge harder ones.
pub fn admits(victim: &Victim, class: u16, becomes_tip: bool) -> bool {
    becomes_tip || victim.free || victim.stale || class <= victim.class
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(seq: u32, stale: bool, class: u16) -> SlotInfo {
        SlotInfo { seq, stale, class }
    }

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
    fn the_protected_tip_chain_spans_204_blocks_at_the_finality_cap() {
        assert_eq!(protected_tip_depth(30), 204);
    }

    #[test]
    fn difficulty_classes_are_log2_of_the_target() {
        // 0x2007ffff: 0x07ffff << 8*(0x20-3) = 2^19 * 2^232 -> just under 2^251.
        assert_eq!(difficulty_class(0x2007ffff), 250);
        assert_eq!(difficulty_class(0x1f07ffff), 242);
        assert_eq!(difficulty_class(0x1d00ffff), 8 * 26 + 15);
        assert_eq!(difficulty_class(0x03000001), 0);
        assert_eq!(difficulty_class(0x04800000), u16::MAX, "negative encoding");
        assert!(difficulty_class(0x2007ffff) > difficulty_class(0x1f07ffff));
    }

    #[test]
    fn uses_free_slots_before_evicting() {
        let seqs = [3, 4, 0, 0];
        let v = choose_slot(4, |i| info(seqs[i], false, 5), |_| false).unwrap();
        assert_eq!((v.index, v.free), (2, true));
    }

    #[test]
    fn evicts_the_oldest_unprotected_slot_among_equals() {
        let seqs = [7, 5, 9, 6];
        let pick = |p: &dyn Fn(usize) -> bool| {
            choose_slot(4, |i| info(seqs[i], false, 5), p)
                .unwrap()
                .index
        };
        assert_eq!(pick(&|_| false), 1);
        assert_eq!(pick(&|i| i == 1), 3);
        assert_eq!(pick(&|i| i == 1 || i == 3), 0);
    }

    #[test]
    fn stale_slots_go_first_then_the_easiest_block() {
        // slot: (seq, stale, class)
        let slots = [
            info(1, false, 240),
            info(9, false, 250),
            info(8, true, 230),
            info(2, false, 250),
        ];
        let v = choose_slot(4, |i| slots[i], |_| false).unwrap();
        assert_eq!(v.index, 2, "stale beats any live block");
        let v = choose_slot(4, |i| slots[i], |i| i == 2).unwrap();
        assert_eq!(v.index, 3, "easiest class, older of the two");
        let v = choose_slot(4, |i| slots[i], |i| i == 2 || i == 3).unwrap();
        assert_eq!(v.index, 1);
    }

    #[test]
    fn never_evicts_a_protected_slot_so_insertion_cannot_deadlock() {
        let seqs = [1, 2, 3];
        let v = choose_slot(3, |i| info(seqs[i], false, 5), |i| i < 2).unwrap();
        assert_eq!(v.index, 2);
        assert_eq!(choose_slot(2, |i| info(seqs[i], false, 5), |_| true), None);
    }

    #[test]
    fn a_cheap_header_cannot_push_out_a_harder_live_block() {
        let victim = Victim {
            index: 0,
            free: false,
            stale: false,
            class: 242,
        };
        assert!(!admits(&victim, 250, false), "easier than the victim");
        assert!(admits(&victim, 242, false), "as hard");
        assert!(admits(&victim, 240, false), "harder");
        assert!(admits(&victim, 250, true), "a new tip is always admitted");
        let stale = Victim {
            stale: true,
            ..victim
        };
        assert!(admits(&stale, 250, false));
        let free = Victim {
            free: true,
            ..victim
        };
        assert!(admits(&free, 250, false));
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
        mark_chain(1, 3, protected_tip_depth(30), |i| slots[i], &mut marked);
        assert_eq!(marked, [true, true, true, false]);
        let seqs = [2, 3, 1, 9];
        let v = choose_slot(4, |i| info(seqs[i], false, 5), |i| marked[i]).unwrap();
        assert_eq!(v.index, 3, "only the sibling is evictable");
    }

    #[test]
    fn a_stale_pointer_ends_the_marked_chain() {
        // slot 2 was overwritten by seq 7 after slot 0 pointed at seq 1.
        let slots = [(2, 2, 1), (3, 0, 2), (7, CHECKPOINT_SLOT, 0)];
        let mut marked = [false; 3];
        mark_chain(1, 3, protected_tip_depth(30), |i| slots[i], &mut marked);
        assert_eq!(marked, [true, true, false]);
    }

    #[test]
    fn marking_stops_after_the_requested_depth() {
        // 250 blocks in a line: slot i holds seq i + 1, child of slot i - 1.
        let link = |i: usize| (i as u32 + 1, i.wrapping_sub(1) as u16, i as u32);
        let mut marked = [false; 250];
        mark_chain(249, 250, protected_tip_depth(30), link, &mut marked);
        assert_eq!(
            marked.iter().filter(|m| **m).count(),
            protected_tip_depth(30)
        );
        assert!(marked[249] && marked[46] && !marked[45]);
    }
}
