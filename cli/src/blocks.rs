//! Block data the Relay does not keep: the transaction Merkle roots of every final block (to rebuild
//! the Final Block Tree) and the transaction IDs of the payment's block (to build the Inclusion Proof).
//!
//! Where it comes from, and why:
//!
//! * Merkle roots: Zcash P2P `getheaders` against live testnet peers. A header carries its Merkle root,
//!   headers are cheap (160 per request, no key, no account) and are the same bytes the Relayer sent
//!   to the Header Relay. The rebuilt tree must reproduce the Relay's own root, which pins every root.
//! * Transaction IDs: the explorer, because the Relay needs the whole block's txid list and a header
//!   has none. It is untrusted: the list is accepted only if its Merkle root equals the header's.
//!   `getdata` for the full block over P2P would remove the explorer, but the txid of an Ironwood (v6)
//!   transaction is a ZIP 244 digest that needs librustzcash to parse every transaction.

use std::net::SocketAddr;

use anyhow::{Result, bail, ensure};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;
use zolar_relayer::p2p::{self, Peer, RawHeader};

#[cfg(test)]
use crate::internal_txid;

/// Offset of the Merkle root in a raw header.
const MERKLE_ROOT: std::ops::Range<usize> = 36..68;
const PREVIOUS_HASH: std::ops::Range<usize> = 4..36;

pub fn header_hash(header: &RawHeader) -> [u8; 32] {
    p2p::sha256d(header)
}

pub fn merkle_root(header: &RawHeader) -> [u8; 32] {
    header[MERKLE_ROOT].try_into().expect("32 bytes")
}

/// Checks that `headers` follow `parent` one after the other.
fn links(parent: &[u8; 32], headers: &[RawHeader]) -> bool {
    let mut parent = *parent;
    for header in headers {
        if header[PREVIOUS_HASH] != parent {
            return false;
        }
        parent = header_hash(header);
    }
    true
}

/// The `count` headers after the block with hash `start` (internal byte order), from live peers.
pub fn fetch_headers(start: [u8; 32], count: usize, hosts: &[String]) -> Result<Vec<RawHeader>> {
    let mut addresses: Vec<SocketAddr> = p2p::resolve(hosts);
    addresses.sort();
    addresses.dedup();
    ensure!(!addresses.is_empty(), "no Zcash testnet peers found");
    let mut out: Vec<RawHeader> = Vec::with_capacity(count);
    let mut tip = start;
    let mut peer: Option<Peer<std::net::TcpStream>> = None;
    let mut next = 0usize;
    let mut failures = 0;
    while out.len() < count {
        let Some(connected) = peer.as_mut() else {
            if failures > 3 * addresses.len().max(10) {
                bail!("could not read {count} headers from the Zcash testnet peers");
            }
            let addr = addresses[next % addresses.len()];
            next += 1;
            match Peer::connect(addr) {
                Ok(p) => peer = Some(p),
                Err(_) => failures += 1,
            }
            continue;
        };
        match connected.getheaders(&[tip]) {
            Ok(headers) if !headers.is_empty() && links(&tip, &headers) => {
                tip = header_hash(headers.last().expect("not empty"));
                out.extend(headers);
            }
            _ => {
                // A peer on a stale branch, a peer that knows nothing after `tip`, or a dropped
                // connection: try the next one.
                failures += 1;
                peer = None;
            }
        }
    }
    out.truncate(count);
    Ok(out)
}

fn read_compact_size(r: &mut impl std::io::Read) -> Result<u64> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first)?;
    Ok(match first[0] {
        0xfd => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            u16::from_le_bytes(b) as u64
        }
        0xfe => {
            let mut b = [0u8; 4];
            r.read_exact(&mut b)?;
            u32::from_le_bytes(b) as u64
        }
        0xff => bail!("absurd transaction count"),
        n => n as u64,
    })
}

/// The transaction IDs of a raw `block` message payload, in block order, internal byte order, and the
/// header it carries. Every transaction is parsed, so its ID is computed from the bytes (ZIP 244 for
/// v5 and v6) rather than read from anyone's list.
pub fn parse_block(raw: &[u8]) -> Result<(RawHeader, Vec<[u8; 32]>)> {
    ensure!(
        raw.len() > p2p::HEADER_LEN,
        "block is shorter than its header"
    );
    let header: RawHeader = raw[..p2p::HEADER_LEN].try_into().expect("length checked");
    ensure!(
        header[140..143] == [0xfd, 0x40, 0x05],
        "header has no 1,344 byte solution"
    );
    let mut rest: &[u8] = &raw[p2p::HEADER_LEN..];
    let count = read_compact_size(&mut rest)?;
    ensure!(count > 0, "block has no transactions");
    let mut txids = Vec::new();
    for _ in 0..count {
        let tx = Transaction::read(&mut rest, BranchId::Nu7)
            .map_err(|e| anyhow::anyhow!("transaction does not parse: {e}"))?;
        txids.push(*tx.txid().as_ref());
    }
    ensure!(rest.is_empty(), "bytes after the last transaction");
    Ok((header, txids))
}

/// Fetches the block with `hash` (internal byte order) from Zcash P2P peers and returns its transaction
/// IDs. The header must hash to `hash` and the transactions to the header's Merkle root, so a peer
/// cannot lie. Nothing about the Payer's payment is sent: the request names a block hash, which any
/// syncing node asks for.
pub fn fetch_block_txids(hash: [u8; 32], hosts: &[String]) -> Result<Vec<[u8; 32]>> {
    let mut addresses: Vec<SocketAddr> = p2p::resolve(hosts);
    addresses.sort();
    addresses.dedup();
    ensure!(!addresses.is_empty(), "no Zcash testnet peers found");
    for addr in addresses.iter().take(12) {
        let Ok(mut peer) = Peer::connect(*addr) else {
            continue;
        };
        let Ok(raw) = peer.getdata_block(&hash) else {
            continue;
        };
        let Ok((header, txids)) = parse_block(&raw) else {
            continue;
        };
        if header_hash(&header) == hash
            && zolar_claim::tx_merkle_root(&txids) == merkle_root(&header)
        {
            return Ok(txids);
        }
    }
    bail!(
        "no Zcash testnet peer returned the block {}",
        hex::encode(hash)
    )
}

/// Where a payment was found.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    /// Position of the block among the candidates.
    pub candidate: usize,
    /// All transaction IDs of the block, internal byte order.
    pub txids: Vec<[u8; 32]>,
    /// Position of the payment in the block.
    pub tx_index: usize,
}

/// Looks for `txid` in the blocks with the given hashes, in order, using `fetch` to get each block's
/// transaction IDs. The Payer finds its own payment this way, locally, without telling an explorer
/// which transaction or which height it cares about.
pub fn find_payment(
    hashes: &[[u8; 32]],
    txid: &[u8; 32],
    mut fetch: impl FnMut(&[u8; 32]) -> Result<Vec<[u8; 32]>>,
) -> Result<Option<Found>> {
    for (candidate, hash) in hashes.iter().enumerate() {
        let txids = fetch(hash)?;
        if let Some(tx_index) = txids.iter().position(|t| t == txid) {
            return Ok(Some(Found {
                candidate,
                txids,
                tx_index,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(n: usize) -> Vec<RawHeader> {
        let mut parent = [7u8; 32];
        (0..n)
            .map(|i| {
                let mut h = [0u8; p2p::HEADER_LEN];
                h[PREVIOUS_HASH].copy_from_slice(&parent);
                h[MERKLE_ROOT].copy_from_slice(&[i as u8; 32]);
                parent = header_hash(&h);
                h
            })
            .collect()
    }

    const BLOCK: &[u8] = include_bytes!("../fixtures/block-4517478.bin");

    /// Transaction IDs of block 4,517,478 as the explorer printed them (display order), taken from the
    /// ticket 10 circuit fixture: an independent source for what the parser computes from raw bytes.
    fn expected_txids() -> Vec<[u8; 32]> {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../circuits/fixtures/swap2-block.json")).unwrap();
        fixture["txids_display"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| internal_txid(t.as_str().unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn a_raw_testnet_block_yields_the_transaction_ids_the_explorer_lists() {
        let (header, txids) = parse_block(BLOCK).unwrap();
        assert_eq!(txids, expected_txids());
        assert_eq!(zolar_claim::tx_merkle_root(&txids), merkle_root(&header));
        assert_eq!(
            hex::encode(
                header_hash(&header)
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
            ),
            "000027af3c4bf4943313d7766771d1435fec0dbbd4d858d1e3e9fbb7c9deac6f"
        );
    }

    #[test]
    fn a_truncated_or_padded_block_is_refused() {
        assert!(parse_block(&BLOCK[..BLOCK.len() - 1]).is_err());
        let mut padded = BLOCK.to_vec();
        padded.push(0);
        assert!(parse_block(&padded).is_err());
        assert!(parse_block(&BLOCK[..100]).is_err());
    }

    #[test]
    fn the_payment_is_found_in_the_block_that_holds_it_and_not_before() {
        let txids = expected_txids();
        let payment = txids[3];
        let hashes = [[1u8; 32], [2u8; 32], [3u8; 32]];
        let mut asked = Vec::new();
        let found = find_payment(&hashes, &payment, |h| {
            asked.push(h[0]);
            Ok(if h[0] == 2 {
                txids.clone()
            } else {
                vec![[9; 32]]
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!((found.candidate, found.tx_index), (1, 3));
        assert_eq!(asked, vec![1, 2], "stops at the block that holds it");
        let none = find_payment(&hashes, &[5; 32], |_| Ok(vec![[9; 32]])).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn links_accepts_only_a_connected_chain() {
        let headers = chain(4);
        assert!(links(&[7u8; 32], &headers));
        assert!(!links(&[8u8; 32], &headers));
        let mut broken = headers.clone();
        broken.swap(1, 2);
        assert!(!links(&[7u8; 32], &broken));
        assert_eq!(merkle_root(&headers[2]), [2u8; 32]);
    }
}
