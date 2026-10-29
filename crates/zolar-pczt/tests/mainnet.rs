//! The same behaviour on Zcash mainnet, where the NU6.3 branch (transaction version 6, Ironwood
//! pool) has been active since block 3,428,143 and NU7 has no height yet.
//!
//! No funded mainnet wallet is involved: the PCZT is built with the librustzcash builder from a
//! fake transparent coin and is neither proven, signed nor broadcast.

use rand::{rand_core::UnwrapErr, rngs::SysRng};
use secp256k1::SecretKey;
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_primitives::transaction::Transaction;
use zcash_primitives::transaction::{
    builder::{BuildConfig, Builder, BundlePadding},
    fees::zip317::FeeRule,
};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::{
    consensus::{BlockHeight, MAIN_NETWORK},
    memo::{Memo, MemoBytes},
    value::Zatoshis,
};
use zcash_transparent::{
    address::TransparentAddress,
    bundle::{OutPoint, TxOut},
};
use zolar_pczt::{Network, inspect, summary};

/// A height after NU6.3 activation (3,428,143) and before any NU7: the tip the mainnet relay follows.
const TARGET: u32 = 3_523_000;
const PAYMENT: u64 = 100_000;
const MEMO: &str = "zolar t13 mainnet";

struct Built {
    pczt: Vec<u8>,
    payee_ufvk: String,
    payee_address: String,
}

fn build(memo: &str) -> Built {
    build_at(memo, TARGET)
}

fn build_at(memo: &str, target: u32) -> Built {
    let payee =
        UnifiedSpendingKey::from_seed(&MAIN_NETWORK, &[7u8; 32], zip32::AccountId::ZERO).unwrap();
    let payee_ufvk = payee.to_unified_full_viewing_key();
    let (payee_address, _) = payee_ufvk
        .default_address(UnifiedAddressRequest::ALLOW_ALL)
        .unwrap();
    let ironwood_recipient = payee_address.orchard().copied().expect("orchard receiver");

    let sk = SecretKey::from_secret_bytes([9u8; 32]).unwrap();
    let pubkey = sk.public_key();
    let coin_value = Zatoshis::const_from_u64(PAYMENT + 15_000 + 1_000_000);
    let coin = TxOut::new(
        coin_value,
        TransparentAddress::from_pubkey(&pubkey).script().into(),
    );

    let mut builder = Builder::new(
        MAIN_NETWORK,
        BlockHeight::from_u32(target),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(orchard::Anchor::empty_tree()),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    )
    .with_expiry_height(BlockHeight::from_u32(target + 18));
    builder
        .add_transparent_p2pkh_input(pubkey, OutPoint::new([1u8; 32], 0), coin)
        .unwrap();
    builder
        .add_ironwood_output::<()>(
            None,
            ironwood_recipient,
            Zatoshis::from_u64(PAYMENT).unwrap(),
            MemoBytes::from(memo.parse::<Memo>().unwrap()),
        )
        .unwrap();
    let change = payee_ufvk
        .orchard()
        .unwrap()
        .address_at(1u32, zip32::Scope::Internal);
    builder
        .add_ironwood_output::<()>(
            None,
            change,
            Zatoshis::const_from_u64(1_000_000),
            MemoBytes::empty(),
        )
        .unwrap();
    let result = builder
        .build_for_pczt(UnwrapErr(SysRng), &FeeRule::standard())
        .unwrap();
    let pczt = pczt::roles::creator::Creator::build_from_parts(result.pczt_parts).unwrap();
    Built {
        pczt: pczt.serialize().unwrap(),
        payee_ufvk: payee_ufvk.encode(&MAIN_NETWORK).unwrap(),
        payee_address: payee_address.encode(&MAIN_NETWORK),
    }
}

#[test]
fn a_mainnet_payment_is_a_version_6_transaction_the_payee_can_read() {
    let built = build(MEMO);
    assert!(built.payee_address.starts_with("u1"));
    assert!(built.payee_ufvk.starts_with("uview1"));

    let report = inspect(&built.pczt, &built.payee_ufvk, Network::Mainnet).unwrap();

    assert_eq!(report.tx_version, 6);
    assert_eq!(report.expiry_height, TARGET + 18);
    assert_eq!(report.received.len(), 1);
    assert_eq!(report.received[0].zatoshis, PAYMENT);
    assert_eq!(report.received[0].memo.as_deref(), Some(MEMO));
    assert_eq!(report.received[0].pool, "ironwood");
    assert_eq!(summary(&built.pczt).unwrap().txid, report.txid);
    assert_eq!(report.txid.len(), 64);
}

#[test]
fn a_mainnet_viewing_key_is_refused_as_a_testnet_key() {
    let built = build(MEMO);
    assert!(inspect(&built.pczt, &built.payee_ufvk, Network::Testnet).is_err());
}

#[test]
fn transparent_funding_does_not_disclose_wallet_derivation_or_script_secrets() {
    let built = build(MEMO);
    let public_key = SecretKey::from_secret_bytes([9; 32])
        .unwrap()
        .public_key()
        .serialize();
    let secret = b"private transparent script preimage";
    let original = pczt::roles::updater::Updater::new(pczt::Pczt::parse(&built.pczt).unwrap())
        .update_transparent_with(|mut updater| {
            updater.update_input_with(0, |mut input| {
                input.set_bip32_derivation(
                    public_key,
                    zcash_transparent::pczt::Bip32Derivation::parse(
                        [11; 32],
                        vec![0x8000002c, 0x80000085, 0x80000000, 0, 0],
                    )
                    .unwrap(),
                );
                input.set_sha256_preimage(secret.to_vec());
                input.set_proprietary(
                    "private wallet metadata".into(),
                    b"private wallet metadata".to_vec(),
                );
                Ok(())
            })
        })
        .unwrap()
        .finish()
        .serialize()
        .unwrap();
    assert!(original.windows(secret.len()).any(|w| w == secret));
    let copy = zolar_pczt::redact_for_payee(&original).unwrap();
    assert!(!copy.windows(secret.len()).any(|w| w == secret));
    assert_eq!(summary(&copy).unwrap(), summary(&original).unwrap());
    assert_eq!(
        inspect(&copy, &built.payee_ufvk, Network::Mainnet).unwrap(),
        inspect(&original, &built.payee_ufvk, Network::Mainnet).unwrap()
    );
    let parsed = pczt::Pczt::parse(&copy).unwrap();
    let input = serde_json::to_value(&parsed.transparent().inputs()[0]).unwrap();
    for field in [
        "bip32_derivation",
        "sha256_preimages",
        "proprietary",
        "partial_signatures",
    ] {
        assert_eq!(input[field], serde_json::json!({}), "retained {field}");
    }
}

/// A real mainnet transaction (block 3,522,900, 2026-10-19), as served by Blockchair: version 6 on
/// the NU6.3 branch with an Ironwood bundle. Its ID is the one the explorers print.
#[test]
fn the_librustzcash_txid_of_a_real_mainnet_v6_transaction_is_the_explorers_txid() {
    let raw = hex::decode(include_str!("fixtures/mainnet-v6-16394087.hex").trim()).unwrap();
    let tx = Transaction::read(&raw[..], BranchId::Nu6_3).unwrap();
    assert_eq!(tx.version().header(), 0x8000_0006);
    assert!(tx.ironwood_bundle().is_some());
    assert_eq!(
        tx.txid().to_string(),
        "16394087c0fbb594e2d52f2aff85415b33f33b310a08a546d2a9e6e57259e2e0"
    );
}

/// Rehearsal helper, not a check: writes a mainnet-format PCZT (fake coin, never signed) plus the
/// Payee's viewing key and address to `$ZOLAR_REHEARSAL_DIR`, with expiry `$ZOLAR_REHEARSAL_TARGET + 18`
/// and memo `$ZOLAR_REHEARSAL_MEMO`, so the Payee CLI can run against the mainnet relay without a
/// funded wallet. `cargo test --test mainnet -- --ignored write_rehearsal_pczt`.
#[test]
#[ignore]
fn write_rehearsal_pczt() {
    let dir = std::path::PathBuf::from(std::env::var("ZOLAR_REHEARSAL_DIR").unwrap());
    let target: u32 = std::env::var("ZOLAR_REHEARSAL_TARGET")
        .unwrap()
        .parse()
        .unwrap();
    let memo = std::env::var("ZOLAR_REHEARSAL_MEMO").unwrap();
    let built = build_at(&memo, target);
    std::fs::write(dir.join("created.pczt"), &built.pczt).unwrap();
    std::fs::write(dir.join("payee.ufvk"), &built.payee_ufvk).unwrap();
    std::fs::write(dir.join("payee.address"), &built.payee_address).unwrap();
}
