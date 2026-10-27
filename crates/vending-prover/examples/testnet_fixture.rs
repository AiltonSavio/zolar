//! Rebuild a public testnet fixture using a pre-finality Relay snapshot, P2P blocks,
//! and a fresh live Relay root check. No wallet access or transaction submission.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use header_relay_interface::{RelayView, R_FILLED, R_FINALIZED_HASH};
use sha2::{Digest, Sha256};
use vending_guest::{decrypt_payment, Witness};
use vending_prover::{backend::Lightwallet, chain, hex32, pubkey};
use vending_types::Memo;
use zolar_claim::{block_leaf, poseidon, FinalBlockTree, DEPTH};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(
        args.len() == 6,
        "usage: testnet_fixture TXID RELAY SNAPSHOT_JSON RPC OUTPUT_DIR"
    );
    let (txid, relay, snapshot, rpc, out) = (&args[1], &args[2], &args[3], &args[4], &args[5]);
    let snapshot: serde_json::Value = serde_json::from_slice(&std::fs::read(snapshot)?)?;
    let account = &snapshot["result"]["value"];
    let data = STANDARD.decode(account["data"][0].as_str().context("snapshot data")?)?;
    let owner = pubkey(account["owner"].as_str().context("snapshot owner")?)?;
    let addr = pubkey(relay)?;
    let view =
        RelayView::parse(&owner, &addr, &addr, &data).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(
        view.network() == header_relay_interface::NETWORK_TESTNET,
        "testnet only"
    );
    let http = reqwest::Client::new();
    let genesis: serde_json::Value = http
        .post(rpc)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getGenesisHash","params":[]}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        genesis["result"] == "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
        "Solana devnet only"
    );
    let light = Lightwallet::new("https://zcash-testnet.chainsafe.dev".into())?;
    let mut internal = hex32(txid)?;
    internal.reverse();
    let raw = light.transaction(internal).await?;
    let height = u32::try_from(raw.height)?;
    ensure!(
        height > view.finalized_height(),
        "snapshot must predate payment finality"
    );
    let peers: Vec<String> = chain::p2p::Network::Testnet
        .seeders()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let start = data[R_FINALIZED_HASH..R_FINALIZED_HASH + 32].try_into()?;
    let headers = chain::fetch_headers(start, (height - view.finalized_height()) as usize, &peers)?;
    let header = headers.last().context("payment header")?;
    let hash = chain::p2p::sha256d(header);
    let txids = chain::fetch_txids(hash, &peers)?;
    let tx_index = txids
        .iter()
        .position(|t| *t == internal)
        .context("payment in full P2P block")?;
    ensure!(
        zolar_claim::tx_merkle_root(&txids) == header[36..68],
        "block merkle root"
    );
    let key = hex::decode(include_str!("../../vending-guest/fixtures/shop.ivk.hex").trim())?;
    let payment = decrypt_payment(&raw.data, &key, 0).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(payment.txid == internal, "txid mismatch");
    let memo = Memo::decode(&payment.memo).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let zeros = FinalBlockTree::new(vec![]);
    let mut filled = [[0; 32]; DEPTH];
    for (level, node) in filled.iter_mut().enumerate() {
        *node = data[R_FILLED + level * 32..R_FILLED + (level + 1) * 32].try_into()?;
    }
    let mut path = [[0; 32]; DEPTH];
    let mut root = [0; 32];
    for (i, h) in headers.iter().enumerate() {
        let index = view.leaf_count() as usize + i;
        let mut node = block_leaf(
            view.finalized_height() + 1 + i as u32,
            &h[36..68].try_into()?,
        )?;
        for level in 0..DEPTH {
            if (index >> level) & 1 == 0 {
                path[level] = zeros.zeros()[level];
                filled[level] = node;
                node = poseidon(&[&node, &path[level]])?;
            } else {
                path[level] = filled[level];
                node = poseidon(&[&path[level], &node])?;
            }
        }
        root = node;
    }
    let live: serde_json::Value = http.post(rpc).json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[relay,{"encoding":"base64","commitment":"finalized"}]})).send().await?.error_for_status()?.json().await?;
    let account = &live["result"]["value"];
    let live_data = STANDARD.decode(account["data"][0].as_str().context("live data")?)?;
    let live_owner = pubkey(account["owner"].as_str().context("live owner")?)?;
    let live_view = RelayView::parse(&live_owner, &addr, &addr, &live_data)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(
        live_view.is_known_root(&root),
        "payment root not known to live Relay yet (finalized={}); retry",
        live_view.finalized_height()
    );
    let witness = Witness {
        tx: raw.data,
        shop_viewing_key: key,
        action_index: 0,
        machine: memo.machine,
        nonce: memo.nonce,
        payout: memo.payout,
        diversifier: payment.diversifier,
        tx_index: tx_index as u32,
        tx_count: txids.len() as u32,
        tx_path: zolar_claim::tx_merkle_path(&txids, tx_index),
        block_merkle_root: header[36..68].try_into()?,
        height,
        tree_index: height - view.checkpoint_height() - 1,
        tree_path: path,
        root,
    };
    let values = vending_guest::verify(&witness).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    std::fs::create_dir_all(out)?;
    std::fs::write(
        format!("{out}/tx.hex"),
        format!("{}\n", hex::encode(&witness.tx)),
    )?;
    std::fs::write(format!("{out}/witness.json"), serde_json::to_vec(&witness)?)?;
    std::fs::write(format!("{out}/header.bin"), header)?;
    std::fs::write(
        format!("{out}/relay-snapshot.json"),
        serde_json::to_vec_pretty(&live)?,
    )?;
    let metadata = serde_json::json!({"network":"testnet", "txid":txid, "height":height,
        "block_hash":hex::encode(hash.iter().rev().copied().collect::<Vec<_>>()),
        "txids_internal": txids.iter().map(hex::encode).collect::<Vec<_>>(),
        "relay":relay, "root":hex::encode(root), "tree":"real Relay-known root",
        "public_values":hex::encode(values.encode()), "tx_sha256":hex::encode(Sha256::digest(&witness.tx))});
    std::fs::write(
        format!("{out}/metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!(
        "REAL_FIXTURE txid={txid} height={height} tx_count={} value={} root={} live_finalized={}",
        txids.len(),
        values.value_zat,
        hex::encode(root),
        live_view.finalized_height()
    );
    Ok(())
}
