//! Creating a Header Relay: build a Checkpoint from live headers, write it, seal it, grow the Store.
use borsh::BorshSerialize;
use header_relay_client::{
    accounts::{Relay, Store},
    instructions::{
        CreateRelayBuilder, GrowStoreBuilder, SealRelayBuilder, WriteCheckpointBuilder,
    },
    types::Checkpoint,
};
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use zcash_header::header;

use crate::{
    chain::{error_name, Cluster, TxError, SYSTEM_PROGRAM},
    p2p::{sha256d, Network, RawHeader},
};

/// Blocks of times and difficulty a Checkpoint carries: the 102-block NU7 window plus the median of 11.
pub const CONTEXT: usize = 113;
/// The Store holds headers once it has 400 slots: three `GrowStore` calls.
const GROWS: usize = 3;
const WRITE_CHUNK: usize = 480;

/// The Checkpoint whose block is the last of the 113 consecutive `headers`; `last_height` is its height.
pub fn checkpoint_from(
    headers: &[RawHeader],
    last_height: u32,
    finality_depth: u16,
    nonce: u64,
    network: Network,
) -> Result<Checkpoint, String> {
    let headers: &[RawHeader; CONTEXT] = headers.try_into().map_err(|_| {
        format!(
            "a Checkpoint needs {CONTEXT} headers, got {}",
            headers.len()
        )
    })?;
    for pair in headers.windows(2) {
        if pair[1][4..36] != sha256d(&pair[0]) {
            return Err("headers are not consecutive".into());
        }
    }
    Ok(Checkpoint {
        version: 1,
        network: network.checkpoint_network(),
        finality_depth,
        nu7_height: network.nu7_height(),
        height: last_height,
        nonce,
        hash: sha256d(&headers[CONTEXT - 1]),
        times: headers.map(|h| header::time(&h)),
        bits: headers.map(|h| header::bits(&h)),
    })
}

pub fn checkpoint_digest(checkpoint: &Checkpoint) -> [u8; 32] {
    let mut bytes = Vec::new();
    checkpoint.serialize(&mut bytes).unwrap();
    Sha256::digest(bytes).into()
}

fn send(
    cluster: &mut impl Cluster,
    payer: &Keypair,
    ixs: &[Instruction],
) -> Result<String, String> {
    cluster.send(ixs, payer).map_err(|e| match e {
        TxError::Program { code, .. } => error_name(code),
        TxError::Other(m) => m,
    })
}

/// Creates, fills and seals the Relay for `checkpoint` and grows its Store to 400 slots.
/// Returns the Relay address and the signatures of the transactions that made it.
pub fn create_relay(
    cluster: &mut impl Cluster,
    payer: &Keypair,
    checkpoint: &Checkpoint,
) -> Result<(Address, Vec<String>), String> {
    let digest = checkpoint_digest(checkpoint);
    let creator = payer.pubkey();
    let relay = Relay::find_pda(&creator, digest).0;
    let store = Store::find_pda(&relay).0;
    let mut signatures = Vec::new();
    let create = CreateRelayBuilder::new()
        .payer(creator)
        .relay(relay)
        .store(store)
        .system_program(SYSTEM_PROGRAM)
        .checkpoint_digest(digest)
        .instruction();
    signatures.push(send(cluster, payer, &[create])?);
    let mut bytes = Vec::new();
    checkpoint.serialize(&mut bytes).unwrap();
    for (i, chunk) in bytes.chunks(WRITE_CHUNK).enumerate() {
        let write = WriteCheckpointBuilder::new()
            .creator(creator)
            .relay(relay)
            .offset((i * WRITE_CHUNK) as u16)
            .bytes(chunk.to_vec())
            .instruction();
        signatures.push(send(cluster, payer, &[write])?);
    }
    let seal = SealRelayBuilder::new().relay(relay).instruction();
    signatures.push(send(cluster, payer, &[seal])?);
    for _ in 0..GROWS {
        let grow = GrowStoreBuilder::new()
            .payer(creator)
            .store(store)
            .system_program(SYSTEM_PROGRAM)
            .instruction();
        signatures.push(send(cluster, payer, &[grow])?);
    }
    Ok((relay, signatures))
}

/// Whether a header was mined within the last hour of `now` (Unix seconds). A scan that meets a
/// silent peer uses it to tell "the tip" from "a peer that stopped answering behind the tip".
pub fn looks_current(header: &RawHeader, now: u64) -> bool {
    now.saturating_sub(u64::from(header::time(header))) < 3_600
}
