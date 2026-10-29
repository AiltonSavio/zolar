//! Shared parts of the Payee and Payer command line tools.
//!
//! Byte orders: Zcash tools print a transaction ID reversed (display order); the block Merkle tree, the
//! Payment Commitment and the Escrow use the internal order. [`internal_txid`] converts.

pub mod blocks;
pub mod checkout;
pub mod devtool;
pub mod explorer;
pub mod merchant;
pub mod payee;
pub mod payer;
pub mod prover;
pub mod rpc;
pub(crate) mod settlement;
pub mod solana;
#[cfg(test)]
pub(crate) mod test_support;
pub mod wallet;
pub mod zkool;

use anyhow::{Context, Result, bail};

/// The default Header Relay on Solana devnet that follows the Zcash testnet.
pub const DEVNET_RELAY: &str = "C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS";
/// The devnet relay that follows the Zcash mainnet (ticket 13).
pub const DEVNET_MAINNET_RELAY: &str = "2m1YC6pGTBpoKDHL5NQgDQDTewE7bYs4m6TdxkMG4Yhs";
pub const DEVNET_RPC: &str = "https://api.devnet.solana.com";
pub const TESTNET_EXPLORER: &str = "https://api.testnet.zecblock.com";

/// The block explorer for the Zcash mainnet (same API as the testnet one).
pub const MAINNET_EXPLORER: &str = "https://api.zecblock.com";

pub use zolar_relayer::p2p::Network;

/// The explorer to ask when the user gave none.
pub fn default_explorer(network: Network) -> &'static str {
    match network {
        Network::Testnet => TESTNET_EXPLORER,
        Network::Mainnet => MAINNET_EXPLORER,
    }
}

/// The Header Relay to use when the user gave none: the devnet relay of that Zcash network.
pub fn default_relay(network: Network) -> &'static str {
    match network {
        Network::Testnet => DEVNET_RELAY,
        Network::Mainnet => DEVNET_MAINNET_RELAY,
    }
}

/// The seeder host names of a network, as the peer pool takes them.
pub fn seeders(network: Network) -> Vec<String> {
    network.seeders().iter().map(|s| s.to_string()).collect()
}

/// Typical seconds between blocks: the testnet mines about every 25 s, the mainnet every 75 s.
pub fn block_seconds(network: Network) -> u32 {
    match network {
        Network::Testnet => 25,
        Network::Mainnet => 75,
    }
}

/// The same network for the PCZT crate, which decodes viewing keys.
pub fn pczt_network(network: Network) -> zolar_pczt::Network {
    match network {
        Network::Testnet => zolar_pczt::Network::Testnet,
        Network::Mainnet => zolar_pczt::Network::Mainnet,
    }
}

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
    parse_salt(&text).with_context(|| format!("salt file {}", path.display()))
}

/// A salt as 64 hex characters.
pub fn parse_salt(hex_salt: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_salt.trim()).context("salt is not hex")?;
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
    fn each_network_has_its_own_defaults() {
        assert_eq!(default_explorer(Network::Testnet), TESTNET_EXPLORER);
        assert_eq!(
            default_explorer(Network::Mainnet),
            "https://api.zecblock.com"
        );
        assert_ne!(
            default_relay(Network::Testnet),
            default_relay(Network::Mainnet)
        );
        assert!(
            seeders(Network::Mainnet)
                .iter()
                .all(|s| !s.contains("testnet"))
        );
        assert!(
            seeders(Network::Testnet)
                .iter()
                .all(|s| s.contains("testnet"))
        );
        assert_eq!(block_seconds(Network::Mainnet), 75);
        assert_eq!(block_seconds(Network::Testnet), 25);
        assert_eq!(pczt_network(Network::Mainnet), zolar_pczt::Network::Mainnet);
        assert_eq!(pczt_network(Network::Testnet), zolar_pczt::Network::Testnet);
    }

    #[test]
    fn salts_are_field_elements() {
        for _ in 0..100 {
            let salt = random_salt();
            assert!(zolar_claim::payment_commitment(&[1; 32], &salt).is_ok());
        }
    }
}
