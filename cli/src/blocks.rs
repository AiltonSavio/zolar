//! Block data the Relay does not keep: the transaction Merkle roots of every final block (to rebuild
//! the Final Block Tree) and the transaction IDs of the payment's block (to build the Inclusion Proof).
//!
//! Where it comes from, and why:
//!
//! * Merkle roots: Zcash P2P `getheaders` against live peers of the chosen network. A header carries its Merkle root,
//!   headers are cheap (160 per request, no key, no account) and are the same bytes the Relayer sent
//!   to the Header Relay. The rebuilt tree must reproduce the Relay's own root, which pins every root.
//! * Transaction IDs: the full block over P2P `getdata`, requested by block hash. Every transaction is
//!   parsed with librustzcash to compute its ZIP 244 ID from the bytes, and the list is accepted only
//!   if its Merkle root equals the header's. No explorer is asked.

use std::net::SocketAddr;

use anyhow::{Result, bail, ensure};
use rand::seq::SliceRandom;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;
use zolar_relayer::p2p::{self, Network, Peer, RawHeader};

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
    // A new random order on every call, so a retry after a bad answer asks other peers first.
    addresses.shuffle(&mut rand::thread_rng());
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
pub fn fetch_block_txids(
    hash: [u8; 32],
    hosts: &[String],
    network: Network,
) -> Result<Vec<[u8; 32]>> {
    let mut addresses: Vec<SocketAddr> = p2p::resolve(hosts, network);
    addresses.sort();
    addresses.dedup();
    ensure!(!addresses.is_empty(), "no Zcash {network:?} peers found");
    // A random order spreads the load, and many peers refuse blocks or are gone.
    addresses.shuffle(&mut rand::thread_rng());
    let started = std::time::Instant::now();
    for addr in addresses.iter().take(40) {
        // Each peer read is bounded, but 40 slow peers add up: give up on this block for now.
        if started.elapsed() > std::time::Duration::from_secs(90) {
            break;
        }
        let Ok(mut peer) = Peer::connect(*addr, network) else {
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
        "no Zcash {network:?} peer returned the block {}",
        hex::encode(hash)
    )
}

/// A connection retained across a scan only for an explicitly configured peer pool.
/// Every response is checked against the current requested hash, never a previous response.
/// Default DNS discovery continues to choose a random peer independently for each block.
pub struct BlockSource {
    addresses: Vec<SocketAddr>,
    next: usize,
    network: Network,
    peer: Option<Peer<std::net::TcpStream>>,
}
impl BlockSource {
    pub fn new(hosts: &[String], network: Network) -> Result<Self> {
        let mut addresses = p2p::resolve(hosts, network);
        addresses.sort();
        addresses.dedup();
        addresses.shuffle(&mut rand::thread_rng());
        addresses.truncate(40);
        ensure!(!addresses.is_empty(), "no configured Zcash peers resolved");
        Ok(Self {
            addresses,
            next: 0,
            network,
            peer: None,
        })
    }
    pub fn fetch(&mut self, hash: &[u8; 32]) -> Result<Vec<[u8; 32]>> {
        let started = std::time::Instant::now();
        for _ in 0..self.addresses.len() {
            if started.elapsed() > std::time::Duration::from_secs(90) {
                break;
            }
            let mut peer = if let Some(peer) = self.peer.take() {
                peer
            } else {
                let address = self.addresses[self.next % self.addresses.len()];
                self.next += 1;
                let Ok(peer) = Peer::connect(address, self.network) else {
                    continue;
                };
                peer
            };
            if let Ok(txids) = peer
                .getdata_block(hash)
                .map_err(anyhow::Error::from)
                .and_then(|raw| verified_block_txids(&raw, hash))
            {
                self.peer = Some(peer);
                return Ok(txids);
            }
            // Drop the connection after transport, parsing or identity failure. The caller keeps
            // its scan position and can reconnect on a later bounded retry.
        }
        bail!("configured peers did not return the requested verified block")
    }
}
fn verified_block_txids(raw: &[u8], hash: &[u8; 32]) -> Result<Vec<[u8; 32]>> {
    let (header, txids) = parse_block(raw)?;
    ensure!(
        header_hash(&header) == *hash,
        "peer returned a different block"
    );
    ensure!(
        zolar_claim::tx_merkle_root(&txids) == merkle_root(&header),
        "block transactions differ from its Merkle root"
    );
    Ok(txids)
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

/// What a look through some blocks found.
#[derive(Debug)]
pub struct Scan {
    pub found: Option<Found>,
    /// How many of the blocks, from the first, were read and do not hold the payment (or do, when found).
    pub scanned: usize,
    /// Why the look stopped early: a block that no peer would give. Try again later.
    pub failure: Option<String>,
}

/// Looks for `txid` in the blocks with the given hashes, in order, using `fetch` to get each block's
/// transaction IDs, and stops at the first block `fetch` cannot give. The Payer finds its own payment
/// this way, locally, without telling an explorer which transaction or which height it cares about.
pub fn find_payment(
    hashes: &[[u8; 32]],
    txid: &[u8; 32],
    mut fetch: impl FnMut(&[u8; 32]) -> Result<Vec<[u8; 32]>>,
) -> Scan {
    for (candidate, hash) in hashes.iter().enumerate() {
        match fetch(hash) {
            Ok(txids) => {
                if let Some(tx_index) = txids.iter().position(|t| t == txid) {
                    return Scan {
                        found: Some(Found {
                            candidate,
                            txids,
                            tx_index,
                        }),
                        scanned: candidate + 1,
                        failure: None,
                    };
                }
            }
            Err(e) => {
                return Scan {
                    found: None,
                    scanned: candidate,
                    failure: Some(e.to_string()),
                };
            }
        }
    }
    Scan {
        found: None,
        scanned: hashes.len(),
        failure: None,
    }
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

    fn accept_test_peer(listener: &std::net::TcpListener) -> std::net::TcpStream {
        use std::io::Write;
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        assert_eq!(
            p2p::read_message(Network::Testnet, &mut stream).unwrap().0,
            "version"
        );
        stream
            .write_all(&p2p::encode_message(
                Network::Testnet,
                "version",
                &p2p::PROTOCOL.to_le_bytes(),
            ))
            .unwrap();
        stream
            .write_all(&p2p::encode_message(Network::Testnet, "verack", &[]))
            .unwrap();
        assert_eq!(
            p2p::read_message(Network::Testnet, &mut stream).unwrap().0,
            "verack"
        );
        stream
    }
    #[test]
    fn configured_scan_reuses_a_connection_but_rejects_a_previous_block_for_the_next_hash() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let mut stream = accept_test_peer(&listener);
            let mut requested = Vec::new();
            for _ in 0..3 {
                let (command, payload) = p2p::read_message(Network::Testnet, &mut stream).unwrap();
                assert_eq!(command, "getdata");
                requested.push(payload[payload.len() - 32..].to_vec());
                stream
                    .write_all(&p2p::encode_message(Network::Testnet, "block", BLOCK))
                    .unwrap();
            }
            requested
        });
        let (header, _) = parse_block(BLOCK).unwrap();
        let hash = header_hash(&header);
        let mut source = BlockSource::new(&[host], Network::Testnet).unwrap();
        assert_eq!(source.fetch(&hash).unwrap(), expected_txids());
        let other = [19; 32];
        let scan = find_payment(&[hash, other], &[27; 32], |hash| source.fetch(hash));
        assert_eq!(
            scan.scanned, 1,
            "a substituted block must not advance the scan"
        );
        assert!(scan.failure.is_some());
        assert!(source.peer.is_none());
        let requests = server.join().unwrap();
        assert_eq!(requests, [hash.to_vec(), hash.to_vec(), other.to_vec()]);
    }
    #[test]
    fn a_truncated_peer_reply_is_dropped_and_the_same_block_can_be_retried_on_a_new_connection() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            for truncated in [true, false] {
                let mut stream = accept_test_peer(&listener);
                assert_eq!(
                    p2p::read_message(Network::Testnet, &mut stream).unwrap().0,
                    "getdata"
                );
                let reply = p2p::encode_message(Network::Testnet, "block", BLOCK);
                stream
                    .write_all(if truncated { &reply[..50] } else { &reply })
                    .unwrap();
            }
        });
        let hash = header_hash(&parse_block(BLOCK).unwrap().0);
        let mut source = BlockSource::new(&[host], Network::Testnet).unwrap();
        assert!(source.fetch(&hash).is_err());
        assert!(source.peer.is_none());
        assert_eq!(source.fetch(&hash).unwrap(), expected_txids());
        server.join().unwrap();
    }
    #[test]
    fn block_identity_and_transaction_merkle_root_are_independent_checks() {
        let hash = header_hash(&parse_block(BLOCK).unwrap().0);
        assert!(verified_block_txids(BLOCK, &hash).is_ok());
        assert!(verified_block_txids(BLOCK, &[19; 32]).is_err());
        let mut forged = BLOCK.to_vec();
        forged[36] ^= 1;
        let forged_hash = header_hash(&parse_block(&forged).unwrap().0);
        assert!(verified_block_txids(&forged, &forged_hash).is_err());
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
        .found
        .unwrap();
        assert_eq!((found.candidate, found.tx_index), (1, 3));
        assert_eq!(asked, vec![1, 2], "stops at the block that holds it");
        let none = find_payment(&hashes, &[5; 32], |_| Ok(vec![[9; 32]]));
        assert!(none.found.is_none() && none.failure.is_none());
        assert_eq!(none.scanned, 3);
    }

    #[test]
    fn a_block_no_peer_gives_stops_the_look_and_keeps_what_was_read() {
        let hashes = [[1u8; 32], [2u8; 32], [3u8; 32]];
        let scan = find_payment(&hashes, &[5; 32], |h| {
            if h[0] == 2 {
                bail!("no peer has it")
            }
            Ok(vec![[9; 32]])
        });
        assert!(scan.found.is_none());
        assert_eq!(scan.scanned, 1, "the first block is not read again");
        assert!(scan.failure.unwrap().contains("no peer"));
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
