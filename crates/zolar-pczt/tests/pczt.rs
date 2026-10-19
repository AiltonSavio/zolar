//! Behaviour of `zolar_pczt::inspect` against a real unsigned Ironwood payment built by
//! zcash-devtool on Zcash testnet (NU7). The expected transaction ID was printed by
//! `zcash-devtool pczt inspect` before signing and later matched the mined transaction.

use zolar_pczt::{Network, inspect, summary};

const PCZT: &[u8] = include_bytes!("fixtures/payment-unsigned.pczt");
const PAYEE_UFVK: &str = include_str!("fixtures/payee.ufvk");
const PAYER_UFVK: &str = include_str!("fixtures/payer.ufvk");
const TXID: &str = "df172741ca487bd157b4fd2f31bef6168707944fc56ff885a520eea62128d9a7";

#[test]
fn txid_is_computed_from_the_unsigned_pczt() {
    let report = inspect(PCZT, PAYEE_UFVK.trim(), Network::Testnet).unwrap();
    assert_eq!(report.txid, TXID);
    assert_eq!(report.tx_version, 6);
    assert_eq!(report.expiry_height, 4_515_973);
}

#[test]
fn the_payer_reads_txid_and_expiry_without_any_viewing_key() {
    let s = summary(PCZT).unwrap();
    assert_eq!(s.txid, TXID);
    assert_eq!(s.tx_version, 6);
    assert_eq!(s.expiry_height, 4_515_973);
    assert!(summary(b"not a pczt").is_err());
}

#[test]
fn payee_sees_amount_and_memo_with_only_its_viewing_key() {
    let report = inspect(PCZT, PAYEE_UFVK.trim(), Network::Testnet).unwrap();
    assert_eq!(report.received.len(), 1);
    let out = &report.received[0];
    assert_eq!(out.zatoshis, 2_500_000);
    assert_eq!(out.memo.as_deref(), Some("zolar t02 spike"));
    assert_eq!(out.pool, "ironwood");
}

#[test]
fn the_payer_viewing_key_sees_only_its_change() {
    let report = inspect(PCZT, PAYER_UFVK.trim(), Network::Testnet).unwrap();
    assert_eq!(report.txid, TXID);
    assert!(report.received.iter().all(|o| o.zatoshis != 2_500_000));
}

#[test]
fn garbage_is_rejected() {
    assert!(inspect(b"not a pczt", PAYEE_UFVK.trim(), Network::Testnet).is_err());
    assert!(inspect(PCZT, "uviewtest1garbage", Network::Testnet).is_err());
}
