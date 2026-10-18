#![allow(dead_code)]

use std::{path::PathBuf, process::Command, sync::Once};

use borsh::BorshSerialize;
use header_relay_client::{
    accounts::{HeaderBuffer, Relay, Store},
    instructions::*,
    types::Checkpoint,
    HEADER_RELAY_ID,
};
use litesvm::{
    types::{FailedTransactionMetadata, TransactionMetadata},
    LiteSVM,
};
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

pub const CU_CAP: u32 = 1_400_000;
pub const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);
const COMPUTE_BUDGET: Address = Address::from_str_const("ComputeBudget111111111111111111111111111111");

/// Real testnet header at height 4,483,600 (raw 1,487 bytes).
pub const HEADER_4483600: &str = include_str!("../../zcash-header/fixtures/testnet-4483600.hex");
/// Its child at height 4,483,601.
pub const HEADER_4483601: &str = include_str!("../../zcash-header/fixtures/testnet-4483601.hex");
const CHECKPOINT_JSON: &str = include_str!("../../zcash-header/fixtures/testnet-checkpoint-4483599.json");

pub fn header(hex_str: &str) -> Vec<u8> {
    hex::decode(hex_str.trim()).unwrap()
}

fn program_so() -> PathBuf {
    static BUILD: Once = Once::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../programs/header-relay");
    BUILD.call_once(|| {
        let status = Command::new("cargo")
            .args(["build-sbf", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .status()
            .expect("cargo build-sbf must be installed");
        assert!(status.success(), "cargo build-sbf failed");
    });
    root.join("target/deploy/header_relay.so")
}

/// The testnet Checkpoint at height 4,483,599 built from the exported fixture.
pub fn testnet_checkpoint(finality_depth: u16, nonce: u64) -> Checkpoint {
    let json: serde_json::Value = serde_json::from_str(CHECKPOINT_JSON).unwrap();
    let blocks = json["blocks"].as_array().unwrap();
    let times: Vec<u32> = blocks.iter().map(|b| b["time"].as_u64().unwrap() as u32).collect();
    let bits: Vec<u32> = blocks
        .iter()
        .map(|b| u32::from_str_radix(b["bits"].as_str().unwrap(), 16).unwrap())
        .collect();
    let mut hash: [u8; 32] = hex::decode(blocks.last().unwrap()["hash"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    hash.reverse();
    Checkpoint {
        version: 1,
        network: 1,
        finality_depth,
        nu7_height: 4_465_026,
        height: json["tip_height"].as_u64().unwrap() as u32,
        nonce,
        hash,
        times: times.try_into().unwrap(),
        bits: bits.try_into().unwrap(),
    }
}

pub fn digest_of(checkpoint: &Checkpoint) -> [u8; 32] {
    Sha256::digest(borsh::to_vec(checkpoint).unwrap()).into()
}

pub struct Harness {
    pub svm: LiteSVM,
    pub payer: Keypair,
}

pub struct Sent {
    pub cu: u64,
}

pub fn set_compute_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction::new_with_bytes(COMPUTE_BUDGET, &data, vec![])
}

impl Harness {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(HEADER_RELAY_ID, program_so()).unwrap();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();
        Self { svm, payer }
    }

    pub fn send(
        &mut self,
        ixs: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        self.svm.expire_blockhash();
        let mut signers: Vec<&Keypair> = vec![&self.payer];
        signers.extend_from_slice(extra_signers);
        let tx = Transaction::new(
            &signers,
            Message::new(ixs, Some(&self.payer.pubkey())),
            self.svm.latest_blockhash(),
        );
        self.svm.send_transaction(tx)
    }

    /// Sends and expects success; returns compute units used.
    pub fn ok(&mut self, ixs: &[Instruction], extra_signers: &[&Keypair]) -> u64 {
        match self.send(ixs, extra_signers) {
            Ok(meta) => meta.compute_units_consumed,
            Err(e) => panic!("transaction failed: {:?}\n{}", e.err, e.meta.pretty_logs()),
        }
    }

    /// Sends and expects the instruction at `index` to fail with a custom error code.
    pub fn custom_err(&mut self, ixs: &[Instruction], extra_signers: &[&Keypair]) -> u32 {
        match self.send(ixs, extra_signers) {
            Ok(_) => panic!("transaction unexpectedly succeeded"),
            Err(e) => match e.err {
                TransactionError::InstructionError(_, InstructionError::Custom(code)) => code,
                other => panic!("expected a custom error, got {other:?}\n{}", e.meta.pretty_logs()),
            },
        }
    }

    pub fn relay_addr(checkpoint: &Checkpoint) -> Address {
        Relay::find_pda(digest_of(checkpoint)).0
    }

    pub fn store_addr(relay: &Address) -> Address {
        Store::find_pda(relay).0
    }

    pub fn buffer_addr(relay: &Address, authority: &Address, nonce: u8) -> Address {
        HeaderBuffer::find_pda(relay, authority, nonce).0
    }

    pub fn create_relay_ix(&self, checkpoint: &Checkpoint) -> Instruction {
        let relay = Self::relay_addr(checkpoint);
        CreateRelayBuilder::new()
            .payer(self.payer.pubkey())
            .relay(relay)
            .store(Self::store_addr(&relay))
            .system_program(SYSTEM_PROGRAM)
            .checkpoint_digest(digest_of(checkpoint))
            .instruction()
    }

    pub fn write_checkpoint_ixs(&self, checkpoint: &Checkpoint) -> Vec<Instruction> {
        let bytes = borsh::to_vec(checkpoint).unwrap();
        let relay = Self::relay_addr(checkpoint);
        bytes
            .chunks(480)
            .enumerate()
            .map(|(i, chunk)| {
                WriteCheckpointBuilder::new()
                    .creator(self.payer.pubkey())
                    .relay(relay)
                    .offset((i * 480) as u16)
                    .bytes(chunk.to_vec())
                    .instruction()
            })
            .collect()
    }

    pub fn seal_ix(&self, checkpoint: &Checkpoint) -> Instruction {
        SealRelayBuilder::new()
            .relay(Self::relay_addr(checkpoint))
            .instruction()
    }

    /// Creates, fills and seals a Relay; returns its address.
    pub fn init_relay(&mut self, checkpoint: &Checkpoint) -> Address {
        let create = self.create_relay_ix(checkpoint);
        self.ok(&[create], &[]);
        for ix in self.write_checkpoint_ixs(checkpoint) {
            self.ok(&[ix], &[]);
        }
        let seal = self.seal_ix(checkpoint);
        self.ok(&[seal], &[]);
        Self::relay_addr(checkpoint)
    }

    pub fn relay(&self, relay: &Address) -> Relay {
        Relay::from_bytes(&self.svm.get_account(relay).unwrap().data).unwrap()
    }
}

/// Compute units of each transaction used to relay one header.
#[derive(Debug, Default)]
pub struct Cu {
    pub upload_first: u64,
    pub upload_rest_and_first_leaves: u64,
    pub last_leaves_and_accept: u64,
    pub close: u64,
}

impl Harness {
    pub fn create_buffer_ix(&self, relay: &Address, nonce: u8) -> Instruction {
        self.create_buffer_for(&self.payer.pubkey(), relay, nonce)
    }

    pub fn create_buffer_for(&self, authority: &Address, relay: &Address, nonce: u8) -> Instruction {
        CreateBufferBuilder::new()
            .authority(*authority)
            .relay(*relay)
            .buffer(Self::buffer_addr(relay, authority, nonce))
            .system_program(SYSTEM_PROGRAM)
            .nonce(nonce)
            .instruction()
    }

    pub fn write_ix(&self, buffer: &Address, offset: usize, bytes: &[u8]) -> Instruction {
        WriteBufferBuilder::new()
            .authority(self.payer.pubkey())
            .buffer(*buffer)
            .offset(offset as u16)
            .bytes(bytes.to_vec())
            .instruction()
    }

    pub fn begin_ix(&self, buffer: &Address) -> Instruction {
        BeginVerifyBuilder::new()
            .authority(self.payer.pubkey())
            .buffer(*buffer)
            .instruction()
    }

    pub fn step_ix(&self, buffer: &Address, leaves: u16) -> Instruction {
        StepVerifyBuilder::new()
            .authority(self.payer.pubkey())
            .buffer(*buffer)
            .leaves(leaves)
            .instruction()
    }

    pub fn accept_ix(&self, buffer: &Address, relay: &Address) -> Instruction {
        AcceptHeaderBuilder::new()
            .authority(self.payer.pubkey())
            .buffer(*buffer)
            .relay(*relay)
            .store(Self::store_addr(relay))
            .instruction()
    }

    pub fn close_ix(&self, buffer: &Address) -> Instruction {
        CloseBufferBuilder::new()
            .authority(self.payer.pubkey())
            .buffer(*buffer)
            .instruction()
    }

    /// Uploads a header and runs it through verification up to, but not including, acceptance.
    /// Two transactions; returns the buffer address.
    pub fn upload_and_verify(&mut self, relay: &Address, nonce: u8, header: &[u8], cu: &mut Cu) -> Address {
        self.try_upload_and_verify(relay, nonce, header, cu).expect("verification failed")
    }

    /// Like [`Self::upload_and_verify`] but returns the custom error code of a failing step.
    pub fn try_upload_and_verify(
        &mut self,
        relay: &Address,
        nonce: u8,
        header: &[u8],
        cu: &mut Cu,
    ) -> Result<Address, u32> {
        let buffer = Self::buffer_addr(relay, &self.payer.pubkey(), nonce);
        let split = 850;
        let first = [self.create_buffer_ix(relay, nonce), self.write_ix(&buffer, 0, &header[..split])];
        cu.upload_first = self.ok(&first, &[]);
        let second = [
            set_compute_limit(CU_CAP),
            self.write_ix(&buffer, split, &header[split..]),
            self.begin_ix(&buffer),
            self.step_ix(&buffer, 256),
        ];
        match self.send(&second, &[]) {
            Ok(meta) => cu.upload_rest_and_first_leaves = meta.compute_units_consumed,
            Err(e) => match e.err {
                TransactionError::InstructionError(_, InstructionError::Custom(code)) => return Err(code),
                other => panic!("unexpected failure {other:?}"),
            },
        }
        Ok(buffer)
    }

    /// The transaction that finishes verification and accepts the header.
    pub fn finish_ixs(&self, buffer: &Address, relay: &Address) -> Vec<Instruction> {
        vec![
            set_compute_limit(CU_CAP),
            self.step_ix(buffer, 256),
            self.accept_ix(buffer, relay),
        ]
    }

    /// Relays one header end to end and closes its buffer.
    pub fn relay_header(&mut self, relay: &Address, nonce: u8, header: &[u8]) -> Cu {
        let mut cu = Cu::default();
        let buffer = self.upload_and_verify(relay, nonce, header, &mut cu);
        let last = self.finish_ixs(&buffer, relay);
        cu.last_leaves_and_accept = self.ok(&last, &[]);
        cu.close = self.ok(&[self.close_ix(&buffer)], &[]);
        cu
    }

    pub fn store(&self, relay: &Address) -> (Store, Vec<u8>) {
        let data = self.svm.get_account(&Self::store_addr(relay)).unwrap().data;
        (Store::from_bytes(&data).unwrap(), data)
    }
}

pub fn serialize<T: BorshSerialize>(v: &T) -> Vec<u8> {
    borsh::to_vec(v).unwrap()
}

/// System program transfer, built by hand so the harness needs no system-interface crate.
pub fn solana_system_transfer(from: &Address, to: &Address, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction::new_with_bytes(
        SYSTEM_PROGRAM,
        &data,
        vec![
            solana_instruction::AccountMeta::new(*from, true),
            solana_instruction::AccountMeta::new(*to, false),
        ],
    )
}
