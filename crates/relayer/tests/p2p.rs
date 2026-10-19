use std::io::{Cursor, Read, Write};

use sha2::{Digest, Sha256};
use zolar_relayer::p2p::{Network, Peer, HEADER_LEN};

const TRANSCRIPT: &[u8] = include_bytes!("../fixtures/p2p-transcript.bin");
const FIXTURE: &str = include_str!("../../zcash-header/fixtures/testnet-nu7-4495261-4495660.txt");

/// What a live NU7 testnet peer sent us during one handshake and one `getheaders` request.
struct Recorded {
    input: Cursor<&'static [u8]>,
    output: Vec<u8>,
}

impl Read for Recorded {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buf)
    }
}

impl Write for Recorded {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn sha256d(data: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(data)).into()
}

fn locator_4495260() -> [u8; 32] {
    let mut hash: [u8; 32] =
        hex::decode("00000005c6568f0bc6f3ce796a6daf312edaeb01a846a387680ba28703f35ac5")
            .unwrap()
            .try_into()
            .unwrap();
    hash.reverse();
    hash
}

#[test]
fn a_recorded_peer_hands_over_the_headers_after_a_locator() {
    let stream = Recorded {
        input: Cursor::new(TRANSCRIPT),
        output: Vec::new(),
    };
    let mut peer =
        Peer::handshake(stream, Network::Testnet).expect("handshake with the recorded peer");
    assert!(peer.protocol_version() >= 170_180);

    let headers = peer.getheaders(&[locator_4495260()]).unwrap();

    assert_eq!(headers.len(), 160);
    let mut parent = locator_4495260();
    for (i, header) in headers.iter().enumerate() {
        assert_eq!(header.len(), HEADER_LEN);
        assert_eq!(
            header[4..36],
            parent,
            "header {i} links to the previous one"
        );
        parent = sha256d(header);
    }
    // The first headers equal the independently exported fixture of heights 4,495,261 onward.
    for (header, line) in headers.iter().zip(FIXTURE.lines()) {
        let (_, hex_header) = line.split_once(' ').unwrap();
        assert_eq!(hex::encode(header), hex_header);
    }
}

#[test]
fn the_getheaders_request_is_the_wire_format_peers_expect() {
    let stream = Recorded {
        input: Cursor::new(TRANSCRIPT),
        output: Vec::new(),
    };
    let mut peer = Peer::handshake(stream, Network::Testnet).unwrap();
    peer.getheaders(&[locator_4495260()]).unwrap();
    let sent = peer.into_stream().output;

    // The last message we sent: magic, "getheaders" padded to 12, length, checksum, payload.
    let payload_len = 4 + 1 + 32 + 32;
    let message = &sent[sent.len() - (24 + payload_len)..];
    assert_eq!(message[..4], [0xfa, 0x1a, 0xf9, 0xbf]);
    assert_eq!(&message[4..16], b"getheaders\0\0");
    assert_eq!(message[16..20], (payload_len as u32).to_le_bytes());
    let payload = &message[24..];
    assert_eq!(message[20..24], sha256d(payload)[..4]);
    assert_eq!(payload[..4], 170_180i32.to_le_bytes());
    assert_eq!(payload[4], 1);
    assert_eq!(payload[5..37], locator_4495260());
    assert_eq!(payload[37..], [0u8; 32]);
}

#[test]
fn a_corrupted_checksum_is_refused() {
    let mut bytes = TRANSCRIPT.to_vec();
    // First message is the peer's version: flip a payload byte.
    bytes[30] ^= 0xff;
    let stream = Recorded {
        input: Cursor::new(Box::leak(bytes.into_boxed_slice())),
        output: Vec::new(),
    };
    assert!(Peer::handshake(stream, Network::Testnet).is_err());
}

const MAINNET_TRANSCRIPT: &[u8] = include_bytes!("../fixtures/p2p-transcript-mainnet.bin");
const MAINNET_FIXTURE: &str =
    include_str!("../../zcash-header/fixtures/mainnet/mainnet-3522201-3522600.txt");
const MAINNET_LOCATOR_3522200: &str =
    "000000000035424cfdfeceb49ae71093095c2e760ef6700d38ed115841120ff5";

fn mainnet_stream(bytes: &'static [u8]) -> Recorded {
    Recorded {
        input: Cursor::new(bytes),
        output: Vec::new(),
    }
}

fn internal(hash: &str) -> [u8; 32] {
    let mut hash: [u8; 32] = hex::decode(hash).unwrap().try_into().unwrap();
    hash.reverse();
    hash
}

#[test]
fn a_recorded_mainnet_peer_hands_over_the_headers_after_a_locator() {
    let mut peer = Peer::handshake(mainnet_stream(MAINNET_TRANSCRIPT), Network::Mainnet)
        .expect("handshake with the recorded mainnet peer");
    assert!(peer.protocol_version() >= 170_100);

    let locator = internal(MAINNET_LOCATOR_3522200);
    let headers = peer.getheaders(&[locator]).unwrap();

    assert_eq!(headers.len(), 160);
    let mut parent = locator;
    for header in &headers {
        assert_eq!(header[4..36], parent);
        parent = sha256d(header);
    }
    // Independently exported fixture (two peers, Blockchair cross-check): heights 3,522,201 onward.
    for (header, line) in headers.iter().zip(MAINNET_FIXTURE.lines()) {
        let (_, hex_header) = line.split_once(' ').unwrap();
        assert_eq!(hex::encode(header), hex_header);
    }
}

#[test]
fn the_mainnet_request_carries_the_mainnet_magic() {
    let mut peer = Peer::handshake(mainnet_stream(MAINNET_TRANSCRIPT), Network::Mainnet).unwrap();
    peer.getheaders(&[internal(MAINNET_LOCATOR_3522200)])
        .unwrap();
    let sent = peer.into_stream().output;
    assert_eq!(sent[..4], [0x24, 0xe9, 0x27, 0x64]);
    assert_eq!(&sent[4..11], b"version");
    let last = &sent[sent.len() - (24 + 4 + 1 + 32 + 32)..];
    assert_eq!(last[..4], [0x24, 0xe9, 0x27, 0x64]);
    assert_eq!(&last[4..14], b"getheaders");
}

#[test]
fn a_testnet_connection_refuses_mainnet_bytes_and_the_other_way_round() {
    assert!(Peer::handshake(mainnet_stream(MAINNET_TRANSCRIPT), Network::Testnet).is_err());
    assert!(Peer::handshake(mainnet_stream(TRANSCRIPT), Network::Mainnet).is_err());
}

#[test]
fn each_network_has_its_own_port_seeders_and_checkpoint_fields() {
    assert_eq!(Network::Mainnet.port(), 8233);
    assert_eq!(Network::Testnet.port(), 18233);
    assert!(Network::Mainnet
        .seeders()
        .iter()
        .all(|s| !s.contains("testnet")));
    assert_eq!(Network::Mainnet.checkpoint_network(), 0);
    assert_eq!(Network::Mainnet.nu7_height(), u32::MAX);
    assert_eq!(Network::Testnet.checkpoint_network(), 1);
    assert_eq!(Network::Testnet.nu7_height(), 4_465_026);
    assert_eq!("mainnet".parse::<Network>().unwrap(), Network::Mainnet);
    assert!("regtest".parse::<Network>().is_err());
}

/// A peer that answers the handshake and then says nothing: what a Zebra peer does when it has no
/// header after the locator.
struct Silent {
    hello: Cursor<Vec<u8>>,
    kind: std::io::ErrorKind,
}

impl Read for Silent {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.hello.read(buf)? {
            0 => Err(self.kind.into()),
            n => Ok(n),
        }
    }
}

impl Write for Silent {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn silent_peer(kind: std::io::ErrorKind) -> Silent {
    // Everything before the headers reply of the recorded mainnet peer: version, verack, ...
    let mut offset = 0;
    loop {
        let len = u32::from_le_bytes(
            MAINNET_TRANSCRIPT[offset + 16..offset + 20]
                .try_into()
                .unwrap(),
        ) as usize;
        if &MAINNET_TRANSCRIPT[offset + 4..offset + 11] == b"headers" {
            break;
        }
        offset += 24 + len;
    }
    Silent {
        hello: Cursor::new(MAINNET_TRANSCRIPT[..offset].to_vec()),
        kind,
    }
}

#[test]
fn a_peer_that_stays_silent_after_getheaders_has_no_newer_header() {
    for kind in [std::io::ErrorKind::WouldBlock, std::io::ErrorKind::TimedOut] {
        let mut peer = Peer::handshake(silent_peer(kind), Network::Mainnet).unwrap();
        assert!(peer
            .getheaders(&[internal(MAINNET_LOCATOR_3522200)])
            .unwrap()
            .is_empty());
    }
}

#[test]
fn a_peer_that_hangs_up_after_getheaders_is_an_error() {
    let mut peer = Peer::handshake(
        silent_peer(std::io::ErrorKind::UnexpectedEof),
        Network::Mainnet,
    )
    .unwrap();
    assert!(peer
        .getheaders(&[internal(MAINNET_LOCATOR_3522200)])
        .is_err());
}

#[test]
fn a_peer_that_stalls_inside_a_message_is_an_error_not_silence() {
    // Ten bytes of a message header, then nothing: the stream is out of frame and must be dropped.
    let mut hello = silent_peer(std::io::ErrorKind::WouldBlock)
        .hello
        .into_inner();
    hello.extend_from_slice(&MAINNET_TRANSCRIPT[..10]);
    let stream = Silent {
        hello: Cursor::new(hello),
        kind: std::io::ErrorKind::WouldBlock,
    };
    let mut peer = Peer::handshake(stream, Network::Mainnet).unwrap();
    assert!(peer
        .getheaders(&[internal(MAINNET_LOCATOR_3522200)])
        .is_err());
}
