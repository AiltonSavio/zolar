//! Mixed-pool funding must not disclose the Payer's notes to the Payee. These PCZTs are built from
//! synthetic notes, never proven, signed or sent; the Payee's decryption uses the real library.
use incrementalmerkletree::{Hashable, Level, Position};
use rand::{rand_core::UnwrapErr, rngs::SysRng};
use serde_json::Value;
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_primitives::transaction::{
    builder::{BuildConfig, Builder, BundlePadding},
    fees::fixed::FeeRule,
};
use zcash_protocol::{
    consensus::{BlockHeight, MAIN_NETWORK},
    memo::MemoBytes,
    value::Zatoshis,
};
use zolar_pczt::{Network, inspect, redact_for_payee, summary};

fn sapling_funded_payment() -> (Vec<u8>, String) {
    let sk = sapling::zip32::ExtendedSpendingKey::master(&[31; 32]).unwrap();
    let fvk = sk.to_diversifiable_full_viewing_key();
    let recipient = fvk.default_address().1;
    let note = recipient.create_note(
        sapling::value::NoteValue::from_raw(135_000),
        sapling::Rseed::AfterZip212([42; 32]),
    );
    let leaf = sapling::Node::from_cmu(&note.cmu());
    let path = sapling::MerklePath::from_parts(
        (0..32)
            .map(|i| sapling::Node::empty_root(Level::from(i)))
            .collect(),
        Position::from(0),
    )
    .unwrap();
    let payee = UnifiedSpendingKey::from_seed(&MAIN_NETWORK, &[7; 32], zip32::AccountId::ZERO)
        .unwrap()
        .to_unified_full_viewing_key();
    let mut builder = Builder::new(
        MAIN_NETWORK,
        BlockHeight::from_u32(3_523_000),
        BuildConfig::Standard {
            sapling_anchor: Some(path.root(leaf).into()),
            orchard_anchor: None,
            ironwood_anchor: Some(orchard::Anchor::empty_tree()),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    builder
        .add_sapling_spend::<()>(fvk.fvk().clone(), note, path)
        .unwrap();
    builder
        .add_sapling_output::<()>(
            None,
            recipient,
            Zatoshis::const_from_u64(5_000),
            MemoBytes::empty(),
        )
        .unwrap();
    builder
        .add_ironwood_output::<()>(
            None,
            payee
                .orchard()
                .unwrap()
                .address_at(0u32, orchard::keys::Scope::External),
            Zatoshis::const_from_u64(100_000),
            MemoBytes::empty(),
        )
        .unwrap();
    let result = builder
        .build_for_pczt(
            UnwrapErr(SysRng),
            &FeeRule::non_standard(Zatoshis::const_from_u64(30_000)),
        )
        .unwrap();
    let pczt = pczt::roles::creator::Creator::build_from_parts(result.pczt_parts).unwrap();
    (
        pczt.serialize().unwrap(),
        payee.encode(&MAIN_NETWORK).unwrap(),
    )
}

fn private_fields_absent(value: &Value, fields: &[&str]) {
    for field in fields {
        assert!(
            value[*field].is_null(),
            "Payee's copy retains private field {field}"
        );
    }
    assert_eq!(value["proprietary"], serde_json::json!({}));
}

#[test]
fn sapling_funding_and_change_are_redacted_without_changing_the_payment() {
    let (full, ufvk) = sapling_funded_payment();
    let before = pczt::Pczt::parse(&full).unwrap();
    let spend = serde_json::to_value(&before.sapling().spends()[0]).unwrap();
    assert_eq!(spend["value"], 135_000);
    assert!(!spend["witness"].is_null());
    assert!(
        before
            .sapling()
            .outputs()
            .iter()
            .any(|o| o.value().is_some())
    );

    let copy = redact_for_payee(&full).unwrap();
    assert_eq!(summary(&copy).unwrap(), summary(&full).unwrap());
    let received = inspect(&copy, &ufvk, Network::Mainnet).unwrap();
    assert_eq!(received, inspect(&full, &ufvk, Network::Mainnet).unwrap());
    assert_eq!(received.received[0].zatoshis, 100_000);
    let parsed = pczt::Pczt::parse(&copy).unwrap();
    for spend in parsed.sapling().spends() {
        private_fields_absent(
            &serde_json::to_value(spend).unwrap(),
            &[
                "recipient",
                "value",
                "rcm",
                "rseed",
                "rcv",
                "proof_generation_key",
                "witness",
                "alpha",
                "zip32_derivation",
                "dummy_ask",
            ],
        );
    }
    for output in parsed.sapling().outputs() {
        private_fields_absent(
            &serde_json::to_value(output).unwrap(),
            &[
                "recipient",
                "value",
                "rseed",
                "rcv",
                "ock",
                "zip32_derivation",
                "user_address",
            ],
        );
    }
    assert_eq!(redact_for_payee(&copy).unwrap(), copy);
}
