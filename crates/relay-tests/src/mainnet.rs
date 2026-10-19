//! Helpers for the mainnet Header Relay tests: 400 recorded mainnet headers, 3,522,201 to 3,522,600
//! (see `scripts/export-mainnet-headers.sh`), and a mainnet Checkpoint built from them.
use header_relay_client::types::Checkpoint;

use crate::checkpoint_before;

pub const MAINNET_FIXTURE: &str =
    include_str!("../../zcash-header/fixtures/mainnet/mainnet-3522201-3522600.txt");

/// Mainnet has no NU7 height yet: the Checkpoint says "never", so the 17-block window applies.
pub const NEVER: u32 = u32::MAX;

/// A mainnet Checkpoint on the last of `rows[..end]`, with the 113 blocks before it as context.
pub fn mainnet_checkpoint(
    rows: &[(u32, Vec<u8>)],
    end: usize,
    finality_depth: u16,
    nonce: u64,
) -> Checkpoint {
    Checkpoint {
        network: 0,
        ..checkpoint_before(rows, end, NEVER, finality_depth, nonce)
    }
}

/// The Checkpoint of a chain whose recent blocks were 256 times harder than the recorded ones (every
/// context `nBits` exponent one lower). The relay then demands a target about 256 times tighter than
/// the one real block `rows[end]` carries, so that real, valid header is a target above the real
/// difficulty of that chain. A header with real work at the mainnet limit would be the purer forgery,
/// but mining one takes about 8,192 solutions (measured: tens of seconds per solver nonce, hence core-days).
pub fn harder_mainnet_checkpoint(rows: &[(u32, Vec<u8>)], end: usize) -> Checkpoint {
    let mut checkpoint = mainnet_checkpoint(rows, end, 10, 0);
    for bits in checkpoint.bits.iter_mut() {
        *bits -= 0x0100_0000;
    }
    checkpoint
}
