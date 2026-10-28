use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;
use sp1_solana_v6::Error;
use std::path::PathBuf;

const PROOF: &[u8; 356] = include_bytes!("../../fixtures/proof.bin");
const VALUES: &[u8] = include_bytes!("../../fixtures/public_values.bin");
const VKEY: &[u8; 32] = include_bytes!("../../fixtures/vkey_hash.bin");
const PROGRAM: Address = Address::new_from_array([42; 32]);

fn instruction(proof: &[u8], values: &[u8], vkey: &[u8; 32]) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![],
        data: [vkey.as_slice(), proof, values].concat(),
    }
}

fn run(ix: Instruction) -> Result<u64, TransactionError> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../program/target/deploy/sp1_v6_test_program.so");
    assert!(
        path.exists(),
        "build the test program with cargo build-sbf first"
    );
    let payer = Keypair::new();
    let mut svm = LiteSVM::new();
    svm.add_program_from_file(PROGRAM, path).unwrap();
    svm.airdrop(&payer.pubkey(), 1_000_000).unwrap();
    let budget = Instruction {
        program_id: Address::from_str_const("ComputeBudget111111111111111111111111111111"),
        accounts: vec![],
        data: [vec![2], 1_400_000u32.to_le_bytes().to_vec()].concat(),
    };
    let tx = Transaction::new(
        &[&payer],
        Message::new(&[budget, ix], Some(&payer.pubkey())),
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx)
        .map(|meta| meta.compute_units_consumed - 150)
        .map_err(|failed| {
            println!("CU SP1 rejected {}", failed.meta.compute_units_consumed);
            eprintln!("SBF failure logs: {:?}", failed.meta.logs);
            failed.err
        })
}

#[test]
fn vending_sized_public_values_reach_portable_blake3_and_reject_cleanly() {
    assert_eq!(
        run(instruction(PROOF, &[42; 249], VKEY)),
        Err(custom(Error::VerificationFailed))
    );
}

fn custom(error: Error) -> TransactionError {
    TransactionError::InstructionError(1, InstructionError::Custom(error as u32 + 1))
}

#[test]
fn verifies_real_proof_in_sbf_and_reports_cu() {
    let cu = run(instruction(PROOF, VALUES, VKEY)).unwrap();
    println!("CU SP1 v6 Groth16: {cu}");
    assert!(
        cu < 200_000,
        "SP1 verification exceeded the 200k CU budget: {cu}"
    );
}

#[test]
fn rejects_tampered_proof_public_values_and_vkey_in_sbf() {
    let mut proof = *PROOF;
    proof[355] ^= 1;
    assert!(run(instruction(&proof, VALUES, VKEY)).is_err());
    let mut values = VALUES.to_vec();
    values[0] ^= 1;
    assert_eq!(
        run(instruction(PROOF, &values, VKEY)),
        Err(custom(Error::VerificationFailed))
    );
    let mut vkey = *VKEY;
    vkey[31] ^= 1;
    assert_eq!(
        run(instruction(PROOF, VALUES, &vkey)),
        Err(custom(Error::VerificationFailed))
    );
}

#[test]
fn rejects_nonzero_exit_code_untrusted_root_and_nonce_in_sbf() {
    for (offset, expected) in [
        (35, Error::ExitCodeMismatch),
        (67, Error::VkeyRootMismatch),
        (99, Error::VerificationFailed),
    ] {
        let mut proof = *PROOF;
        proof[offset] ^= 1;
        assert_eq!(
            run(instruction(&proof, VALUES, VKEY)),
            Err(custom(expected))
        );
    }
}

#[test]
fn rejects_short_adapter_instruction_in_sbf() {
    let mut ix = instruction(PROOF, VALUES, VKEY);
    ix.data.truncate(31);
    assert_eq!(
        run(ix),
        Err(TransactionError::InstructionError(
            1,
            InstructionError::InvalidInstructionData
        ))
    );
}
