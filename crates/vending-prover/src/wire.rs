//! Minimal lightwalletd protobuf projection, pinned to librustzcash's vendored
//! lightwallet-protocol. Unknown fields are skipped by prost.
use anyhow::{bail, ensure, Result};
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct BlockId {
    #[prost(uint64, tag = "1")]
    pub height: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
pub struct TxFilter {
    #[prost(bytes = "vec", tag = "3")]
    pub hash: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
pub struct RawTransaction {
    #[prost(bytes = "vec", tag = "1")]
    pub data: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub height: u64,
}
#[derive(Clone, PartialEq, Message)]
pub struct CompactBlock {
    #[prost(uint64, tag = "2")]
    pub height: u64,
    #[prost(message, repeated, tag = "7")]
    pub vtx: Vec<CompactTx>,
}
#[derive(Clone, PartialEq, Message)]
pub struct CompactTx {
    #[prost(bytes = "vec", tag = "2")]
    pub txid: Vec<u8>,
    #[prost(message, repeated, tag = "9")]
    pub ironwood_actions: Vec<CompactAction>,
}
#[derive(Clone, PartialEq, Message)]
pub struct CompactAction {
    #[prost(bytes = "vec", tag = "1")]
    pub nullifier: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub cmx: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub ephemeral_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub ciphertext: Vec<u8>,
}

pub fn frame(message: &impl Message) -> Vec<u8> {
    let b = message.encode_to_vec();
    let mut out = vec![0];
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend(b);
    out
}

pub fn unframe(mut bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut messages = Vec::new();
    let mut trailers = false;
    while !bytes.is_empty() {
        ensure!(!trailers && bytes.len() >= 5, "invalid gRPC-web framing");
        let flag = bytes[0];
        let len = u32::from_be_bytes(bytes[1..5].try_into()?) as usize;
        bytes = &bytes[5..];
        ensure!(len <= bytes.len(), "truncated gRPC-web frame");
        let payload = &bytes[..len];
        bytes = &bytes[len..];
        match flag {
            0 => messages.push(payload.to_vec()),
            0x80 => {
                trailers = true;
                let text = std::str::from_utf8(payload)?;
                let status = text.lines().find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("grpc-status"))
                        .map(|(_, v)| v.trim())
                });
                ensure!(status == Some("0"), "gRPC-web error: {text}");
            }
            _ => bail!("compressed or unknown gRPC-web frame"),
        }
    }
    ensure!(trailers, "missing gRPC-web status trailers");
    Ok(messages)
}
