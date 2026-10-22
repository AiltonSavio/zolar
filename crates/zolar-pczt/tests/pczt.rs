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

mod redaction {
    use super::*;
    use zcash_keys::keys::UnifiedFullViewingKey;
    use zcash_protocol::consensus::TEST_NETWORK;
    use zolar_pczt::redact_for_payee;

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn ironwood_actions(bytes: &[u8]) -> Vec<pczt::orchard::Action> {
        pczt::Pczt::parse(bytes)
            .unwrap()
            .ironwood()
            .actions()
            .clone()
    }

    #[test]
    fn the_payees_copy_has_the_same_transaction_and_payment() {
        let redacted = redact_for_payee(PCZT).unwrap();
        assert_eq!(summary(&redacted).unwrap(), summary(PCZT).unwrap());
        let full = inspect(PCZT, PAYEE_UFVK.trim(), Network::Testnet).unwrap();
        let copy = inspect(&redacted, PAYEE_UFVK.trim(), Network::Testnet).unwrap();
        assert_eq!(
            copy, full,
            "same txid, expiry, amount and memo for the Payee"
        );
    }

    #[test]
    fn the_payees_copy_carries_nothing_private_to_the_payer() {
        let redacted = redact_for_payee(PCZT).unwrap();
        // The original does carry them: the test would pass vacuously otherwise.
        let before = ironwood_actions(PCZT);
        assert!(
            before.iter().any(|a| a.spend().witness().is_some()),
            "witness in the original"
        );
        assert!(
            before.iter().all(|a| a.output().value().is_some()),
            "output values in the original"
        );
        for a in ironwood_actions(&redacted) {
            assert!(a.spend().witness().is_none());
            assert!(a.spend().dummy_sk().is_none());
            assert!(a.spend().proprietary().is_empty());
            assert!(a.output().recipient().is_none());
            assert!(a.output().value().is_none());
            assert!(a.output().rseed().is_none());
            assert!(a.output().user_address().is_none());
        }
        // The Payer's full viewing key for Ironwood/Orchard is not in the copy.
        let fvk = UnifiedFullViewingKey::decode(&TEST_NETWORK, PAYER_UFVK.trim())
            .unwrap()
            .orchard()
            .unwrap()
            .to_bytes();
        assert!(
            contains(PCZT, &fvk),
            "the unredacted PCZT carries the Payer's viewing key"
        );
        assert!(!contains(&redacted, &fvk));
    }

    #[test]
    fn a_redacted_copy_can_be_redacted_again() {
        let once = redact_for_payee(PCZT).unwrap();
        let twice = redact_for_payee(&once).unwrap();
        assert_eq!(summary(&twice).unwrap(), summary(PCZT).unwrap());
    }
}
