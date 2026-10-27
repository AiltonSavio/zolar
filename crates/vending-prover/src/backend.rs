use crate::{chain, hex32, pubkey, wire, FindRequest, FoundPayment, PaymentBackend, ProveRequest};
use anyhow::{anyhow, bail, ensure, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use header_relay_interface::{RelayView, CP_HASH, R_CHECKPOINT};
use orchard::{
    keys::IncomingViewingKey,
    note::{ExtractedNoteCommitment, Nullifier},
    note_encryption::{CompactAction, IronwoodDomain},
};
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};
use vending_guest::{decrypt_payment, Witness};
use vending_types::Memo;
use zcash_note_encryption::{try_compact_note_decryption, EphemeralKeyBytes};

/// Public shop metadata. StaticShops is an explicit offline fallback.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShopConfig {
    pub machine: String,
    /// Hex of the public external incoming viewing key, never a full/spending key.
    pub shop_viewing_key: String,
    pub svk_hash: String,
    pub diversifier: String,
    pub relay: String,
    pub scan_start: u32,
}
impl ShopConfig {
    pub fn validate(&self) -> Result<()> {
        pubkey(&self.machine)?;
        pubkey(&self.relay)?;
        let key = hex::decode(&self.shop_viewing_key)?;
        let raw: &[u8; 64] = key
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("Shop Viewing Key must be 64 bytes"))?;
        ensure!(
            bool::from(IncomingViewingKey::from_bytes(raw).is_some()),
            "invalid Shop Viewing Key"
        );
        ensure!(
            hex::decode(&self.diversifier)?.len() == 11,
            "diversifier must be 11 bytes"
        );
        ensure!(
            <[u8; 32]>::from(Sha256::digest(raw)) == hex32(&self.svk_hash)?,
            "Shop Viewing Key hash mismatch"
        );
        Ok(())
    }
}
#[async_trait]
pub trait ShopSource: Send + Sync {
    async fn shop(&self, machine: &str) -> Result<ShopConfig>;
}
pub struct StaticShops(HashMap<String, ShopConfig>);
impl StaticShops {
    pub fn new(shops: Vec<ShopConfig>) -> Result<Self> {
        let mut map = HashMap::new();
        for shop in shops {
            shop.validate()?;
            ensure!(
                map.insert(shop.machine.clone(), shop).is_none(),
                "duplicate machine"
            );
        }
        Ok(Self(map))
    }
}
#[async_trait]
impl ShopSource for StaticShops {
    async fn shop(&self, machine: &str) -> Result<ShopConfig> {
        self.0
            .get(machine)
            .cloned()
            .ok_or_else(|| anyhow!("unknown machine"))
    }
}
/// Codama vending program ID from the program worker branch.
pub const VENDING_PROGRAM_ID: &str = "J9paKuXLpRr4rmWVp8vBZ6ZMwQhrTSk6w3Rmyv5JHFW5";

/// Revision 2 Machine: exact 291 bytes; validates owner, discriminator, version and SVK hash.
pub fn decode_machine(
    machine: &str,
    owner: &str,
    executable: bool,
    data: &[u8],
) -> Result<ShopConfig> {
    ensure!(
        owner == VENDING_PROGRAM_ID && !executable,
        "wrong Vending Machine owner or executable account"
    );
    ensure!(
        data.len() == 291 && data[..2] == [1, 1],
        "invalid revision 2 Vending Machine layout"
    );
    let shop = ShopConfig {
        machine: machine.into(),
        relay: bs58::encode(&data[100..132]).into_string(),
        diversifier: hex::encode(&data[148..159]),
        svk_hash: hex::encode(&data[159..191]),
        shop_viewing_key: hex::encode(&data[227..291]),
        scan_start: 0,
    };
    shop.validate()?;
    Ok(shop)
}

pub struct RpcShops {
    rpc: String,
    http: reqwest::Client,
}
impl RpcShops {
    pub fn new(rpc: String) -> Result<Self> {
        Ok(Self {
            rpc,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()?,
        })
    }
}
#[async_trait]
impl ShopSource for RpcShops {
    async fn shop(&self, machine: &str) -> Result<ShopConfig> {
        pubkey(machine)?;
        let response: serde_json::Value = self
            .http
            .post(&self.rpc)
            .json(&json!({
                "jsonrpc":"2.0", "id":1, "method":"getAccountInfo",
                "params":[machine, {"encoding":"base64", "commitment":"finalized"}]
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(
            response.get("error").is_none(),
            "Solana RPC: {}",
            response["error"]
        );
        let value = &response["result"]["value"];
        ensure!(!value.is_null(), "missing Vending Machine");
        let data = STANDARD.decode(value["data"][0].as_str().context("missing Machine data")?)?;
        decode_machine(
            machine,
            value["owner"].as_str().context("missing Machine owner")?,
            value["executable"]
                .as_bool()
                .context("missing executable flag")?,
            &data,
        )
    }
}

#[derive(Clone)]
pub struct Lightwallet {
    http: reqwest::Client,
    endpoint: String,
}
impl Lightwallet {
    pub fn new(endpoint: String) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()?,
            endpoint,
        })
    }
    async fn call<R: Message + Default>(&self, method: &str, request: &impl Message) -> Result<R> {
        let mut response = self
            .http
            .post(format!(
                "{}/cash.z.wallet.sdk.rpc.CompactTxStreamer/{method}",
                self.endpoint.trim_end_matches('/')
            ))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(wire::frame(request))
            .send()
            .await?
            .error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                body.len() + chunk.len() <= 8 * 1024 * 1024,
                "gRPC response too large"
            );
            body.extend(chunk);
        }
        let frames = wire::unframe(&body)?;
        ensure!(frames.len() == 1, "expected unary gRPC result");
        Ok(R::decode(frames[0].as_slice())?)
    }
    pub async fn transaction(&self, txid: [u8; 32]) -> Result<wire::RawTransaction> {
        self.call(
            "GetTransaction",
            &wire::TxFilter {
                hash: txid.to_vec(),
            },
        )
        .await
    }
    pub async fn block(&self, height: u32) -> Result<wire::CompactBlock> {
        let block: wire::CompactBlock = self
            .call(
                "GetBlock",
                &wire::BlockId {
                    height: height as u64,
                    hash: vec![],
                },
            )
            .await?;
        ensure!(block.height == height as u64, "wrong compact block height");
        Ok(block)
    }
}

#[derive(Clone)]
pub struct ChainBackend {
    shops: Arc<dyn ShopSource>,
    rpc: String,
    http: reqwest::Client,
    light: Lightwallet,
    peers: Vec<String>,
}
impl ChainBackend {
    pub async fn new(
        shops: Arc<dyn ShopSource>,
        rpc: String,
        lightwallet: String,
        peers: Vec<String>,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        let backend = Self {
            shops,
            rpc,
            http,
            light: Lightwallet::new(lightwallet)?,
            peers,
        };
        let genesis = backend.rpc("getGenesisHash", json!([])).await?;
        ensure!(
            genesis == "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
            "only Solana devnet is supported"
        );
        Ok(backend)
    }
    async fn rpc(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let v: serde_json::Value = self
            .http
            .post(&self.rpc)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(v.get("error").is_none(), "Solana RPC: {}", v["error"]);
        v.get("result").cloned().context("missing RPC result")
    }
    async fn relay(&self, shop: &ShopConfig) -> Result<(Vec<u8>, [u8; 32])> {
        let response = self
            .rpc(
                "getAccountInfo",
                json!([shop.relay,{"encoding":"base64","commitment":"finalized"}]),
            )
            .await?;
        let value = &response["value"];
        ensure!(!value.is_null(), "missing Relay");
        let data = STANDARD.decode(value["data"][0].as_str().context("missing account data")?)?;
        let owner = pubkey(value["owner"].as_str().context("missing owner")?)?;
        let addr = pubkey(&shop.relay)?;
        let view =
            RelayView::parse(&owner, &addr, &addr, &data).map_err(|e| anyhow!("Relay {e:?}"))?;
        ensure!(
            view.network() == header_relay_interface::NETWORK_TESTNET,
            "only Zcash testnet is supported"
        );
        Ok((data, owner))
    }
    pub async fn discover_nonce(&self, txid: &str, machine: &str) -> Result<String> {
        let shop = self.shops.shop(machine).await?;
        shop.validate()?;
        let mut internal = hex32(txid)?;
        internal.reverse();
        let raw = self.light.transaction(internal).await?;
        let key = hex::decode(shop.shop_viewing_key)?;
        let tx = zcash_primitives::transaction::Transaction::read(
            raw.data.as_slice(),
            zcash_protocol::consensus::BranchId::Nu7,
        )?;
        if let Some(bundle) = tx.ironwood_bundle() {
            for index in 0..bundle.actions().len() {
                let Ok(payment) = decrypt_payment(&raw.data, &key, index as u32) else {
                    continue;
                };
                let Ok(memo) = Memo::decode(&payment.memo) else {
                    continue;
                };
                if memo.machine == pubkey(machine)? {
                    return Ok(bs58::encode(memo.nonce).into_string());
                }
            }
        }
        bail!("transaction has no vending text memo for this machine")
    }
}

fn compact_matches(
    action: &wire::CompactAction,
    key: &IncomingViewingKey,
    d: &[u8],
) -> Result<bool> {
    let nf = Option::<Nullifier>::from(Nullifier::from_bytes(
        &action.nullifier.as_slice().try_into()?,
    ))
    .context("invalid compact nullifier")?;
    let cmx = Option::<ExtractedNoteCommitment>::from(ExtractedNoteCommitment::from_bytes(
        &action.cmx.as_slice().try_into()?,
    ))
    .context("invalid compact cmx")?;
    let compact = CompactAction::from_parts(
        nf,
        cmx,
        EphemeralKeyBytes(action.ephemeral_key.as_slice().try_into()?),
        action.ciphertext.as_slice().try_into()?,
    );
    Ok(try_compact_note_decryption(
        &IronwoodDomain::for_compact_action(&compact),
        &key.prepare(),
        &compact,
    )
    .is_some_and(|(_, address)| address.diversifier().as_array() == d))
}
#[async_trait]
impl PaymentBackend for ChainBackend {
    async fn prepare(&self, r: &ProveRequest) -> Result<Witness> {
        r.validate()?;
        let shop = self.shops.shop(&r.machine).await?;
        shop.validate()?;
        let mut txid = hex32(&r.txid)?;
        txid.reverse();
        let raw = self.light.transaction(txid).await?;
        if raw.height == 0 || raw.height == u64::MAX {
            return Err(crate::WaitingFinality.into());
        }
        let height = u32::try_from(raw.height).context("payment is not in the canonical chain")?;
        let (data, owner) = self.relay(&shop).await?;
        let addr = pubkey(&shop.relay)?;
        let view = RelayView::parse(&owner, &addr, &addr, &data).map_err(|e| anyhow!("{e:?}"))?;
        ensure!(
            height > view.checkpoint_height(),
            "payment predates Relay Checkpoint"
        );
        if height > view.finalized_height() {
            return Err(crate::WaitingFinality.into());
        }
        let size = view.leaf_count() as usize;
        let checkpoint = view.checkpoint_height();
        let start: [u8; 32] =
            data[R_CHECKPOINT + CP_HASH..R_CHECKPOINT + CP_HASH + 32].try_into()?;
        let peers = self.peers.clone();
        let headers =
            tokio::task::spawn_blocking(move || chain::fetch_headers(start, size, &peers))
                .await??;
        let leaves = headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                zolar_claim::block_leaf(checkpoint + 1 + i as u32, &h[36..68].try_into().unwrap())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let tree = zolar_claim::FinalBlockTree::new(leaves);
        let root = tree.root(size);
        ensure!(
            root == view.latest_root(),
            "P2P tree does not match Relay root"
        );
        let leaf = (height - checkpoint - 1) as usize;
        let hash = chain::p2p::sha256d(&headers[leaf]);
        let peers = self.peers.clone();
        let txids = tokio::task::spawn_blocking(move || chain::fetch_txids(hash, &peers)).await??;
        let tx_index = txids
            .iter()
            .position(|x| *x == txid)
            .context("txid not found in Relay-final block")?;
        let key = hex::decode(&shop.shop_viewing_key)?;
        let machine = pubkey(&r.machine)?;
        let nonce = pubkey(&r.nonce)?;
        let parsed = zcash_primitives::transaction::Transaction::read(
            raw.data.as_slice(),
            zcash_protocol::consensus::BranchId::Nu7,
        )?;
        ensure!(
            *parsed.txid().as_ref() == txid,
            "raw transaction txid mismatch"
        );
        let bundle = parsed.ironwood_bundle().context("no Ironwood bundle")?;
        for index in 0..bundle.actions().len() {
            let Ok(payment) = decrypt_payment(&raw.data, &key, index as u32) else {
                continue;
            };
            let Ok(memo) = Memo::decode(&payment.memo) else {
                continue;
            };
            if memo.machine != machine || memo.nonce != nonce {
                continue;
            }
            let witness = Witness {
                tx: raw.data,
                shop_viewing_key: key,
                action_index: index as u32,
                machine,
                nonce,
                payout: memo.payout,
                diversifier: hex::decode(&shop.diversifier)?
                    .try_into()
                    .map_err(|_| anyhow!("diversifier"))?,
                tx_index: tx_index as u32,
                tx_count: txids.len() as u32,
                tx_path: zolar_claim::tx_merkle_path(&txids, tx_index),
                block_merkle_root: headers[leaf][36..68].try_into()?,
                height,
                tree_index: leaf as u32,
                tree_path: tree
                    .path(leaf, size)
                    .try_into()
                    .map_err(|_| anyhow!("tree depth"))?,
                root,
            };
            vending_guest::verify(&witness).map_err(|e| anyhow!("guest check: {e:?}"))?;
            let (latest, owner) = self.relay(&shop).await?;
            let live =
                RelayView::parse(&owner, &addr, &addr, &latest).map_err(|e| anyhow!("{e:?}"))?;
            ensure!(
                live.is_known_root(&root),
                "Relay root aged out during preparation; retry"
            );
            return Ok(witness);
        }
        bail!("no matching shop output and reservation memo")
    }
    async fn find(&self, r: &FindRequest) -> Result<Option<FoundPayment>> {
        r.validate()?;
        let shop = self.shops.shop(&r.machine).await?;
        shop.validate()?;
        let (data, owner) = self.relay(&shop).await?;
        let addr = pubkey(&shop.relay)?;
        let view = RelayView::parse(&owner, &addr, &addr, &data).map_err(|e| anyhow!("{e:?}"))?;
        let scan_start = if shop.scan_start == 0 {
            view.tip_height().saturating_sub(999)
        } else {
            shop.scan_start
        };
        ensure!(
            view.tip_height().saturating_sub(scan_start) < 1000,
            "scan range exceeds 1000 blocks; narrow public scan_start"
        );
        let key_bytes: [u8; 64] = hex::decode(&shop.shop_viewing_key)?
            .try_into()
            .map_err(|_| anyhow!("viewing key length"))?;
        let key = IncomingViewingKey::from_bytes(&key_bytes).unwrap();
        let d = hex::decode(&shop.diversifier)?;
        for height in (scan_start..=view.tip_height()).rev() {
            for tx in self.light.block(height).await?.vtx {
                let mut matches = false;
                for action in &tx.ironwood_actions {
                    if compact_matches(action, &key, &d)? {
                        matches = true;
                        break;
                    }
                }
                if !matches {
                    continue;
                }
                let internal: [u8; 32] = tx.txid.as_slice().try_into()?;
                let raw = self.light.transaction(internal).await?;
                if raw.height != height as u64 {
                    continue;
                }
                for index in 0..tx.ironwood_actions.len() {
                    let Ok(payment) = decrypt_payment(&raw.data, &key_bytes, index as u32) else {
                        continue;
                    };
                    if payment.txid != internal {
                        continue;
                    }
                    let Ok(memo) = Memo::decode(&payment.memo) else {
                        continue;
                    };
                    if memo.machine == pubkey(&r.machine)?
                        && memo.nonce == pubkey(&r.nonce)?
                        && payment.diversifier.as_slice() == d
                    {
                        let mut display = internal;
                        display.reverse();
                        return Ok(Some(FoundPayment {
                            txid: hex::encode(display),
                            height,
                            value_zat: payment.value_zat,
                            payout: bs58::encode(memo.payout).into_string(),
                        }));
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_scanning_binds_ironwood_recipient_and_diversifier() {
        let raw = hex::decode(include_str!("../../vending-guest/fixtures/ticket02-tx.hex").trim())
            .unwrap();
        let key_bytes: [u8; 64] =
            hex::decode(include_str!("../../vending-guest/fixtures/shop.ivk.hex").trim())
                .unwrap()
                .try_into()
                .unwrap();
        let key = IncomingViewingKey::from_bytes(&key_bytes).unwrap();
        let p = decrypt_payment(&raw, &key_bytes, 0).unwrap();
        let tx = zcash_primitives::transaction::Transaction::read(
            raw.as_slice(),
            zcash_protocol::consensus::BranchId::Nu7,
        )
        .unwrap();
        let a = &tx.ironwood_bundle().unwrap().actions()[0];
        let wire = wire::CompactAction {
            nullifier: a.nullifier().to_bytes().to_vec(),
            cmx: a.cmx().to_bytes().to_vec(),
            ephemeral_key: a.encrypted_note().epk_bytes.to_vec(),
            ciphertext: a.encrypted_note().enc_ciphertext.as_ref()[..52].to_vec(),
        };
        assert!(compact_matches(&wire, &key, &p.diversifier).unwrap());
        let mut d = p.diversifier;
        d[0] ^= 1;
        assert!(!compact_matches(&wire, &key, &d).unwrap());
        let mut bad = wire;
        bad.cmx[0] ^= 1;
        assert!(!compact_matches(&bad, &key, &p.diversifier).unwrap_or(false));
    }
    #[test]
    fn public_shop_metadata_refuses_a_mismatched_viewing_key_hash() {
        let key = include_str!("../../vending-guest/fixtures/shop.ivk.hex").trim();
        let mut shop = ShopConfig {
            machine: bs58::encode([2; 32]).into_string(),
            relay: bs58::encode([3; 32]).into_string(),
            shop_viewing_key: key.into(),
            svk_hash: "00".repeat(32),
            diversifier: "00".repeat(11),
            scan_start: 0,
        };
        assert!(shop.validate().is_err());
        shop.svk_hash = hex::encode(Sha256::digest(hex::decode(key).unwrap()));
        assert!(shop.validate().is_ok());
    }
}
