use std::{path::PathBuf, process::Command, sync::Once};

use header_relay::{BUFFER_LEN, IX_BEGIN, IX_STEP, IX_WRITE, STATE_OFFSET};
use litesvm::LiteSVM;
use solana_sdk::{
    account::Account,
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction, InstructionError},
    message::Message,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::{Transaction, TransactionError},
};

const HEADER_HEX: &str = include_str!("../../../crates/zcash-header/fixtures/testnet-4483600.hex");
const CU_CAP: u32 = 1_400_000;

fn program_so() -> PathBuf {
    static BUILD: Once = Once::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    BUILD.call_once(|| {
        let status = Command::new("cargo")
            .args(["build-sbf", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .status()
            .expect("cargo build-sbf must be installed");
        assert!(status.success(), "cargo build-sbf failed");
    });
    root.join("../../target/deploy/header_relay.so")
}

struct Harness {
    svm: LiteSVM,
    payer: Keypair,
    program: Pubkey,
    buffer: Pubkey,
}

impl Harness {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        let program = Pubkey::new_unique();
        svm.add_program_from_file(program, program_so()).unwrap();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
        let buffer = Pubkey::new_unique();
        svm.set_account(
            buffer,
            Account {
                lamports: 100_000_000,
                data: vec![0; BUFFER_LEN],
                owner: program,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
        Self {
            svm,
            payer,
            program,
            buffer,
        }
    }

    /// Sends one transaction made of the given instruction payloads; returns compute units used.
    fn send_all(
        &mut self,
        payloads: &[Vec<u8>],
        cu_limit: Option<u32>,
    ) -> Result<u64, TransactionError> {
        self.svm.expire_blockhash();
        let mut ixs: Vec<Instruction> = cu_limit
            .map(ComputeBudgetInstruction::set_compute_unit_limit)
            .into_iter()
            .collect();
        ixs.extend(payloads.iter().map(|data| {
            Instruction::new_with_bytes(
                self.program,
                data,
                vec![AccountMeta::new(self.buffer, false)],
            )
        }));
        let msg = Message::new(&ixs, Some(&self.payer.pubkey()));
        let tx = Transaction::new(&[&self.payer], msg, self.svm.latest_blockhash());
        let size = bincode::serialize(&tx).unwrap().len();
        assert!(size <= 1232, "transaction is {size} bytes, limit 1232");
        self.svm
            .send_transaction(tx)
            .map(|meta| meta.compute_units_consumed)
            .map_err(|e| {
                if std::env::var("LOGS").is_ok() {
                    eprintln!("{:#?}", e.meta.logs);
                }
                e.err
            })
    }

    fn send(&mut self, data: Vec<u8>) -> Result<u64, TransactionError> {
        self.send_all(&[data], Some(CU_CAP))
    }

    fn step(&mut self, leaves: u16) -> Result<u64, TransactionError> {
        self.send(step_ix(leaves))
    }

    fn state_stage(&self) -> u8 {
        self.svm.get_account(&self.buffer).unwrap().data[STATE_OFFSET]
    }
}

fn header() -> Vec<u8> {
    hex::decode(HEADER_HEX.trim()).unwrap()
}

fn step_ix(leaves: u16) -> Vec<u8> {
    let mut data = vec![IX_STEP];
    data.extend(leaves.to_le_bytes());
    data
}

fn write_ix(header: &[u8], from: usize, to: usize) -> Vec<u8> {
    let mut data = vec![IX_WRITE];
    data.extend((from as u16).to_le_bytes());
    data.extend(&header[from..to]);
    data
}

fn custom_any(err: &TransactionError) -> Option<u32> {
    match err {
        TransactionError::InstructionError(_, InstructionError::Custom(c)) => Some(*c),
        _ => None,
    }
}

/// Bytes of header sent by the first, write-only transaction. A legacy
/// transaction is limited to 1232 bytes in total.
const FIRST_WRITE: usize = 950;

/// Relays one header: a write-only transaction, then a transaction that
/// finishes the write, begins and runs the first step, then one per further
/// step. Returns the compute units of every transaction.
fn relay_header(
    h: &mut Harness,
    header: &[u8],
    leaves_per_step: u16,
) -> Result<Vec<u64>, TransactionError> {
    let mut cu = vec![h.send_all(&[write_ix(header, 0, FIRST_WRITE)], None)?];
    cu.push(h.send_all(
        &[
            write_ix(header, FIRST_WRITE, header.len()),
            vec![IX_BEGIN],
            step_ix(leaves_per_step),
        ],
        Some(CU_CAP),
    )?);
    for _ in 1..512usize.div_ceil(leaves_per_step as usize) {
        cu.push(h.step(leaves_per_step)?);
    }
    Ok(cu)
}

#[test]
fn real_testnet_header_verifies_across_transactions_within_the_cap() {
    for per_step in [256u16, 171, 128] {
        let mut h = Harness::new();
        let cu = relay_header(&mut h, &header(), per_step).unwrap();
        assert_eq!(h.state_stage(), 2, "state must be Verified");
        println!("CU leaves_per_step={per_step}: {cu:?}");
        assert!(cu.iter().all(|c| *c < CU_CAP as u64), "{cu:?}");
    }
}

#[test]
fn corrupted_solution_fails() {
    let good = header();
    for byte in [143usize, 700, 1486] {
        let mut bad = good.clone();
        bad[byte] ^= 0x10;
        let mut h = Harness::new();
        let err = relay_header(&mut h, &bad, 256).unwrap_err();
        assert!(matches!(custom_any(&err), Some(4..=7)), "{err:?}");
        assert_ne!(h.state_stage(), 2);
    }
}

#[test]
fn step_before_begin_and_after_verified_fail() {
    let mut h = Harness::new();
    let hdr = header();
    h.send_all(&[write_ix(&hdr, 0, FIRST_WRITE)], None).unwrap();
    h.send_all(&[write_ix(&hdr, FIRST_WRITE, hdr.len())], None)
        .unwrap();
    assert_eq!(custom_any(&h.step(10).unwrap_err()), Some(3));
    h.send(vec![IX_BEGIN]).unwrap();
    h.step(256).unwrap();
    h.step(256).unwrap();
    assert_eq!(h.state_stage(), 2);
    assert_eq!(custom_any(&h.step(1).unwrap_err()), Some(3));
}

#[test]
fn header_with_wrong_prefix_is_rejected_by_begin() {
    let mut h = Harness::new();
    let mut bad = header();
    bad[140] = 0;
    h.send_all(&[write_ix(&bad, 0, FIRST_WRITE)], None).unwrap();
    h.send_all(&[write_ix(&bad, FIRST_WRITE, bad.len())], None)
        .unwrap();
    assert_eq!(custom_any(&h.send(vec![IX_BEGIN]).unwrap_err()), Some(1));
}

#[test]
fn write_past_the_header_is_rejected() {
    let mut h = Harness::new();
    let mut data = vec![IX_WRITE];
    data.extend(1480u16.to_le_bytes());
    data.extend([0u8; 8]);
    assert_eq!(
        h.send(data).unwrap_err(),
        TransactionError::InstructionError(1, InstructionError::InvalidInstructionData)
    );
}

#[test]
fn account_not_owned_by_the_program_is_rejected() {
    let mut h = Harness::new();
    let mut acct = h.svm.get_account(&h.buffer).unwrap();
    acct.owner = Pubkey::new_unique();
    h.svm.set_account(h.buffer, acct).unwrap();
    assert!(h.send(vec![IX_BEGIN]).is_err());
}
