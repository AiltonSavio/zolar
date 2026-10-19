//! Block data the Relay does not keep: the transaction Merkle roots of every final block (to rebuild
//! the Final Block Tree) and the transaction IDs of the payment's block (to build the Inclusion Proof).
//!
//! Where it comes from, and why:
//!
//! * Merkle roots: Zcash P2P `getheaders` against live peers of the chosen network. A header carries its Merkle root,
//!   headers are cheap (160 per request, no key, no account) and are the same bytes the Relayer sent
//!   to the Header Relay. The rebuilt tree must reproduce the Relay's own root, which pins every root.
//! * Transaction IDs: the explorer, because the Relay needs the whole block's txid list and a header
//!   has none. It is untrusted: the list is accepted only if its Merkle root equals the header's.
//!   `getdata` for the full block over P2P would remove the explorer, but the txid of an Ironwood (v6)
//!   transaction is a ZIP 244 digest that needs librustzcash to parse every transaction.

use std::net::SocketAddr;

use anyhow::{Result, bail, ensure};
use zolar_relayer::p2p::{self, Network, Peer, RawHeader};

use crate::{explorer::Explorer, internal_txid};

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
pub fn fetch_headers(
    start: [u8; 32],
    count: usize,
    hosts: &[String],
    network: Network,
) -> Result<Vec<RawHeader>> {
    let mut addresses: Vec<SocketAddr> = p2p::resolve(hosts, network);
    addresses.sort();
    addresses.dedup();
    ensure!(!addresses.is_empty(), "no Zcash {network:?} peers found");
    let mut out: Vec<RawHeader> = Vec::with_capacity(count);
    let mut tip = start;
    let mut peer: Option<Peer<std::net::TcpStream>> = None;
    let mut next = 0usize;
    let mut failures = 0;
    while out.len() < count {
        let Some(connected) = peer.as_mut() else {
            if failures > 3 * addresses.len().max(10) {
                bail!("could not read {count} headers from the Zcash {network:?} peers");
            }
            let addr = addresses[next % addresses.len()];
            next += 1;
            match Peer::connect(addr, network) {
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

/// The transaction IDs of the block at `height` in internal byte order, accepted only when their
/// Merkle root equals `expected_root` (from the block's header).
pub fn verified_block_txids(
    explorer: &Explorer,
    height: u32,
    expected_root: &[u8; 32],
) -> Result<Vec<[u8; 32]>> {
    let txids: Vec<[u8; 32]> = explorer
        .block_txids(height)?
        .iter()
        .map(|t| internal_txid(t))
        .collect::<Result<_>>()?;
    ensure!(!txids.is_empty(), "block {height} has no transactions");
    let root = zolar_claim::tx_merkle_root(&txids);
    ensure!(
        &root == expected_root,
        "the explorer's transaction list for block {height} does not hash to the block header's Merkle root"
    );
    Ok(txids)
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
