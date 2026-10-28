//! Shared Zcash P2P implementation; this keeps the Relayer and prover's wire checks identical.
#[path = "../../relayer/src/p2p.rs"]
pub mod p2p;
use anyhow::{bail, ensure, Result};
use p2p::{Network, Peer, RawHeader, HEADER_LEN};
use rand::seq::SliceRandom;
use std::io::Read;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

fn peers(hosts: &[String]) -> Vec<std::net::SocketAddr> {
    let mut addresses = p2p::resolve(hosts, Network::Testnet);
    addresses.sort();
    addresses.dedup();
    addresses.shuffle(&mut rand::thread_rng());
    addresses
}
pub fn fetch_headers(start: [u8; 32], count: usize, hosts: &[String]) -> Result<Vec<RawHeader>> {
    let mut headers = Vec::new();
    let mut parent = start;
    for addr in peers(hosts).into_iter().take(12) {
        let Ok(mut peer) = Peer::connect(addr, Network::Testnet) else {
            continue;
        };
        while headers.len() < count {
            let Ok(batch) = peer.getheaders(&[parent]) else {
                break;
            };
            if batch.is_empty() {
                break;
            }
            let mut linked = true;
            for header in batch.into_iter().take(count - headers.len()) {
                if header[4..36] != parent {
                    linked = false;
                    break;
                }
                parent = p2p::sha256d(&header);
                headers.push(header);
            }
            if !linked {
                break;
            }
        }
        if headers.len() == count {
            return Ok(headers);
        }
    }
    if count == 0 {
        return Ok(headers);
    }
    bail!(
        "P2P could not fetch all {count} Relay headers (got {})",
        headers.len()
    )
}

pub fn parse_block(raw: &[u8]) -> Result<(RawHeader, Vec<[u8; 32]>)> {
    ensure!(raw.len() > HEADER_LEN, "short block");
    let header: RawHeader = raw[..HEADER_LEN].try_into()?;
    ensure!(
        header[140..143] == [0xfd, 0x40, 0x05],
        "invalid header solution length"
    );
    let mut rest = &raw[HEADER_LEN..];
    let mut first = [0];
    rest.read_exact(&mut first)?;
    let count = match first[0] {
        0xfd => {
            let mut b = [0; 2];
            rest.read_exact(&mut b)?;
            let n = u16::from_le_bytes(b) as usize;
            ensure!(n >= 253, "noncanonical count");
            n
        }
        0xfe => {
            let mut b = [0; 4];
            rest.read_exact(&mut b)?;
            let n = u32::from_le_bytes(b) as usize;
            ensure!(n > 65535, "noncanonical count");
            n
        }
        0xff => bail!("absurd transaction count"),
        n => n as usize,
    };
    ensure!(
        count > 0 && count <= raw.len() / 10,
        "invalid transaction count"
    );
    let mut txids = Vec::new();
    for _ in 0..count {
        let tx = Transaction::read(&mut rest, BranchId::Nu7)?;
        txids.push(*tx.txid().as_ref());
    }
    ensure!(rest.is_empty(), "trailing block data");
    ensure!(
        zolar_claim::tx_merkle_root(&txids) == header[36..68],
        "block Merkle root mismatch"
    );
    Ok((header, txids))
}
pub fn fetch_txids(hash: [u8; 32], hosts: &[String]) -> Result<Vec<[u8; 32]>> {
    for addr in peers(hosts).into_iter().take(12) {
        let Ok(mut peer) = Peer::connect(addr, Network::Testnet) else {
            continue;
        };
        let Ok(raw) = peer.getdata_block(&hash) else {
            continue;
        };
        let Ok((header, txids)) = parse_block(&raw) else {
            continue;
        };
        if p2p::sha256d(&header) == hash {
            return Ok(txids);
        }
    }
    bail!("no peer returned the payment block")
}
