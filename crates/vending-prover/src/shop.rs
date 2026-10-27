//! Dedicated testnet ZIP-32 account. Only the external incoming viewing key is exported.
use anyhow::{ensure, Context, Result};
use clap::Parser;
use orchard::keys::{FullViewingKey, IncomingViewingKey, Scope, SpendingKey};
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};
use zcash_address::unified::{Address, Container, Encoding, Receiver, Revision, Uitem};
use zcash_protocol::consensus::NetworkType;

#[derive(Parser)]
#[command(about = "Create or derive a dedicated Zcash-testnet shop account; never prints its seed")]
struct Cli {
    #[arg(long)]
    seed: PathBuf,
    /// Create a new 32-byte seed, refusing to overwrite anything.
    #[arg(long)]
    create: bool,
    /// Public metadata only (safe to publish as the Machine's Shop Viewing Key).
    #[arg(long)]
    out: PathBuf,
}
#[derive(Serialize, PartialEq, Debug)]
struct Shop {
    network: &'static str,
    path: &'static str,
    address: String,
    diversifier: String,
    shop_viewing_key: String,
    svk_hash: String,
    receiver: Vec<u8>,
}
fn derive(seed: &[u8]) -> Result<Shop> {
    ensure!(seed.len() == 32, "expected a raw 32-byte ZIP-32 seed");
    let sk = SpendingKey::from_zip32_seed(seed, 1, zip32::AccountId::ZERO)
        .map_err(|_| anyhow::anyhow!("ZIP-32 derivation failed"))?;
    let ivk = FullViewingKey::from(&sk).to_ivk(Scope::External);
    let key = ivk.to_bytes();
    let address = ivk.address_at(0u32);
    let receiver = address.to_raw_address_bytes();
    let ua = Address::try_from_items(Revision::R0, vec![Uitem::Data(Receiver::Orchard(receiver))])?;
    let encoded = ua.encode(&NetworkType::Test);
    let (network, _, decoded) = Address::decode(&encoded)?;
    ensure!(
        network == NetworkType::Test && decoded.items().len() == 1,
        "testnet receiver only"
    );
    let parsed = Option::<IncomingViewingKey>::from(IncomingViewingKey::from_bytes(&key))
        .context("invalid external incoming viewing key")?;
    ensure!(
        parsed.address(address.diversifier()) == address,
        "receiver/key mismatch"
    );
    Ok(Shop {
        network: "testnet",
        path: "m/32'/1'/0' external index 0",
        address: encoded,
        diversifier: hex::encode(&receiver[..11]),
        shop_viewing_key: hex::encode(key),
        svk_hash: hex::encode(Sha256::digest(key)),
        receiver: receiver.to_vec(),
    })
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.create {
        let mut seed = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&cli.seed)?;
        file.write_all(&seed)?;
        file.sync_all()?;
    }
    let metadata = std::fs::symlink_metadata(&cli.seed)?;
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o077 == 0,
        "seed must be a regular private file (mode 600)"
    );
    let shop = derive(&std::fs::read(&cli.seed)?)?;
    let json = serde_json::to_vec_pretty(&shop)?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cli.out)?;
    out.write_all(&json)?;
    println!(
        "testnet shop address={} public_metadata={}",
        shop.address,
        cli.out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedicated_testnet_account_round_trips_and_is_reproducible() {
        let a = derive(&[42; 32]).unwrap();
        assert_eq!(a, derive(&[42; 32]).unwrap());
        assert_ne!(a.address, derive(&[43; 32]).unwrap().address);
        assert_eq!(a.shop_viewing_key.len(), 128);
        assert_eq!(a.diversifier.len(), 22);
        let (_, _, ua) = Address::decode(&a.address).unwrap();
        assert!(matches!(&ua.items()[0], Receiver::Orchard(_)));
        assert!(derive(&[0; 31]).is_err());
    }
}
