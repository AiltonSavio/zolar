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
    assert_eq!(&b[189..], &[0; 60]);
    assert_eq!(PublicValues::decode(&b).unwrap(), v);
}

#[test]
fn decoding_rejects_noncanonical_values() {
    let good = values().encode();
    for (offset, byte) in [(0, 2), (188, 2), (189, 1), (248, 1)] {
        let mut b = good;
        b[offset] = byte;
        assert!(PublicValues::decode(&b).is_err());
    }
    assert!(PublicValues::decode(&good[..248]).is_err());
    assert!(PublicValues::decode(&[0; 250]).is_err());
}

#[test]
fn binary_memo_is_exact_and_rejects_all_other_formats() {
    let m = Memo {
        machine: [8; 32],
        nonce: [9; 32],
    };
    let good = m.encode();
    assert_eq!(&good[..2], &[255, 1]);
    assert_eq!(&good[2..34], &m.machine);
    assert_eq!(&good[34..66], &m.nonce);
    assert_eq!(&good[66..], &[0; 446]);
    assert_eq!(Memo::decode(&good).unwrap(), m);
    for offset in [0, 1, 66, 511] {
        let mut b = good;
        b[offset] ^= 1;
        assert!(Memo::decode(&b).is_err());
    }
    assert!(Memo::decode(&good[..511]).is_err());
}
