//! The Header Relay on Solana: the transactions that relay one header, and reading the relay back.
use header_relay_client::{
    accounts::{HeaderBuffer, Relay, Store},
    errors::HeaderRelayError,
    instructions::{
        AcceptHeaderBuilder, BeginVerifyBuilder, CloseBufferBuilder, CreateBufferBuilder,
        StepVerifyBuilder, WriteBufferBuilder,
    },
    types::HeaderSlot,
};
use num_traits::FromPrimitive;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;

use crate::{
    follower::{ChainError, Hash, Outcome, RelayChain, Stage, Tip},
    p2p::RawHeader,
};

pub const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);
const COMPUTE_BUDGET: Address =
    Address::from_str_const("ComputeBudget111111111111111111111111111111");
/// Largest compute budget a transaction may ask for.
pub const CU_CAP: u32 = 1_400_000;
/// Header bytes written by the first transaction; the rest go with the first verification step.
const SPLIT: usize = 850;
/// Equihash leaves one `StepVerify` covers (two steps verify a header).
const LEAVES: u16 = 256;
const DUPLICATE_HEADER: u32 = 13;
const STORE_HEADER_LEN: usize = 40;
const SLOT_LEN: usize = 102;

#[derive(Debug)]
pub enum TxError {
    /// The program refused instruction `index` with this custom error code.
    Program { index: usize, code: u32 },
    /// The cluster did not take or confirm the transaction; trying again may work.
    Other(String),
}

/// A Solana cluster: sends transactions signed by the payer and reads accounts.
pub trait Cluster {
    /// Sends and waits until the transaction is confirmed; returns its signature.
    fn send(&mut self, instructions: &[Instruction], payer: &Keypair) -> Result<String, TxError>;
    /// The owner program and data of an account, if it exists.
    fn account_with_owner(
        &mut self,
        address: &Address,
    ) -> Result<Option<(Address, Vec<u8>)>, String>;
    fn account(&mut self, address: &Address) -> Result<Option<Vec<u8>>, String> {
        Ok(self.account_with_owner(address)?.map(|(_, data)| data))
    }
}

fn compute_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction::new_with_bytes(COMPUTE_BUDGET, &data, vec![])
}

pub fn error_name(code: u32) -> String {
    match HeaderRelayError::from_u32(code) {
        Some(e) => format!("{e:?} ({code})"),
        None => format!("custom error {code}"),
    }
}

/// The relay a Relayer feeds, with the payer that signs and pays.
pub struct RelayClient<C> {
    cluster: C,
    payer: Keypair,
    relay: Address,
    nonce: u8,
}

impl<C: Cluster> RelayClient<C> {
    pub fn new(cluster: C, payer: Keypair, relay: Address) -> Self {
        Self {
            cluster,
            payer,
            relay,
            nonce: 0,
        }
    }

    pub fn cluster_mut(&mut self) -> &mut C {
        &mut self.cluster
    }

    fn buffer(&self) -> Address {
        HeaderBuffer::find_pda(&self.relay, &self.payer.pubkey(), self.nonce).0
    }

    fn authority(&self) -> Address {
        self.payer.pubkey()
    }

    fn read_relay(&mut self) -> Result<Relay, ChainError> {
        let data = self
            .cluster
            .account(&self.relay)
            .map_err(ChainError::Transient)?
            .ok_or_else(|| ChainError::Transient("the relay account does not exist".into()))?;
        Relay::from_bytes(&data).map_err(|e| ChainError::Transient(format!("relay account: {e}")))
    }

    fn close_ix(&self) -> Instruction {
        CloseBufferBuilder::new()
            .authority(self.authority())
            .buffer(self.buffer())
            .instruction()
    }

    /// Closes a buffer left by an attempt that stopped halfway; its rent comes back.
    fn close_leftover(&mut self) -> Result<(), ChainError> {
        let buffer = self.buffer();
        let exists = self
            .cluster
            .account(&buffer)
            .map_err(ChainError::Transient)?
            .is_some();
        if exists {
            self.send(&[self.close_ix()])?;
        }
        Ok(())
    }

    fn send(&mut self, ixs: &[Instruction]) -> Result<String, ChainError> {
        match self.cluster.send(ixs, &self.payer) {
            Ok(signature) => Ok(signature),
            Err(TxError::Program { code, .. }) => Err(ChainError::Transient(error_name(code))),
            Err(TxError::Other(m)) => Err(ChainError::Transient(m)),
        }
    }

    /// Sends a verification transaction; a refusal by the program is a rejection of the header,
    /// anything else is worth retrying.
    fn send_checked(&mut self, ixs: &[Instruction]) -> Result<Result<String, u32>, ChainError> {
        match self.cluster.send(ixs, &self.payer) {
            Ok(signature) => Ok(Ok(signature)),
            Err(TxError::Program { code, .. }) => Ok(Err(code)),
            Err(TxError::Other(m)) => Err(ChainError::Transient(m)),
        }
    }

    fn refused(&mut self, code: u32) -> Result<Outcome, ChainError> {
        self.close_leftover()?;
        Ok(if code == DUPLICATE_HEADER {
            Outcome::AlreadyKnown
        } else {
            Outcome::Rejected {
                code,
                reason: error_name(code),
            }
        })
    }
}

impl<C: Cluster> RelayChain for RelayClient<C> {
    fn tip(&mut self) -> Result<Tip, ChainError> {
        let relay = self.read_relay()?;
        Ok(Tip {
            height: relay.tip_height,
            hash: relay.tip_hash,
            halted: relay.halted,
        })
    }

    fn known_blocks(&mut self) -> Result<Vec<(Hash, u32)>, ChainError> {
        let relay = self.read_relay()?;
        let store = self
            .cluster
            .account(&Store::find_pda(&self.relay).0)
            .map_err(ChainError::Transient)?
            .ok_or_else(|| ChainError::Transient("the store account does not exist".into()))?;
        let mut blocks = vec![(relay.checkpoint.hash, relay.checkpoint.height)];
        for slot in store[STORE_HEADER_LEN..].chunks_exact(SLOT_LEN) {
            let slot = parse_slot(slot)?;
            if slot.seq != 0 {
                blocks.push((slot.hash, slot.height));
            }
        }
        blocks.sort_by_key(|(_, height)| std::cmp::Reverse(*height));
        Ok(blocks)
    }

    fn submit(
        &mut self,
        header: &RawHeader,
        stage: &mut dyn FnMut(Stage),
    ) -> Result<Outcome, ChainError> {
        self.close_leftover()?;
        let buffer = self.buffer();
        let authority = self.authority();
        let write = |offset: usize, bytes: &[u8]| {
            WriteBufferBuilder::new()
                .authority(authority)
                .buffer(buffer)
                .offset(offset as u16)
                .bytes(bytes.to_vec())
                .instruction()
        };

        let create = CreateBufferBuilder::new()
            .authority(authority)
            .relay(self.relay)
            .buffer(buffer)
            .system_program(SYSTEM_PROGRAM)
            .nonce(self.nonce)
            .instruction();
        let mut signatures = Vec::new();
        // Creating the buffer can fail for reasons that say nothing about the header (rent, a Halt):
        // those are retried, never counted against the peer.
        signatures.push(self.send(&[create, write(0, &header[..SPLIT])])?);

        let begin = BeginVerifyBuilder::new()
            .authority(authority)
            .buffer(buffer)
            .instruction();
        let step = StepVerifyBuilder::new()
            .authority(authority)
            .buffer(buffer)
            .leaves(LEAVES)
            .instruction();
        let second = [
            compute_limit(CU_CAP),
            write(SPLIT, &header[SPLIT..]),
            begin,
            step.clone(),
        ];
        match self.send_checked(&second)? {
            Ok(signature) => signatures.push(signature),
            Err(code) => return self.refused(code),
        }
        stage(Stage::Uploaded);

        let accept = AcceptHeaderBuilder::new()
            .authority(authority)
            .buffer(buffer)
            .relay(self.relay)
            .store(Store::find_pda(&self.relay).0)
            .instruction();
        match self.send_checked(&[compute_limit(CU_CAP), step, accept])? {
            Ok(signature) => signatures.push(signature),
            Err(code) => return self.refused(code),
        }
        stage(Stage::Verified);

        // The accepted buffer only holds rent now; a failure to close is cleaned up by the next header.
        let _ = self.send(&[self.close_ix()]);
        let tip = self.tip()?;
        Ok(Outcome::Accepted { tip, signatures })
    }
}

fn parse_slot(bytes: &[u8]) -> Result<HeaderSlot, ChainError> {
    use borsh::BorshDeserialize;
    let mut bytes = bytes;
    HeaderSlot::deserialize(&mut bytes)
        .map_err(|e| ChainError::Transient(format!("store slot: {e}")))
}
