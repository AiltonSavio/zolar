//! The slice of the Zcash peer-to-peer protocol the Relayer needs: handshake and `getheaders`.
use std::{
    fmt,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rand::Rng;
use sha2::{Digest, Sha256};

/// Protocol version of NU7; peers older than this are on the stale pre-NU7 branch.
pub const PROTOCOL: i32 = 170_180;
/// 140 byte header, `fd4005`, 1,344 byte Equihash solution.
pub const HEADER_LEN: usize = 1487;
const TESTNET_SEEDERS: [&str; 3] = [
    "testnet.seeder.zfnd.org",
    "seeder.testnet.zec.rocks",
    "dnsseed.testnet.z.cash",
];
const MAINNET_SEEDERS: [&str; 3] = [
    "mainnet.seeder.zfnd.org",
    "seeder.mainnet.zec.rocks",
    "dnsseed.z.cash",
];
/// Mainnet has no NU7 yet, so any peer on the Canopy-or-later chain is on the chain we follow.
const MAINNET_MIN_PROTOCOL: i32 = 170_100;
/// The NU7 activation height on the Zcash testnet.
const TESTNET_NU7_HEIGHT: u32 = 4_465_026;
const MAX_HANDSHAKE_MESSAGES: usize = 50;
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
const USER_AGENT: &[u8] = b"/zolar-relayer:0.1/";

/// The Zcash network a Relayer, a peer connection or a Checkpoint belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Testnet,
    Mainnet,
}

impl Network {
    pub fn magic(self) -> [u8; 4] {
        match self {
            Self::Testnet => [0xfa, 0x1a, 0xf9, 0xbf],
            Self::Mainnet => [0x24, 0xe9, 0x27, 0x64],
        }
    }

    pub fn port(self) -> u16 {
        match self {
            Self::Testnet => 18233,
            Self::Mainnet => 8233,
        }
    }

    pub fn seeders(self) -> &'static [&'static str] {
        match self {
            Self::Testnet => &TESTNET_SEEDERS,
            Self::Mainnet => &MAINNET_SEEDERS,
        }
    }

    /// The oldest peer protocol version that follows the chain we want.
    pub fn min_peer_protocol(self) -> i32 {
        match self {
            Self::Testnet => PROTOCOL,
            Self::Mainnet => MAINNET_MIN_PROTOCOL,
        }
    }

    /// The `network` byte of a Header Relay Checkpoint.
    pub fn checkpoint_network(self) -> u8 {
        match self {
            Self::Mainnet => 0,
            Self::Testnet => 1,
        }
    }

    /// The NU7 activation height; `u32::MAX` while mainnet has none (the relay then keeps the
    /// 17-block difficulty window forever, so a new mainnet relay is needed after activation).
    pub fn nu7_height(self) -> u32 {
        match self {
            Self::Testnet => TESTNET_NU7_HEIGHT,
            Self::Mainnet => u32::MAX,
        }
    }
}

impl std::str::FromStr for Network {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "testnet" => Ok(Self::Testnet),
            "mainnet" => Ok(Self::Mainnet),
            other => Err(format!("unknown network {other}: use testnet or mainnet")),
        }
    }
}

pub type RawHeader = [u8; HEADER_LEN];

#[derive(Debug)]
pub enum P2pError {
    Io(io::Error),
    Protocol(String),
}

impl fmt::Display for P2pError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "i/o: {e}"),
            Self::Protocol(m) => write!(f, "protocol: {m}"),
        }
    }
}

impl std::error::Error for P2pError {}

impl From<io::Error> for P2pError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn protocol<T>(message: impl Into<String>) -> Result<T, P2pError> {
    Err(P2pError::Protocol(message.into()))
}

pub fn sha256d(data: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(data)).into()
}

fn varint(n: usize) -> Vec<u8> {
    match n {
        0..=252 => vec![n as u8],
        253..=0xffff => [&[0xfd][..], &(n as u16).to_le_bytes()].concat(),
        _ => [&[0xfe][..], &(n as u32).to_le_bytes()].concat(),
    }
}

fn read_varint(data: &[u8]) -> Result<(usize, usize), P2pError> {
    match data.first() {
        None => protocol("empty varint"),
        Some(&b) if b < 253 => Ok((b as usize, 1)),
        Some(253) if data.len() >= 3 => Ok((u16::from_le_bytes([data[1], data[2]]) as usize, 3)),
        Some(254) if data.len() >= 5 => Ok((
            u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize,
            5,
        )),
        Some(_) => protocol("truncated varint"),
    }
}

pub fn encode_message(network: Network, command: &str, payload: &[u8]) -> Vec<u8> {
    let mut name = [0u8; 12];
    name[..command.len()].copy_from_slice(command.as_bytes());
    let mut out = Vec::with_capacity(24 + payload.len());
    out.extend_from_slice(&network.magic());
    out.extend_from_slice(&name);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&sha256d(payload)[..4]);
    out.extend_from_slice(payload);
    out
}

pub fn read_message(network: Network, r: &mut impl Read) -> Result<(String, Vec<u8>), P2pError> {
    let mut head = [0u8; 24];
    r.read_exact(&mut head)?;
    if head[..4] != network.magic() {
        return protocol("wrong network magic");
    }
    let len = u32::from_le_bytes(head[16..20].try_into().unwrap()) as usize;
    if len > MAX_PAYLOAD {
        return protocol(format!("payload of {len} bytes"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    if sha256d(&payload)[..4] != head[20..24] {
        return protocol("bad checksum");
    }
    let command = String::from_utf8_lossy(&head[4..16])
        .trim_end_matches('\0')
        .to_string();
    Ok((command, payload))
}

fn netaddr() -> Vec<u8> {
    let mut out = vec![0u8; 8];
    out.extend_from_slice(&[0u8; 10]);
    out.extend_from_slice(&[0xff, 0xff, 0, 0, 0, 0, 0, 0]);
    out
}

fn version_payload(now: i64, nonce: u64) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&PROTOCOL.to_le_bytes());
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&now.to_le_bytes());
    p.extend_from_slice(&netaddr());
    p.extend_from_slice(&netaddr());
    p.extend_from_slice(&nonce.to_le_bytes());
    p.extend_from_slice(&varint(USER_AGENT.len()));
    p.extend_from_slice(USER_AGENT);
    p.extend_from_slice(&0i32.to_le_bytes());
    p.push(0);
    p
}

/// Locator hashes are in internal byte order, newest first; a zero stop hash asks for up to 160 headers.
fn getheaders_payload(locator: &[[u8; 32]]) -> Vec<u8> {
    let mut p = PROTOCOL.to_le_bytes().to_vec();
    p.extend_from_slice(&varint(locator.len()));
    for hash in locator {
        p.extend_from_slice(hash);
    }
    p.extend_from_slice(&[0u8; 32]);
    p
}

pub fn parse_headers(payload: &[u8]) -> Result<Vec<RawHeader>, P2pError> {
    let (count, mut offset) = read_varint(payload)?;
    if payload.len() != offset + count * (HEADER_LEN + 1) {
        return protocol("headers message has the wrong length");
    }
    let mut headers = Vec::with_capacity(count);
    for _ in 0..count {
        let header: RawHeader = payload[offset..offset + HEADER_LEN].try_into().unwrap();
        if payload[offset + HEADER_LEN] != 0 {
            return protocol("header carries transactions");
        }
        if header[140..143] != [0xfd, 0x40, 0x05] {
            return protocol("header has no 1,344 byte solution");
        }
        headers.push(header);
        offset += HEADER_LEN + 1;
    }
    Ok(headers)
}

/// A handshaken connection to one peer.
pub struct Peer<S> {
    stream: S,
    network: Network,
    version: i32,
}

impl<S: Read + Write> Peer<S> {
    /// Exchanges `version` and `verack`; refuses peers older than the network's chain needs.
    pub fn handshake(mut stream: S, network: Network) -> Result<Self, P2pError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let nonce = rand::thread_rng().gen();
        stream.write_all(&encode_message(
            network,
            "version",
            &version_payload(now, nonce),
        ))?;
        let (mut got_verack, mut version) = (false, None);
        let mut messages = 0;
        while !(got_verack && version.is_some()) {
            messages += 1;
            if messages > MAX_HANDSHAKE_MESSAGES {
                return protocol("peer never finished the handshake");
            }
            let (command, payload) = read_message(network, &mut stream)?;
            match command.as_str() {
                "version" if payload.len() >= 4 => {
                    version = Some(i32::from_le_bytes(payload[..4].try_into().unwrap()));
                    stream.write_all(&encode_message(network, "verack", &[]))?;
                }
                "verack" => got_verack = true,
                _ => {}
            }
        }
        let version = version.unwrap();
        let oldest = network.min_peer_protocol();
        if version < oldest {
            return protocol(format!("peer speaks {version}, older than {oldest}"));
        }
        Ok(Self {
            stream,
            network,
            version,
        })
    }

    pub fn protocol_version(&self) -> i32 {
        self.version
    }

    pub fn into_stream(self) -> S {
        self.stream
    }

    /// The headers that follow the first locator hash the peer knows; none when the peer has
    /// nothing newer, answers with an empty list or stays silent until the read timeout.
    pub fn getheaders(&mut self, locator: &[[u8; 32]]) -> Result<Vec<RawHeader>, P2pError> {
        self.stream.write_all(&encode_message(
            self.network,
            "getheaders",
            &getheaders_payload(locator),
        ))?;
        for _ in 0..60 {
            let (command, payload) = match read_message(self.network, &mut self.stream) {
                Ok(message) => message,
                // Zebra peers on mainnet stay silent when no block follows the locator: that is
                // "nothing newer", not a dead peer (a dead peer resets or closes the connection).
                Err(P2pError::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(Vec::new());
                }
                Err(e) => return Err(e),
            };
            match command.as_str() {
                "headers" => return parse_headers(&payload),
                "ping" => self
                    .stream
                    .write_all(&encode_message(self.network, "pong", &payload))?,
                _ => {}
            }
        }
        protocol("no headers reply")
    }
}

impl Peer<TcpStream> {
    pub fn connect(addr: std::net::SocketAddr, network: Network) -> Result<Self, P2pError> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
        stream.set_read_timeout(Some(Duration::from_secs(15)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        Self::handshake(stream, network)
    }
}

/// IPv4 addresses of peers behind the DNS seeders (or the given `host:port` overrides).
pub fn resolve(hosts: &[String], network: Network) -> Vec<std::net::SocketAddr> {
    let mut out = Vec::new();
    for host in hosts {
        let target = if host.contains(':') {
            host.clone()
        } else {
            format!("{host}:{}", network.port())
        };
        if let Ok(found) = target.to_socket_addrs() {
            out.extend(found.filter(|a| a.is_ipv4()));
        }
    }
    out
}
