//! The Payee's side: read the Payer's PCZT, check it, commit to it and lock the tokens.

use std::{os::unix::fs::OpenOptionsExt, path::PathBuf};

use anyhow::{Context, Result, ensure};
use escrow_client::{
    accounts::Escrow,
    instructions::{CreateEscrowBuilder, RefundBuilder, RequestRefundBuilder},
};
use solana_address::Address;
use solana_signer::Signer;
use zolar_pczt::{Network, inspect};
use zolar_relayer::rpc::HttpRpc;

use crate::{
    explorer::Explorer,
    internal_txid, random_salt,
    solana::{
        RefundStep, associated_token_account, create_ata_idempotent, load_keypair,
        parse_token_account, read_account, read_escrow, read_relay, refund_step, send,
    },
};

/// Expiry is at most this many blocks above the relay tip (the Escrow enforces the same bound).
const MAX_EXPIRY_AHEAD: u32 = 20;

pub struct CreateEscrow {
    pub rpc_url: String,
    pub explorer_url: String,
    pub keypair: PathBuf,
    pub relay: Address,
    pub pczt: PathBuf,
    pub ufvk_file: PathBuf,
    /// The text the Payee asked the Payer to put in the memo (the replay challenge).
    pub expect_memo: String,
    /// The least the PCZT must pay this Payee, in zatoshis.
    pub min_zats: u64,
    /// The Payer's wallet on Solana.
    pub payout: Address,
    pub mint: Address,
    /// Tokens to lock, in the mint's smallest unit.
    pub amount: u64,
    /// Where the salt is written for the Payer (mode 0600).
    pub salt_out: PathBuf,
    /// Tokens of the mint the Payer must lock before a Claim can settle (0: none).
    pub bond: u64,
}

pub struct Created {
    pub escrow: Address,
    pub signature: String,
    pub txid: String,
    pub expiry_height: u32,
}

pub fn create_escrow(a: &CreateEscrow) -> Result<Created> {
    ensure!(a.min_zats > 0, "--min-zats must be positive");
    let pczt = std::fs::read(&a.pczt).with_context(|| format!("reading {}", a.pczt.display()))?;
    let ufvk = std::fs::read_to_string(&a.ufvk_file)
        .with_context(|| format!("reading {}", a.ufvk_file.display()))?;
    let report = inspect(&pczt, ufvk.trim(), Network::Testnet)?;

    println!("PCZT transaction ID : {}", report.txid);
    println!("transaction version : {}", report.tx_version);
    println!("expiry height       : {}", report.expiry_height);
    for out in &report.received {
        println!(
            "pays this viewing key: {} zatoshis ({:.8} ZEC) in {}, memo {:?}",
            out.zatoshis,
            out.zatoshis as f64 / 1e8,
            out.pool,
            out.memo
        );
    }
    let paid: u64 = report
        .received
        .iter()
        .filter(|o| o.memo.as_deref() == Some(a.expect_memo.as_str()))
        .map(|o| o.zatoshis)
        .sum();
    ensure!(
        paid >= a.min_zats,
        "the PCZT pays {paid} zatoshis with the memo {:?}; at least {} are required. Without the \
         challenge memo the payment could be an old one",
        a.expect_memo,
        a.min_zats
    );

    let mut rpc = HttpRpc::new(&a.rpc_url);
    let relay = read_relay(&mut rpc, &a.relay)?;
    let view = relay.view()?;
    view.ensure_running()
        .map_err(|e| anyhow::anyhow!("the relay is not running: {e:?}"))?;
    let tip = view.tip_height();
    println!("relay tip {tip}, Finality Depth {}", view.finality_depth());
    ensure!(
        report.expiry_height > tip && report.expiry_height <= tip + MAX_EXPIRY_AHEAD,
        "expiry {} is not within (relay tip {tip}, {}]; the Payer must rebuild the PCZT with a shorter expiry",
        report.expiry_height,
        tip + MAX_EXPIRY_AHEAD
    );
    println!(
        "the Payer has until block {} (about {} s after the relay tip)",
        report.expiry_height,
        (report.expiry_height - tip) * 25
    );

    let explorer = Explorer::new(&a.explorer_url);
    ensure!(
        !explorer.knows(&report.txid)?,
        "the explorer already knows transaction {}: it is not a new payment",
        report.txid
    );

    let txid = internal_txid(&report.txid)?;
    let salt = random_salt();
    let commitment = zolar_claim::payment_commitment(&txid, &salt)?;

    let payee = load_keypair(&a.keypair)?;
    let source = associated_token_account(&payee.pubkey(), &a.mint);
    let token = parse_token_account(&read_account(&mut rpc, &source)?)
        .context("the Payee has no token account for this mint")?;
    ensure!(
        token.amount >= a.amount,
        "the Payee holds {} tokens, fewer than the {} to lock",
        token.amount,
        a.amount
    );

    // The salt goes to disk before any transaction so a crash cannot lose the only copy.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&a.salt_out)
        .with_context(|| format!("creating {} (it must not exist)", a.salt_out.display()))?;
    std::io::Write::write_all(&mut file, hex::encode(salt).as_bytes())?;

    let escrow = Escrow::find_pda(&payee.pubkey(), commitment).0;
    let vault = associated_token_account(&escrow, &a.mint);
    let create_vault = create_ata_idempotent(&payee.pubkey(), &escrow, &a.mint);
    let create = CreateEscrowBuilder::new()
        .payee(payee.pubkey())
        .escrow(escrow)
        .payee_token(source)
        .vault(vault)
        .relay(a.relay)
        .commitment(commitment)
        .payout(a.payout)
        .amount(a.amount)
        .bond_required(a.bond)
        .expiry_height(report.expiry_height)
        .claim_mode(0)
        .instruction();
    let signature = send(&mut rpc, &[create_vault, create], &payee)?;
    println!("Escrow created      : {escrow}");
    println!("signature           : {signature}");
    println!("commitment          : {}", hex::encode(commitment));
    println!("Bond required       : {}", a.bond);
    println!(
        "salt written to {} (give it to the Payer privately)",
        a.salt_out.display()
    );
    Ok(Created {
        escrow,
        signature,
        txid: report.txid,
        expiry_height: report.expiry_height,
    })
}

pub struct RefundArgs {
    pub rpc_url: String,
    pub keypair: PathBuf,
    pub escrow: Address,
    /// Keep going through the claim window instead of stopping when a step must wait.
    pub wait: bool,
}

/// Where a refund stands after [`refund`] returned.
pub struct Refunded {
    pub request_signature: Option<String>,
    pub refund_signature: Option<String>,
}

fn unix_now() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64)
}

/// Sends the transaction. With `wait`, a refusal because the Escrow is not refundable yet (the local
/// clock runs ahead of Solana's) is not an error: `None` means look again.
fn send_or_wait(
    rpc: &mut HttpRpc,
    instructions: &[solana_instruction::Instruction],
    payee: &solana_keypair::Keypair,
    wait: bool,
) -> Result<Option<String>> {
    match send(rpc, instructions, payee) {
        Ok(signature) => Ok(Some(signature)),
        Err(e)
            if wait
                && (e.to_string().contains("ClaimWindowOpen")
                    || e.to_string().contains("RefundNotYet")) =>
        {
            println!("Solana's clock says not yet: {e}");
            std::thread::sleep(std::time::Duration::from_secs(10));
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Takes an unclaimed Escrow back in the two steps the program requires: RequestRefund once the
/// relay tip is above the expiry height plus Finality Depth (or 24 hours after creation), then Refund
/// after the claim window. The whole vault, Bond included, returns to the Payee.
pub fn refund(a: &RefundArgs) -> Result<Refunded> {
    let mut rpc = HttpRpc::new(&a.rpc_url);
    let payee = load_keypair(&a.keypair)?;
    let mut done = Refunded {
        request_signature: None,
        refund_signature: None,
    };
    loop {
        let escrow = read_escrow(&read_account(&mut rpc, &a.escrow)?)?;
        ensure!(
            escrow.payee == payee.pubkey(),
            "{} was created by {}, not by this keypair",
            a.escrow,
            escrow.payee
        );
        let relay = read_relay(&mut rpc, &escrow.relay)?;
        let view = relay.view()?;
        let step = refund_step(
            &escrow,
            view.tip_height(),
            view.finality_depth(),
            view.is_halted(),
            unix_now()?,
        );
        println!("refund step: {step:?}");
        match step {
            RefundStep::Request => {
                let ix = RequestRefundBuilder::new()
                    .escrow(a.escrow)
                    .relay(escrow.relay)
                    .instruction();
                let Some(signature) = send_or_wait(&mut rpc, &[ix], &payee, a.wait)? else {
                    continue;
                };
                println!(
                    "refund requested    : {signature} (a Payer may still claim for {} s)",
                    crate::solana::CLAIM_WINDOW_SECS
                );
                done.request_signature = Some(signature);
            }
            RefundStep::Refund => {
                let destination = associated_token_account(&escrow.payee, &escrow.mint);
                let held = parse_token_account(&read_account(&mut rpc, &escrow.vault)?)?.amount;
                // The Payee may have closed its token account after locking everything.
                let create = create_ata_idempotent(&payee.pubkey(), &escrow.payee, &escrow.mint);
                let ix = RefundBuilder::new()
                    .escrow(a.escrow)
                    .relay(escrow.relay)
                    .vault(escrow.vault)
                    .payee_token(destination)
                    .payee(escrow.payee)
                    .instruction();
                let Some(signature) = send_or_wait(&mut rpc, &[create, ix], &payee, a.wait)? else {
                    continue;
                };
                println!("refunded            : {signature}");
                println!(
                    "returned to {destination}: {held} tokens ({} locked, {} Bond)",
                    escrow.amount, escrow.bond_deposited
                );
                done.refund_signature = Some(signature);
                return Ok(done);
            }
            RefundStep::WaitForTip { .. }
            | RefundStep::Halted { .. }
            | RefundStep::WaitForWindow { .. } => {
                ensure!(
                    a.wait,
                    "not yet; run again later, or pass --wait to keep going"
                );
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        }
    }
}
