//! Read-only provider checks, deliberately separate from the offline suite.
use vending_prover::backend::Lightwallet;
#[tokio::test]
#[ignore = "requires the public ChainSafe testnet endpoint"]
async fn chainsafe_returns_the_recorded_transaction_and_height() {
    let client = Lightwallet::new("https://zcash-testnet.chainsafe.dev".into()).unwrap();
    let mut txid =
        vending_prover::hex32("df172741ca487bd157b4fd2f31bef6168707944fc56ff885a520eea62128d9a7")
            .unwrap();
    txid.reverse();
    let raw = client.transaction(txid).await.unwrap();
    assert_eq!(raw.height, 4_515_934);
    assert_eq!(raw.data.len(), 9166);
    let block = client.block(raw.height as u32).await.unwrap();
    assert!(block.vtx.iter().any(|tx| tx.txid == txid));
}
