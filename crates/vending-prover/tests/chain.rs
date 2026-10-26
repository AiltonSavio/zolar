use vending_prover::chain::parse_block;
#[test]
fn recorded_full_block_recomputes_all_transaction_ids_and_rejects_truncation() {
    let raw = include_bytes!("../../../cli/fixtures/block-4517478.bin");
    let (header, txids) = parse_block(raw).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../../../circuits/fixtures/swap2-block.json")).unwrap();
    let display: Vec<_> = txids
        .iter()
        .map(|t| {
            let mut t = *t;
            t.reverse();
            hex::encode(t)
        })
        .collect();
    assert_eq!(
        serde_json::to_value(display).unwrap(),
        expected["txids_display"]
    );
    assert_eq!(zolar_claim::tx_merkle_root(&txids), header[36..68]);
    assert!(parse_block(&raw[..raw.len() - 1]).is_err());
    let mut padded = raw.to_vec();
    padded.push(0);
    assert!(parse_block(&padded).is_err());
    let mut bad = raw.to_vec();
    bad[36] ^= 1;
    assert!(parse_block(&bad).is_err());
}
