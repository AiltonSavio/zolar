//! The pool against local TCP servers that replay a recorded mainnet peer.
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener},
    thread,
    time::Duration,
};

use zolar_relayer::{
    follower::HeaderSource,
    p2p::{Network, HEADER_LEN},
    peers::PeerPool,
};

const MAINNET_TRANSCRIPT: &[u8] = include_bytes!("../fixtures/p2p-transcript-mainnet.bin");
const LOCATOR_3522200: &str = "000000000035424cfdfeceb49ae71093095c2e760ef6700d38ed115841120ff5";

/// Serves `connections` connections: the first `refuse` are closed at once, the rest get the
/// recorded peer's bytes.
fn serve(connections: usize, refuse: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for (i, stream) in listener.incoming().take(connections).enumerate() {
            let mut stream = stream.unwrap();
            if i < refuse {
                stream.shutdown(Shutdown::Both).ok();
            } else {
                stream.write_all(MAINNET_TRANSCRIPT).unwrap();
                thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    });
    address
}

fn locator() -> [u8; 32] {
    let mut hash: [u8; 32] = hex::decode(LOCATOR_3522200).unwrap().try_into().unwrap();
    hash.reverse();
    hash
}

#[test]
fn the_pool_serves_mainnet_headers_over_tcp() {
    let mut pool = PeerPool::new(vec![serve(1, 0)], Network::Mainnet);
    let batch = pool.headers_after(&[locator()]).unwrap();
    assert_eq!(batch.headers.len(), 160);
    assert_eq!(batch.headers[0].len(), HEADER_LEN);
}

#[test]
fn a_pool_whose_only_peer_just_failed_tries_it_again_instead_of_giving_up() {
    let mut pool = PeerPool::new(vec![serve(2, 1)], Network::Mainnet);
    assert!(pool.headers_after(&[locator()]).is_err());
    let batch = pool.headers_after(&[locator()]).unwrap();
    assert_eq!(batch.headers.len(), 160);
}

/// Splits the recorded bytes into the messages before the headers reply and the reply itself.
fn split_transcript() -> (Vec<u8>, Vec<u8>) {
    let mut offset = 0;
    loop {
        let len = u32::from_le_bytes(
            MAINNET_TRANSCRIPT[offset + 16..offset + 20]
                .try_into()
                .unwrap(),
        ) as usize;
        let end = offset + 24 + len;
        if &MAINNET_TRANSCRIPT[offset + 4..offset + 11] == b"headers" {
            return (
                MAINNET_TRANSCRIPT[..offset].to_vec(),
                MAINNET_TRANSCRIPT[offset..end].to_vec(),
            );
        }
        offset = end;
    }
}

/// Accepts the first connection and answers every `getheaders` on it; closes every later one.
fn serve_one_and_refuse_the_rest() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (hello, reply) = split_transcript();
    thread::spawn(move || {
        let mut first = true;
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            if !first {
                stream.shutdown(Shutdown::Both).ok();
                continue;
            }
            first = false;
            let reply = reply.clone();
            stream.write_all(&hello).unwrap();
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if buf[..n].windows(10).any(|w| w == b"getheaders") {
                        stream.write_all(&reply).unwrap();
                    }
                }
            });
        }
    });
    address
}

#[test]
fn a_working_peer_is_kept_when_no_replacement_can_be_found() {
    let mut pool = PeerPool::new(vec![serve_one_and_refuse_the_rest()], Network::Mainnet)
        .rotating_after(Duration::ZERO);
    assert_eq!(pool.headers_after(&[locator()]).unwrap().headers.len(), 160);
    // Rotation is due, the replacement is refused, and the old connection still serves.
    assert_eq!(pool.headers_after(&[locator()]).unwrap().headers.len(), 160);
    assert_eq!(pool.headers_after(&[locator()]).unwrap().headers.len(), 160);
}
