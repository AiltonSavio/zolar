//! Which blocks a new tip makes final, and whether it reorganises past one that already is.
//!
//! A block is final once Finality Depth `D` blocks follow it on the tip's chain: a tip at height `T`
//! finalises every block up to `T - D`. The relay keeps a cursor `F`, the height of the last block
//! appended to the Final Block Tree, and moves it on every tip change, so the cursor block is always
//! on the tip's chain unless the relay Halted and, after every accepted header, `F` is the tip height
//! minus `D` (or higher, when a heavier chain is shorter). A header that would append more than
//! `MAX_APPENDS` blocks is refused whole, so the cursor never lags.

use crate::state::CHECKPOINT_SLOT;

/// Most leaves one header may append. Each costs about 21k compute units.
pub const MAX_APPENDS: usize = 40;

/// A stored block as the walk sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub height: u32,
    pub hash: [u8; 32],
    pub parent_slot: u16,
    pub parent_seq: u32,
}

/// The finalised cursor and the Checkpoint block, the two fixed points a walk can end at.
#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    pub checkpoint_height: u32,
    pub checkpoint_hash: [u8; 32],
    pub height: u32,
    pub hash: [u8; 32],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The new tip's chain contains the cursor block. Append the blocks in `slots[..count]`, oldest
    /// first (`slots[0]` is the block at height `F + 1`).
    Extend {
        slots: [u16; MAX_APPENDS],
        count: usize,
    },
    /// The new tip's chain lacks the cursor block.
    Reorg,
    /// An ancestor was evicted, so the question cannot be answered. Nothing may change.
    Unreadable,
    /// The new tip's chain contains the cursor block but would finalise more than `MAX_APPENDS` blocks.
    TooManyAppends,
}

/// Plans the effect of a header at `height` whose parent is at `(parent_slot, parent_seq)` becoming the
/// tip. `read(slot, seq)` returns the stored block only while the slot still holds sequence `seq`.
pub fn plan(
    cursor: &Cursor,
    depth: u16,
    height: u32,
    parent_slot: u16,
    parent_seq: u32,
    read: impl Fn(u16, u32) -> Option<Block>,
) -> Outcome {
    if height <= cursor.height {
        return Outcome::Reorg;
    }
    let finalised_top = height.saturating_sub(depth as u32);
    let count = finalised_top.saturating_sub(cursor.height) as usize;
    let mut slots = [0u16; MAX_APPENDS];
    let (mut slot, mut seq) = (parent_slot, parent_seq);
    loop {
        let (block_height, hash, next) = if slot == CHECKPOINT_SLOT {
            (cursor.checkpoint_height, cursor.checkpoint_hash, None)
        } else {
            match read(slot, seq) {
                Some(b) => (b.height, b.hash, Some((b.parent_slot, b.parent_seq))),
                None => return Outcome::Unreadable,
            }
        };
        if block_height == cursor.height {
            if hash != cursor.hash {
                return Outcome::Reorg;
            }
            if count > MAX_APPENDS {
                return Outcome::TooManyAppends;
            }
            return Outcome::Extend { slots, count };
        }
        if block_height < cursor.height {
            return Outcome::Unreadable;
        }
        if block_height <= finalised_top {
            // Blocks F+1..=top are met top-down; `slots` keeps the first `MAX_APPENDS` of them.
            if let Some(entry) = slots.get_mut((block_height - cursor.height - 1) as usize) {
                *entry = slot;
            }
        }
        match next {
            Some((s, q)) => (slot, seq) = (s, q),
            None => return Outcome::Unreadable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CP: u32 = 100;

    fn hash_of(height: u32, branch: u8) -> [u8; 32] {
        let mut h = [branch; 32];
        h[..4].copy_from_slice(&height.to_le_bytes());
        h
    }

    /// A chain on branch 0 of blocks CP+1..=top, block `h` in slot `h - CP - 1`, seq = slot + 1.
    fn chain(top: u32, branch: u8) -> impl Fn(u16, u32) -> Option<Block> {
        move |slot, seq| {
            let height = CP + 1 + slot as u32;
            (height <= top && seq == slot as u32 + 1).then(|| Block {
                height,
                hash: hash_of(height, branch),
                parent_slot: if slot == 0 { CHECKPOINT_SLOT } else { slot - 1 },
                parent_seq: slot as u32,
            })
        }
    }

    fn cursor_at(height: u32) -> Cursor {
        Cursor {
            checkpoint_height: CP,
            checkpoint_hash: hash_of(CP, 0),
            height,
            hash: hash_of(height, 0),
        }
    }

    fn parent_of(height: u32) -> (u16, u32) {
        let slot = height - CP - 1;
        (slot as u16, slot + 1)
    }

    #[test]
    fn nothing_is_final_until_depth_blocks_follow() {
        let (s, q) = parent_of(102);
        let out = plan(&cursor_at(CP), 3, 103, s, q, chain(102, 0));
        assert!(matches!(out, Outcome::Extend { count: 0, .. }));
    }

    #[test]
    fn the_block_depth_below_the_tip_becomes_final() {
        let (s, q) = parent_of(103);
        let Outcome::Extend { slots, count } = plan(&cursor_at(CP), 3, 104, s, q, chain(103, 0))
        else {
            panic!("expected extend")
        };
        assert_eq!((count, slots[0]), (1, 0), "block 101 is in slot 0");
    }

    #[test]
    fn a_taller_tip_appends_every_block_in_height_order() {
        let (s, q) = parent_of(110);
        let Outcome::Extend { slots, count } = plan(&cursor_at(103), 3, 111, s, q, chain(110, 0))
        else {
            panic!("expected extend")
        };
        assert_eq!(count, 5, "blocks 104..=108");
        assert_eq!(&slots[..5], &[3, 4, 5, 6, 7]);
    }

    #[test]
    fn a_shorter_but_heavier_tip_that_keeps_the_cursor_appends_nothing() {
        let (s, q) = parent_of(108);
        let out = plan(&cursor_at(107), 3, 109, s, q, chain(108, 0));
        assert!(matches!(out, Outcome::Extend { count: 0, .. }));
    }

    #[test]
    fn a_chain_without_the_cursor_block_is_a_reorg() {
        let (s, q) = parent_of(110);
        let out = plan(&cursor_at(105), 3, 111, s, q, chain(110, 1));
        assert_eq!(out, Outcome::Reorg);
    }

    #[test]
    fn a_tip_at_or_below_the_cursor_is_a_reorg() {
        let (s, q) = parent_of(104);
        assert_eq!(
            plan(&cursor_at(105), 3, 105, s, q, chain(104, 0)),
            Outcome::Reorg
        );
        let (s, q) = parent_of(103);
        assert_eq!(
            plan(&cursor_at(105), 3, 104, s, q, chain(103, 0)),
            Outcome::Reorg
        );
    }

    #[test]
    fn a_fork_above_the_cursor_is_not_a_reorg() {
        // Branch 1 shares blocks up to 106 with branch 0, cursor at 105.
        let read = |slot: u16, seq: u32| {
            let height = CP + 1 + slot as u32;
            let branch = if height <= 106 { 0 } else { 1 };
            (seq == slot as u32 + 1 && height <= 109).then(|| Block {
                height,
                hash: hash_of(height, branch),
                parent_slot: if slot == 0 { CHECKPOINT_SLOT } else { slot - 1 },
                parent_seq: slot as u32,
            })
        };
        let (s, q) = parent_of(109);
        let out = plan(&cursor_at(105), 3, 110, s, q, read);
        assert!(matches!(out, Outcome::Extend { count: 2, .. }), "{out:?}");
    }

    #[test]
    fn an_evicted_ancestor_makes_the_question_unanswerable() {
        let read = |slot: u16, seq: u32| (slot != 4).then(|| chain(110, 0)(slot, seq)).flatten();
        let (s, q) = parent_of(110);
        assert_eq!(
            plan(&cursor_at(103), 3, 111, s, q, read),
            Outcome::Unreadable
        );
    }

    #[test]
    fn a_pointer_to_a_reused_slot_is_unreadable() {
        let (s, q) = parent_of(110);
        let out = plan(&cursor_at(103), 3, 111, s, q + 9, chain(110, 0));
        assert_eq!(out, Outcome::Unreadable);
    }

    #[test]
    fn at_most_forty_blocks_become_final_at_once() {
        let (s, q) = parent_of(140);
        let ok = plan(&cursor_at(CP), 1, 141, s, q, chain(140, 0));
        let Outcome::Extend { slots, count } = ok else {
            panic!("expected extend")
        };
        assert_eq!((count, slots[0], slots[39]), (40, 0, 39));
        let (s, q) = parent_of(141);
        let too = plan(&cursor_at(CP), 1, 142, s, q, chain(141, 0));
        assert_eq!(too, Outcome::TooManyAppends);
    }

    #[test]
    fn a_conflicting_chain_is_a_reorg_even_when_it_is_far_ahead() {
        let (s, q) = parent_of(180);
        assert_eq!(
            plan(&cursor_at(105), 1, 181, s, q, chain(180, 1)),
            Outcome::Reorg,
            "the cursor check comes before the append limit"
        );
    }
}
