#![allow(clippy::result_large_err)]
//! LiteSVM harness; the Relay is initialized and advanced using relay-tests' real program helpers.
use header_relay_interface::{RelayView, HEADER_RELAY_PROGRAM};
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use relay_tests::{
    synthetic::{synthetic_checkpoint, Branch},
    Harness,
};
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::{path::PathBuf, process::Command, sync::Once};
use vending::{
    error::VendingError,
    state::{Machine, Reservation},
};
use vending_client::{instructions::*, VENDING_ID};
pub const TOKEN: Address = Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const ATA: Address = Address::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const SYSTEM: Address = Address::new_from_array([0; 32]);
pub const START: i64 = 1_800_000_000;
pub fn program_so() -> PathBuf {
    static BUILD: Once = Once::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../programs/vending");
    BUILD.call_once(|| {
        loop {
            let info = std::fs::read_to_string("/proc/meminfo").unwrap();
            let kb: u64 = info
                .lines()
                .find_map(|line| line.strip_prefix("MemAvailable:"))
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap();
            if kb >= 3 * 1024 * 1024 {
                break;
            }
            eprintln!("waiting for 3 GiB available before SBF build");
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        // CI runners have no user systemd bus. Local builds use the laptop's resource cap.
        let mut build = if std::env::var_os("CI").is_some() {
            Command::new("cargo")
        } else {
            let mut capped = Command::new("systemd-run");
            capped.args([
                "--user",
                "--scope",
                "-q",
                "-p",
                "MemoryMax=3G",
                "-p",
                "MemorySwapMax=0",
                "--",
                "env",
                "CARGO_BUILD_JOBS=2",
                "cargo",
            ]);
            capped
        };
        assert!(build
            .args(["build-sbf", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .status()
            .unwrap()
            .success());
    });
    root.join("target/deploy/vending.so")
}
pub struct World {
    pub h: Harness,
    pub relay: Address,
    pub branch: Branch,
    pub payee: Keypair,
    pub payer: Keypair,
    pub mint: Keypair,
    pub machine: Address,
    pub authority: Address,
}
impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
impl World {
    pub fn new() -> Self {
        let mut h = Harness::new();
        h.svm
            .add_program_from_file(VENDING_ID, program_so())
            .unwrap();
        let checkpoint = synthetic_checkpoint(1, 16);
        let relay = h.init_relay(&checkpoint);
        let branch = Branch::new(&checkpoint);
        let payee = Keypair::new();
        let payer = Keypair::new();
        let mint = Keypair::new();
        h.svm.airdrop(&payee.pubkey(), 1_000_000_000).unwrap();
        h.svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();
        let machine = Address::find_program_address(
            &[b"machine", payee.pubkey().as_ref(), &[16; 32]],
            &VENDING_ID,
        )
        .0;
        let authority = Address::find_program_address(&[b"mint", machine.as_ref()], &VENDING_ID).0;
        let mut w = Self {
            h,
            relay,
            branch,
            payee,
            payer,
            mint,
            machine,
            authority,
        };
        w.time(START);
        w.mine(3);
        w
    }
    pub fn init_ix(&self, stock: u32) -> Instruction {
        InitMachine {
            payee: self.payee.pubkey(),
            machine: self.machine,
            mint: self.mint.pubkey(),
            mint_authority: self.authority,
            relay: self.relay,
            token_program: TOKEN,
            system_program: SYSTEM,
        }
        .instruction(InitMachineInstructionArgs {
            seed: [16; 32],
            price_zat: 2_500_000,
            stock,
            diversifier: [5; 11],
            svk_hash: [6; 32],
            relay_address: self.relay,
            vkey_hash: [7; 32],
            reservation_blocks: 2,
        })
    }
    pub fn init(&mut self, stock: u32) {
        let ix = self.init_ix(stock);
        let meta = self.h.send(&[ix], &[&self.payee, &self.mint]).unwrap();
        println!("CU InitMachine {}", meta.compute_units_consumed);
    }
    pub fn reservation(&self, nonce: [u8; 32]) -> Address {
        Address::find_program_address(
            &[b"reservation", self.machine.as_ref(), &nonce],
            &VENDING_ID,
        )
        .0
    }
    pub fn reserve_ix(&self, nonce: [u8; 32]) -> Instruction {
        Reserve {
            payer: self.payer.pubkey(),
            machine: self.machine,
            reservation: self.reservation(nonce),
            relay: self.relay,
            system_program: SYSTEM,
        }
        .instruction(ReserveInstructionArgs { nonce })
    }
    pub fn reserve(
        &mut self,
        nonce: [u8; 32],
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        let ix = self.reserve_ix(nonce);
        let meta = self.h.send(&[ix], &[&self.payer]);
        if let Ok(m) = &meta {
            println!("CU Reserve {}", m.compute_units_consumed);
        }
        meta
    }
    pub fn release_ix(&self, nonce: [u8; 32]) -> Instruction {
        ReleaseReservation {
            submitter: self.h.payer.pubkey(),
            machine: self.machine,
            reservation: self.reservation(nonce),
            relay: self.relay,
            payout: self.payer.pubkey(),
        }
        .instruction()
    }
    pub fn release(
        &mut self,
        nonce: [u8; 32],
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        let ix = self.release_ix(nonce);
        let meta = self.h.send(&[ix], &[]);
        if let Ok(m) = &meta {
            println!("CU ReleaseReservation {}", m.compute_units_consumed);
        }
        meta
    }
    pub fn claim_ix(&self, nonce: [u8; 32]) -> Instruction {
        let tag = [11; 32];
        let payment_tag =
            Address::find_program_address(&[b"tag", self.machine.as_ref(), &tag], &VENDING_ID).0;
        let payout_token = Address::find_program_address(
            &[
                self.payer.pubkey().as_ref(),
                TOKEN.as_ref(),
                self.mint.pubkey().as_ref(),
            ],
            &ATA,
        )
        .0;
        let mut public_values = [0; 249];
        public_values[0] = 1;
        public_values[1..5].copy_from_slice(&vending::state::NU7_BRANCH_ID.to_le_bytes());
        public_values[5..37].copy_from_slice(self.machine.as_ref());
        public_values[37..69].copy_from_slice(&nonce);
        public_values[69..77].copy_from_slice(&2_500_000u64.to_le_bytes());
        public_values[77..109].copy_from_slice(&tag);
        public_values[109..141].copy_from_slice(&self.root());
        public_values[141..145].copy_from_slice(&self.tip().to_le_bytes());
        public_values[145..156].fill(5);
        public_values[156..188].fill(6);
        public_values[188] = 3;
        Claim {
            submitter: self.h.payer.pubkey(),
            machine: self.machine,
            reservation: self.reservation(nonce),
            relay: self.relay,
            payout: self.payer.pubkey(),
            mint: self.mint.pubkey(),
            mint_authority: self.authority,
            payout_token,
            payment_tag,
            token_program: TOKEN,
            associated_token_program: ATA,
            system_program: SYSTEM,
        }
        .instruction(ClaimInstructionArgs {
            proof: vec![0; 388].into(),
            public_values,
        })
    }
    pub fn machine_state(&self) -> Machine {
        Machine::read(&self.h.svm.get_account(&self.machine).unwrap().data).unwrap()
    }
    pub fn reservation_state(&self, nonce: [u8; 32]) -> Reservation {
        Reservation::read(
            &self
                .h
                .svm
                .get_account(&self.reservation(nonce))
                .unwrap()
                .data,
        )
        .unwrap()
    }
    pub fn tip(&self) -> u32 {
        self.branch.tip_height()
    }
    pub fn root(&self) -> [u8; 32] {
        let data = self.h.relay_data(&self.relay);
        RelayView::parse(
            &HEADER_RELAY_PROGRAM,
            self.relay.as_array(),
            self.relay.as_array(),
            &data,
        )
        .unwrap()
        .latest_root()
    }
    pub fn mine(&mut self, n: u32) {
        for _ in 0..n {
            let b = self.branch.next(self.branch.tip_height());
            self.h.relay_synthetic(&self.relay, &b.raw).unwrap();
        }
    }
    pub fn time(&mut self, t: i64) {
        let mut c = self.h.svm.get_sysvar::<Clock>();
        c.unix_timestamp = t;
        self.h.svm.set_sysvar(&c);
    }
    pub fn halt(&mut self) {
        let mut a = self.h.svm.get_account(&self.relay).unwrap();
        a.data[header_relay_interface::R_HALTED] = 1;
        self.h.svm.set_account(self.relay, a).unwrap();
    }
}
pub fn assert_error(r: Result<TransactionMetadata, FailedTransactionMetadata>, e: VendingError) {
    use solana_instruction::error::InstructionError;
    use solana_transaction_error::TransactionError;
    let err = r.unwrap_err();
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::Custom(e as u32)),
        "{}",
        err.meta.pretty_logs()
    );
}
