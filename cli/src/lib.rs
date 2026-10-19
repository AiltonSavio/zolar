//! Shared parts of the Payee and Payer command line tools.
//!
//! Byte orders: Zcash tools print a transaction ID reversed (display order); the block Merkle tree, the
//! Payment Commitment and the Escrow use the internal order. [`internal_txid`] converts.

pub mod blocks;
pub mod devtool;
pub mod explorer;
pub mod payee;
pub mod payer;
pub mod solana;

use anyhow::{Context, Result, bail};

/// The default Header Relay on Solana devnet that follows the Zcash testnet.
pub const DEVNET_RELAY: &str = "C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS";
pub const DEVNET_RPC: &str = "https://api.devnet.solana.com";
pub const TESTNET_EXPLORER: &str = "https://api.testnet.zecblock.com";

/// A transaction ID in display order (64 hex digits) as the 32 bytes the Merkle tree uses.
pub fn internal_txid(display: &str) -> Result<[u8; 32]> {
    let mut bytes: [u8; 32] = hex::decode(display.trim())
        .context("transaction ID is not hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("transaction ID is not 32 bytes"))?;
    bytes.reverse();
    Ok(bytes)
}

/// The display form of an internal-order transaction ID.
pub fn display_txid(internal: &[u8; 32]) -> String {
    let mut bytes = *internal;
    bytes.reverse();
    hex::encode(bytes)
}

/// A random salt below the BN254 modulus: 31 random bytes, so it can never reach the modulus.
pub fn random_salt() -> [u8; 32] {
    use rand::RngCore;
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt[1..]);
    salt
}

pub fn read_salt(path: &std::path::Path) -> Result<[u8; 32]> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = hex::decode(text.trim()).context("salt file is not hex")?;
    match bytes.try_into() {
        Ok(salt) => Ok(salt),
        Err(_) => bail!("salt is not 32 bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_internal_txid_orders_are_reverses() {
        let display = "df172741ca487bd157b4fd2f31bef6168707944fc56ff885a520eea62128d9a7";
        let internal = internal_txid(display).unwrap();
        assert_eq!(internal[0], 0xa7);
        assert_eq!(internal[31], 0xdf);
        assert_eq!(display_txid(&internal), display);
        assert!(internal_txid("abcd").is_err());
    }

    #[test]
    fn salts_are_field_elements() {
        for _ in 0..100 {
            let salt = random_salt();
            assert!(zolar_claim::payment_commitment(&[1; 32], &salt).is_ok());
        }
    }
}
