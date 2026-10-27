#![cfg(feature = "sp1")]
use sp1_sdk::{Elf, Prover, ProverClient, SP1Stdin};
use vending_guest::Witness;

fn fixture(kind: &str) -> Witness {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../vending-guest/fixtures/synthetic/{kind}.json"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
#[tokio::test]
async fn sp1_full_statement_and_all_requested_negative_cases() {
    let path =
        std::env::var("VENDING_GUEST_ELF").expect("build the guest and set VENDING_GUEST_ELF");
    let elf = Elf::from(std::fs::read(path).unwrap());
    let client = ProverClient::builder().light().build().await;
    let good = fixture("valid");
    let mut stdin = SP1Stdin::new();
    stdin.write(&good);
    let start = std::time::Instant::now();
    let (pv, report) = client.execute(elf.clone(), stdin).await.unwrap();
    assert_eq!(report.exit_code, 0);
    assert_eq!(
        pv.as_slice(),
        vending_guest::verify(&good).unwrap().encode()
    );
    println!(
        "FULL_GUEST cycles={} elapsed={:?} public_values_bytes={}",
        report.total_instruction_count() + report.total_syscall_count(),
        start.elapsed(),
        pv.as_slice().len()
    );
    let mut cases = Vec::new();
    for kind in [
        "value",
        "rseed",
        "rho",
        "memo",
        "uppercase",
        "spaces",
        "base58",
    ] {
        cases.push((kind, fixture(kind)));
    }
    let mut w = good.clone();
    w.diversifier[0] ^= 1;
    cases.push(("diversifier", w));
    let mut w = good.clone();
    w.shop_viewing_key[32] ^= 1;
    cases.push(("viewing-key", w));
    let mut w = good.clone();
    w.machine[0] ^= 1;
    cases.push(("machine", w));
    let mut w = good.clone();
    w.payout[0] ^= 1;
    cases.push(("payout", w));
    let mut w = good.clone();
    w.nonce[0] ^= 1;
    cases.push(("nonce", w));
    let mut w = good.clone();
    w.tx_count = 2;
    w.tx_path.push([7; 32]);
    cases.push(("merkle-path", w));
    let mut w = good.clone();
    w.root[0] ^= 1;
    cases.push(("root", w));
    let mut w = good.clone();
    w.tree_path[0] = [255; 32];
    cases.push(("field-element", w));
    let mut w = good.clone();
    w.tx =
        hex::decode(include_str!("../../vending-guest/fixtures/ticket02-tx.hex").trim()).unwrap();
    let parsed = zcash_primitives::transaction::Transaction::read(
        w.tx.as_slice(),
        zcash_protocol::consensus::BranchId::Nu7,
    )
    .unwrap();
    w.block_merkle_root = *parsed.txid().as_ref();
    w.tx_count = 1;
    w.tx_path.clear();
    let tree = zolar_claim::FinalBlockTree::new(vec![zolar_claim::block_leaf(
        w.height,
        &w.block_merkle_root,
    )
    .unwrap()]);
    w.tree_path = tree.path(0, 1).try_into().unwrap();
    w.root = tree.root(1);
    cases.push(("recorded-text-memo", w));
    for (name, w) in cases {
        let mut stdin = SP1Stdin::new();
        stdin.write(&w);
        match client.execute(elf.clone(), stdin).await {
            Ok((values, report)) => {
                assert_ne!(report.exit_code, 0, "accepted {name}");
                assert!(values.as_slice().is_empty(), "committed values for {name}");
                println!(
                    "REJECTED {name} exit_code={} public_values_bytes=0",
                    report.exit_code
                );
            }
            Err(error) => println!("REJECTED {name} executor error={error}"),
        }
    }
}

#[tokio::test]
async fn sp1_real_testnet_payment_with_real_block_merkle_path() {
    let w: Witness = serde_json::from_str(include_str!(
        "../../vending-guest/fixtures/testnet/witness.json"
    ))
    .unwrap();
    let elf = Elf::from(std::fs::read(std::env::var("VENDING_GUEST_ELF").unwrap()).unwrap());
    let mut stdin = SP1Stdin::new();
    stdin.write(&w);
    let client = ProverClient::builder().light().build().await;
    let start = std::time::Instant::now();
    let (pv, report) = client.execute(elf, stdin).await.unwrap();
    assert_eq!(report.exit_code, 0);
    assert_eq!(pv.as_slice(), vending_guest::verify(&w).unwrap().encode());
    println!(
        "REAL_GUEST cycles={} elapsed={:?} public_values_bytes={}",
        report.total_instruction_count() + report.total_syscall_count(),
        start.elapsed(),
        pv.as_slice().len()
    );
}
