//! Read interface of the Zolar Header Relay for other Solana programs.
//!
//! A consumer stores the address of the Header Relay it trusts (the address commits to the
//! Checkpoint, so it fixes the network, the start and the Finality Depth). Both constructors take the
//! account's owner and key next to its data and check that the account is **owned by the Header Relay
//! program** and **is the relay the consumer trusts**, so the checks cannot be forgotten:
//!
//! - [`RelayView::parse`] is the default: it also requires the relay to be sealed and **running**, and
//!   only a `RelayView` answers root queries (a Halted relay's frozen ring still holds roots of blocks
//!   a deep reorg orphaned).
//! - [`RelayStatus::parse_allow_halted`] is for readers that must see a Halted relay (status displays,
//!   the CLI, tools). It has no root queries; [`RelayStatus::running`] upgrades it to a `RelayView`.
//!
//! The crate has no dependencies and does no I/O, so it works in any on-chain or off-chain Rust.
//!
//! Hashes are in internal byte order (raw SHA-256d; explorers print them reversed). Tree roots are
//! 32-byte big-endian BN254 field elements, the form the `sol_poseidon` syscall (big-endian mode) and
//! `groth16-solana` public inputs use.
//!
//! The Final Block Tree: depth 20, leaf `k` is the block at height `checkpoint_height + 1 + k`,
//! `leaf = Poseidon(height, root_hi, root_lo)` where `root_hi`/`root_lo` are the first/last 16 bytes of
//! the block's transaction Merkle root as 128-bit big-endian numbers, node = `Poseidon(left, right)`.
//! Every value is passed to Poseidon as a 32-byte big-endian field element.
#![no_std]

/// The Header Relay program that owns every Header Relay account Zolar trusts.
pub const HEADER_RELAY_PROGRAM: [u8; 32] = [
    0x4d, 0xe4, 0xdb, 0x60, 0x3b, 0xec, 0xf8, 0x04, 0xa8, 0x2a, 0x8f, 0x22, 0x6a, 0xca, 0xc6, 0x34,
    0x7d, 0xcc, 0x7d, 0xd8, 0x13, 0xab, 0xa2, 0xe4, 0x5d, 0xd5, 0xc2, 0xdc, 0x5b, 0x49, 0xb4, 0xd8,
];

pub const DISC_RELAY: u8 = 1;
pub const VERSION: u8 = 1;

/// Blocks of times and compact targets in a Checkpoint: the 102-block NU7 window plus the 11-block median.
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

pub const R_DISC: usize = 0;
pub const R_VERSION: usize = 1;
pub const R_STATE: usize = 2;
/// 0 running, otherwise the reason the relay Halted (see `HALT_*`).
pub const R_HALTED: usize = 3;
pub const R_CREATOR: usize = 4;
pub const R_TIP_HEIGHT: usize = 36;
pub const R_TIP_SLOT: usize = 40;
pub const R_TIP_SEQ: usize = 42;
pub const R_TIP_WORK: usize = 46;
pub const R_TIP_HASH: usize = 62;
pub const R_CHECKPOINT: usize = 94;
pub const R_FINALIZED: usize = R_CHECKPOINT + CHECKPOINT_LEN;
/// Height of the last block appended to the Final Block Tree (the checkpoint height before the first).
pub const R_FINALIZED_HEIGHT: usize = R_FINALIZED;
pub const R_FINALIZED_HASH: usize = R_FINALIZED + 4;
/// Leaves in the Final Block Tree (`finalized_height - checkpoint height`).
pub const R_LEAF_COUNT: usize = R_FINALIZED + 36;
pub const R_HALT_HEIGHT: usize = R_FINALIZED + 40;
pub const R_HALT_HASH: usize = R_FINALIZED + 44;
pub const RESERVED_LEN: usize = 128;
pub const R_FILLED: usize = R_FINALIZED + RESERVED_LEN;
pub const R_ROOTS: usize = R_FILLED + TREE_DEPTH * 32;
pub const RELAY_LEN: usize = R_ROOTS + ROOT_HISTORY * 32;

pub const STATE_PENDING: u8 = 0;
pub const STATE_ACTIVE: u8 = 1;

pub const HALT_RUNNING: u8 = 0;
/// The heaviest chain reorganised past a block already in the Final Block Tree.
pub const HALT_DEEP_REORG: u8 = 1;
/// The Final Block Tree has no room for another leaf.
pub const HALT_TREE_FULL: u8 = 2;

/// Levels of the Final Block Tree: 2^20 leaves, 303 days at 25 s blocks.
pub const TREE_DEPTH: usize = 20;
pub const TREE_CAPACITY: u64 = 1 << TREE_DEPTH;
/// Roots kept: the root after each of the last `ROOT_HISTORY` appends (the root after `n` appends
/// sits at ring index `n % ROOT_HISTORY`; index 0 starts as the empty-tree root).
pub const ROOT_HISTORY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// The account is not owned by the Header Relay program.
    WrongOwner,
    /// The account is not the relay the consumer trusts.
    WrongAddress,
    /// Wrong length, discriminator or version: not a Header Relay account of this layout.
    NotARelay,
    /// The Checkpoint is not sealed yet, so the address does not yet commit to its contents.
    NotSealed,
    /// The relay is Halted (the reason code is attached).
    Halted(u8),
}

/// Read-only view over a sealed Header Relay account that may be Halted. It answers everything except
/// root queries; see [`RelayView`] for those.
#[derive(Clone, Copy)]
pub struct RelayStatus<'a>(&'a [u8]);

/// Read-only view over a sealed, **running** Header Relay account: the only type that answers root
/// queries. Dereferences to [`RelayStatus`] for the rest.
#[derive(Clone, Copy)]
pub struct RelayView<'a>(RelayStatus<'a>);

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn bytes32(d: &[u8], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&d[at..at + 32]);
    out
}

impl<'a> RelayStatus<'a> {
    /// Checks that the account is owned by the Header Relay program, is `expected_key`, has the relay
    /// layout and is sealed. A Halted relay parses; check [`Self::ensure_running`] before honouring a
    /// Claim, or use [`RelayView::parse`].
    pub fn parse_allow_halted(
        owner: &[u8; 32],
        key: &[u8; 32],
        expected_key: &[u8; 32],
        data: &'a [u8],
    ) -> Result<Self, ViewError> {
        if owner != &HEADER_RELAY_PROGRAM {
            return Err(ViewError::WrongOwner);
        }
        if key != expected_key {
            return Err(ViewError::WrongAddress);
        }
        if data.len() != RELAY_LEN || data[R_DISC] != DISC_RELAY || data[R_VERSION] != VERSION {
            return Err(ViewError::NotARelay);
        }
        if data[R_STATE] != STATE_ACTIVE {
            return Err(ViewError::NotSealed);
        }
        Ok(Self(data))
    }

    /// The view that answers root queries, if the relay is running.
    pub fn running(self) -> Result<RelayView<'a>, ViewError> {
        self.ensure_running()?;
        Ok(RelayView(self))
    }

    /// `Ok` only while the relay accepts headers. Use it before honouring a Claim.
    pub fn ensure_running(&self) -> Result<(), ViewError> {
        match self.halt_reason() {
            HALT_RUNNING => Ok(()),
            reason => Err(ViewError::Halted(reason)),
        }
    }

    pub fn halt_reason(&self) -> u8 {
        self.0[R_HALTED]
    }

    pub fn is_halted(&self) -> bool {
        self.halt_reason() != HALT_RUNNING
    }

    /// Tip height of the heaviest known chain.
    pub fn tip_height(&self) -> u32 {
        u32_at(self.0, R_TIP_HEIGHT)
    }

    pub fn tip_hash(&self) -> [u8; 32] {
        bytes32(self.0, R_TIP_HASH)
    }

    /// Cumulative work of the tip since the checkpoint block.
    pub fn tip_work(&self) -> u128 {
        let mut w = [0u8; 16];
        w.copy_from_slice(&self.0[R_TIP_WORK..R_TIP_WORK + 16]);
        u128::from_le_bytes(w)
    }

    pub fn finality_depth(&self) -> u16 {
        u16::from_le_bytes([
            self.0[R_CHECKPOINT + CP_FINALITY_DEPTH],
            self.0[R_CHECKPOINT + CP_FINALITY_DEPTH + 1],
        ])
    }

    pub fn network(&self) -> u8 {
        self.0[R_CHECKPOINT + CP_NETWORK]
    }

    pub fn checkpoint_height(&self) -> u32 {
        u32_at(self.0, R_CHECKPOINT + CP_HEIGHT)
    }

    /// Height of the newest block in the Final Block Tree.
    pub fn finalized_height(&self) -> u32 {
        u32_at(self.0, R_FINALIZED_HEIGHT)
    }

    pub fn finalized_hash(&self) -> [u8; 32] {
        bytes32(self.0, R_FINALIZED_HASH)
    }

    /// Number of leaves in the Final Block Tree.
    pub fn leaf_count(&self) -> u32 {
        u32_at(self.0, R_LEAF_COUNT)
    }

    /// Tip height at the Halt and hash of the header that caused it (zero while running).
    pub fn halt_height(&self) -> u32 {
        u32_at(self.0, R_HALT_HEIGHT)
    }

    pub fn halt_hash(&self) -> [u8; 32] {
        bytes32(self.0, R_HALT_HASH)
    }
}

impl<'a> RelayView<'a> {
    /// Checks that the account is owned by the Header Relay program, is `expected_key`, has the relay
    /// layout, is sealed and is running (not Halted).
    pub fn parse(
        owner: &[u8; 32],
        key: &[u8; 32],
        expected_key: &[u8; 32],
        data: &'a [u8],
    ) -> Result<Self, ViewError> {
        RelayStatus::parse_allow_halted(owner, key, expected_key, data)?.running()
    }

    /// The newest Final Block Tree root.
    pub fn latest_root(&self) -> [u8; 32] {
        self.root_at(self.leaf_count()).unwrap_or([0; 32])
    }

    /// The root the tree had when it held `size` leaves, if it is still in the history.
    pub fn root_at(&self, size: u32) -> Option<[u8; 32]> {
        let count = self.leaf_count();
        if size > count || count - size >= ROOT_HISTORY as u32 {
            return None;
        }
        Some(bytes32(
            self.0 .0,
            R_ROOTS + (size as usize % ROOT_HISTORY) * 32,
        ))
    }

    /// The tree size at which `root` was the root, if it is among the last `ROOT_HISTORY` roots
    /// (the newest match wins; an all-zero `root` never matches).
    pub fn root_size(&self, root: &[u8; 32]) -> Option<u32> {
        if root.iter().all(|b| *b == 0) {
            return None;
        }
        let count = self.leaf_count();
        let oldest = count.saturating_sub(ROOT_HISTORY as u32 - 1);
        (oldest..=count)
            .rev()
            .find(|size| self.root_at(*size).as_ref() == Some(root))
    }

    pub fn is_known_root(&self, root: &[u8; 32]) -> bool {
        self.root_size(root).is_some()
    }
}

impl<'a> core::ops::Deref for RelayView<'a> {
    type Target = RelayStatus<'a>;
    fn deref(&self) -> &RelayStatus<'a> {
        &self.0
    }
}

/// The three field elements hashed into the leaf of a block: `[height, root_hi, root_lo]`, each a
/// 32-byte big-endian number below 2^128.
pub fn leaf_inputs(height: u32, merkle_root: &[u8; 32]) -> [[u8; 32]; 3] {
    let mut h = [0u8; 32];
    h[28..].copy_from_slice(&height.to_be_bytes());
    let mut hi = [0u8; 32];
    hi[16..].copy_from_slice(&merkle_root[..16]);
    let mut lo = [0u8; 32];
    lo[16..].copy_from_slice(&merkle_root[16..]);
    [h, hi, lo]
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;
    use std::vec;

    const KEY: [u8; 32] = [7; 32];

    fn relay_with(count: u32, root_for: impl Fn(u32) -> u8) -> std::vec::Vec<u8> {
        let mut d = vec![0u8; RELAY_LEN];
        d[R_DISC] = DISC_RELAY;
        d[R_VERSION] = VERSION;
        d[R_STATE] = STATE_ACTIVE;
        d[R_LEAF_COUNT..R_LEAF_COUNT + 4].copy_from_slice(&count.to_le_bytes());
        for size in count.saturating_sub(ROOT_HISTORY as u32 - 1)..=count {
            let at = R_ROOTS + (size as usize % ROOT_HISTORY) * 32;
            d[at..at + 32].fill(root_for(size));
        }
        d
    }

    fn view(d: &[u8]) -> Result<RelayView<'_>, ViewError> {
        RelayView::parse(&HEADER_RELAY_PROGRAM, &KEY, &KEY, d)
    }

    fn status(d: &[u8]) -> Result<RelayStatus<'_>, ViewError> {
        RelayStatus::parse_allow_halted(&HEADER_RELAY_PROGRAM, &KEY, &KEY, d)
    }

    #[test]
    fn rejects_other_accounts() {
        let mut d = relay_with(0, |_| 1);
        assert!(view(&d).is_ok());
        d[R_DISC] = 2;
        assert_eq!(view(&d).err(), Some(ViewError::NotARelay));
        assert_eq!(view(&d[..100]).err(), Some(ViewError::NotARelay));
        let mut d = relay_with(0, |_| 1);
        d[R_STATE] = STATE_PENDING;
        assert_eq!(view(&d).err(), Some(ViewError::NotSealed));
        assert_eq!(status(&d).err(), Some(ViewError::NotSealed));
    }

    #[test]
    fn the_owner_and_the_address_are_pinned_by_both_constructors() {
        let d = relay_with(0, |_| 1);
        let other = [9u8; 32];
        assert_eq!(
            RelayView::parse(&other, &KEY, &KEY, &d).err(),
            Some(ViewError::WrongOwner)
        );
        assert_eq!(
            RelayView::parse(&HEADER_RELAY_PROGRAM, &other, &KEY, &d).err(),
            Some(ViewError::WrongAddress)
        );
        assert_eq!(
            RelayStatus::parse_allow_halted(&other, &KEY, &KEY, &d).err(),
            Some(ViewError::WrongOwner)
        );
        assert_eq!(
            RelayStatus::parse_allow_halted(&HEADER_RELAY_PROGRAM, &other, &KEY, &d).err(),
            Some(ViewError::WrongAddress)
        );
    }

    #[test]
    fn a_halted_relay_is_refused_by_default_and_visible_through_the_status_reader() {
        let mut d = relay_with(5, |size| size as u8 + 1);
        d[R_HALTED] = HALT_DEEP_REORG;
        assert_eq!(view(&d).err(), Some(ViewError::Halted(HALT_DEEP_REORG)));
        let s = status(&d).unwrap();
        assert!(s.is_halted());
        assert_eq!(s.halt_reason(), HALT_DEEP_REORG);
        assert_eq!(
            s.running().err(),
            Some(ViewError::Halted(HALT_DEEP_REORG)),
            "its frozen roots are unreachable"
        );
    }

    #[test]
    fn history_holds_the_last_128_roots() {
        let d = relay_with(300, |size| (size % 251) as u8 + 1);
        let v = view(&d).unwrap();
        assert_eq!(v.latest_root(), [(300 % 251) as u8 + 1; 32]);
        assert_eq!(v.root_at(173), Some([174u8; 32]));
        assert_eq!(v.root_at(300 - 128), None, "aged out of the ring");
        assert_eq!(v.root_at(301), None, "not appended yet");
        assert_eq!(v.root_size(&[((300 - 5) % 251) as u8 + 1; 32]), Some(295));
        assert!(!v.is_known_root(&[0; 32]));
        assert!(!v.is_known_root(&[255; 32]));
    }

    #[test]
    fn a_young_tree_knows_its_empty_root() {
        let d = relay_with(2, |size| size as u8 + 10);
        let v = view(&d).unwrap();
        assert_eq!(v.root_at(0), Some([10; 32]));
        assert_eq!(v.root_size(&[11; 32]), Some(1));
    }

    #[test]
    fn leaf_inputs_are_big_endian_limbs() {
        let mut root = [0u8; 32];
        for (i, b) in root.iter_mut().enumerate() {
            *b = i as u8 + 1;
        }
        let [h, hi, lo] = leaf_inputs(0x0102_0304, &root);
        assert_eq!(&h[28..], &[1, 2, 3, 4]);
        assert!(h[..28].iter().all(|b| *b == 0));
        assert_eq!(&hi[16..], &root[..16]);
        assert_eq!(&lo[16..], &root[16..]);
        assert!(hi[..16].iter().all(|b| *b == 0) && lo[..16].iter().all(|b| *b == 0));
    }
}
