use vending_types::{Memo, PublicValues, PUBLIC_VALUES_LEN};

fn values() -> PublicValues {
    PublicValues {
        branch_id: 0x77190ad9,
        machine: [2; 32],
        nonce: [3; 32],
        value_zat: 2_500_000,
        payment_tag: [4; 32],
        root: [5; 32],
        height: 4_515_934,
        diversifier: [6; 11],
        svk_hash: [7; 32],
        payout: [10; 32],
    }
}

#[test]
fn exact_contract_offsets_and_round_trip() {
    let v = values();
    let b = v.encode();
    assert_eq!(PUBLIC_VALUES_LEN, 249);
    assert_eq!(b.len(), 249);
    assert_eq!(b[0], 1);
    assert_eq!(&b[1..5], &v.branch_id.to_le_bytes());
    assert_eq!(&b[5..37], &v.machine);
    assert_eq!(&b[37..69], &v.nonce);
    assert_eq!(&b[69..77], &v.value_zat.to_le_bytes());
    assert_eq!(&b[77..109], &v.payment_tag);
    assert_eq!(&b[109..141], &v.root);
    assert_eq!(&b[141..145], &v.height.to_le_bytes());
    assert_eq!(&b[145..156], &v.diversifier);
    assert_eq!(&b[156..188], &v.svk_hash);
    assert_eq!(b[188], 3);
    assert_eq!(&b[189..221], &v.payout);
    assert_eq!(&b[221..], &[0; 28]);
    assert_eq!(PublicValues::decode(&b).unwrap(), v);
}

#[test]
fn decoding_rejects_noncanonical_values() {
    let good = values().encode();
    for (offset, byte) in [(0, 2), (188, 2), (221, 1), (248, 1)] {
        let mut b = good;
        b[offset] = byte;
        assert!(PublicValues::decode(&b).is_err());
    }
    assert!(PublicValues::decode(&good[..248]).is_err());
    assert!(PublicValues::decode(&[0; 250]).is_err());
}

#[test]
fn text_memo_is_exact_and_round_trips() {
    let m = Memo {
        machine: [0; 32],
        nonce: [0; 32],
        payout: [0; 32],
    };
    let good = m.encode();
    let text = format!("zv1:{0}:{0}:{0}", "1".repeat(32));
    assert_eq!(&good[..text.len()], text.as_bytes());
    assert!(good[text.len()..].iter().all(|b| *b == 0));
    assert_eq!(Memo::decode(&good).unwrap(), m);
    for bytes in [[2; 32], [255; 32], [0; 32]] {
        let m = Memo {
            machine: bytes,
            nonce: bytes,
            payout: bytes,
        };
        assert_eq!(Memo::decode(&m.encode()).unwrap(), m);
    }
    assert!(Memo::decode(&good[..511]).is_err());
}

#[test]
fn memo_rejects_noncanonical_base58_spaces_uppercase_extra_fields_and_padding() {
    let key = "1".repeat(32);
    let valid = format!("zv1:{key}:{key}:{key}");
    for text in [
        valid.replacen("zv1", "ZV1", 1),
        format!(" {valid}"),
        format!("{valid} "),
        format!("zv1:{key}: {key}:{key}"),
        format!("zv1:1{key}:{key}:{key}"),
        format!("zv1:{}:{key}:{key}", "1".repeat(31)),
        format!("zv1:{key}:{key}:0{key}"),
        format!("{valid}:{key}"),
        valid.replace(':', ":\0"),
    ] {
        let mut memo = [0; 512];
        memo[..text.len()].copy_from_slice(text.as_bytes());
        assert!(Memo::decode(&memo).is_err(), "accepted {text:?}");
    }
    let mut bad = [0; 512];
    bad[..valid.len()].copy_from_slice(valid.as_bytes());
    bad[511] = 1;
    assert!(Memo::decode(&bad).is_err());
    bad[0] = 255;
    assert!(Memo::decode(&bad).is_err());
}
