//! Escrow account layout: 300 bytes, fixed-width fields, integers little-endian, no padding.

pub const DISC_ESCROW: u8 = 1;
pub const VERSION: u8 = 1;
pub const STATE_OPEN: u8 = 0;
/// A refund was requested; a Claim still settles until the Refund closes the Escrow.
pub const STATE_REFUND_REQUESTED: u8 = 1;

/// Reveal Claim allowed.
pub const MODE_REVEAL_ALLOWED: u8 = 0;
/// Only a Private Claim may settle (ticket 10).
pub const MODE_PRIVATE_ONLY: u8 = 1;

pub const E_DISC: usize = 0;
pub const E_VERSION: usize = 1;
pub const E_STATE: usize = 2;
pub const E_BUMP: usize = 3;
pub const E_PAYEE: usize = 4;
pub const E_PAYOUT: usize = 36;
pub const E_MINT: usize = 68;
pub const E_VAULT: usize = 100;
pub const E_RELAY: usize = 132;
pub const E_COMMITMENT: usize = 164;
pub const E_AMOUNT: usize = 196;
pub const E_BOND_REQUIRED: usize = 204;
pub const E_BOND_DEPOSITED: usize = 212;
pub const E_CREATED_AT: usize = 220;
pub const E_CREATED_TIP: usize = 228;
pub const E_EXPIRY_HEIGHT: usize = 232;
pub const E_CLAIM_MODE: usize = 236;
pub const E_REFUND_REQUESTED_AT: usize = 237;
pub const E_RESERVED: usize = 245;
pub const ESCROW_LEN: usize = 300;

pub const SEED_ESCROW: &[u8] = b"escrow";

/// The Header Relay program the Escrow trusts (devnet and the program id of every Zolar deployment).
pub const HEADER_RELAY_PROGRAM: [u8; 32] = [
    0x4d, 0xe4, 0xdb, 0x60, 0x3b, 0xec, 0xf8, 0x04, 0xa8, 0x2a, 0x8f, 0x22, 0x6a, 0xca, 0xc6, 0x34,
    0x7d, 0xcc, 0x7d, 0xd8, 0x13, 0xab, 0xa2, 0xe4, 0x5d, 0xd5, 0xc2, 0xdc, 0x5b, 0x49, 0xb4, 0xd8,
];

/// The classic SPL Token program.
pub const TOKEN_PROGRAM: [u8; 32] = [
    0x06, 0xdd, 0xf6, 0xe1, 0xd7, 0x65, 0xa1, 0x93, 0xd9, 0xcb, 0xe1, 0x46, 0xce, 0xeb, 0x79, 0xac,
    0x1c, 0xb4, 0x85, 0xed, 0x5f, 0x5b, 0x37, 0x91, 0x3a, 0x8c, 0xf5, 0x85, 0x7e, 0xff, 0x00, 0xa9,
];

/// The BN254 scalar field modulus, big-endian.
pub const BN254_MODULUS: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// Most blocks ahead of the relay tip an expiry may lie.
pub const MAX_EXPIRY_AHEAD: u32 = 20;
/// Deepest transaction Merkle tree a Reveal Claim accepts (2^24 transactions).
pub const MAX_TX_PATH: usize = 24;
/// Seconds after creation when a refund no longer depends on the relay.
pub const BACKSTOP_SECS: i64 = 24 * 60 * 60;
/// Seconds a Payer has to claim between RequestRefund and Refund.
pub const CLAIM_WINDOW_SECS: i64 = 15 * 60;

/// Lowest block height a Claim may prove: blocks above the final cursor can be replaced, so a payment
/// broadcast after the Escrow was created can land `finality_depth - 1` blocks below the tip it saw.
pub fn block_in_window(
    block_height: u32,
    created_tip: u32,
    expiry_height: u32,
    finality_depth: u16,
) -> bool {
    block_height as u64 + finality_depth as u64 > created_tip as u64
        && block_height <= expiry_height
}

/// `value` is a canonical BN254 field element (big-endian, below the modulus).
pub fn below_modulus(value: &[u8; 32]) -> bool {
    value < &BN254_MODULUS
}

/// Index of a block's leaf in the Final Block Tree, if the block is above the Checkpoint.
pub fn leaf_index(block_height: u32, checkpoint_height: u32) -> Option<u32> {
    block_height.checked_sub(checkpoint_height.checked_add(1)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_elements_are_below_the_modulus() {
        assert!(below_modulus(&[0; 32]));
        let mut m = BN254_MODULUS;
        assert!(!below_modulus(&m));
        m[31] -= 1;
        assert!(below_modulus(&m));
        assert!(!below_modulus(&[0xff; 32]));
    }

    #[test]
    fn window_allows_a_shallow_reorg_and_not_an_old_payment() {
        // Created at tip 100, Finality Depth 10, expiry 120.
        assert!(block_in_window(101, 100, 120, 10));
        assert!(block_in_window(100, 100, 120, 10));
        assert!(
            block_in_window(91, 100, 120, 10),
            "lowest replaceable height"
        );
        assert!(!block_in_window(90, 100, 120, 10));
        assert!(block_in_window(120, 100, 120, 10));
        assert!(!block_in_window(121, 100, 120, 10));
    }

    #[test]
    fn leaf_index_starts_after_the_checkpoint() {
        assert_eq!(leaf_index(101, 100), Some(0));
        assert_eq!(leaf_index(105, 100), Some(4));
        assert_eq!(leaf_index(100, 100), None);
        assert_eq!(leaf_index(7, 100), None);
        assert_eq!(leaf_index(7, u32::MAX), None);
    }

    #[test]
    fn layout_is_contiguous() {
        assert_eq!(E_PAYEE + 32, E_PAYOUT);
        assert_eq!(E_PAYOUT + 32, E_MINT);
        assert_eq!(E_MINT + 32, E_VAULT);
        assert_eq!(E_VAULT + 32, E_RELAY);
        assert_eq!(E_RELAY + 32, E_COMMITMENT);
        assert_eq!(E_COMMITMENT + 32, E_AMOUNT);
        assert_eq!(E_AMOUNT + 8, E_BOND_REQUIRED);
        assert_eq!(E_BOND_REQUIRED + 8, E_BOND_DEPOSITED);
        assert_eq!(E_BOND_DEPOSITED + 8, E_CREATED_AT);
        assert_eq!(E_CREATED_AT + 8, E_CREATED_TIP);
        assert_eq!(E_CREATED_TIP + 4, E_EXPIRY_HEIGHT);
        assert_eq!(E_EXPIRY_HEIGHT + 4, E_CLAIM_MODE);
        assert_eq!(E_CLAIM_MODE + 1, E_REFUND_REQUESTED_AT);
        assert_eq!(E_REFUND_REQUESTED_AT + 8, E_RESERVED);
        assert_eq!(E_RESERVED + 55, ESCROW_LEN);
    }
}
