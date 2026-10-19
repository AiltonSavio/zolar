//! Rehearsal for ticket 13, no Zcash funds involved: lock test tokens in a devnet Escrow that
//! commits to a transaction ALREADY mined on Zcash mainnet (the last transaction of a block a few
//! blocks behind the relay tip), wait until the block is final, and send the Reveal Claim as the
//! Payer. It exercises the mainnet relay, the mainnet P2P headers, the mainnet explorer and the
//! Final Block Tree path on real blocks before anyone sends a real payment.
//!
//! `ZOLAR_NETWORK=mainnet ZOLAR_RELAY=<mainnet relay> cargo run --release --example rehearse_claim -- \
//!    --payee-keypair P --payer-keypair Q --mint M --amount N --salt-out FILE`

use std::{os::unix::fs::OpenOptionsExt, path::PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use escrow_client::instructions::CreateEscrowBuilder;
use solana_signer::Signer;
use zolar_cli::{
    Network, default_explorer, default_relay, display_txid,
    explorer::Explorer,
    internal_txid,
    payer::{Chains, Claim, reveal_claim},
    random_salt, seeders,
    solana::{
        associated_token_account, create_ata_idempotent, load_keypair, parse_address, read_relay,
        send,
    },
};
use zolar_relayer::rpc::HttpRpc;

#[derive(Parser)]
struct Args {
    #[arg(long, env = "ZOLAR_NETWORK", default_value = "mainnet")]
    network: Network,
    #[arg(long, env = "ZOLAR_RPC_URL", default_value = zolar_cli::DEVNET_RPC)]
    rpc_url: String,
    #[arg(long, env = "ZOLAR_RELAY")]
    relay: Option<String>,
    #[arg(long, env = "ZOLAR_EXPLORER")]
    explorer: Option<String>,
    #[arg(long)]
    payee_keypair: PathBuf,
    #[arg(long)]
    payer_keypair: PathBuf,
    #[arg(long)]
    mint: String,
    #[arg(long)]
    amount: u64,
    #[arg(long)]
    salt_out: PathBuf,
    /// How many blocks behind the relay tip the transaction's block is.
    #[arg(long, default_value_t = 2)]
    behind: u32,
}

fn main() -> Result<()> {
    let a = Args::parse();
    let relay = parse_address(
        &a.relay
            .unwrap_or_else(|| default_relay(a.network).to_string()),
    )?;
    let explorer_url = a
        .explorer
        .unwrap_or_else(|| default_explorer(a.network).to_string());
    let mint = parse_address(&a.mint)?;
    let payee = load_keypair(&a.payee_keypair)?;
    let payer = load_keypair(&a.payer_keypair)?;
    let mut rpc = HttpRpc::new(&a.rpc_url);

    let state = read_relay(&mut rpc, &relay)?;
    let view = state.view()?;
    view.ensure_running()
        .map_err(|e| anyhow::anyhow!("relay halted: {e:?}"))?;
    let tip = view.tip_height();
    let height = tip - a.behind;
    let explorer = Explorer::new(&explorer_url);
    let txids = explorer.block_txids(height)?;
    let txid = txids.last().context("block without transactions")?.clone();
    ensure!(
        txids.len() > 1 || a.behind > 0,
        "need a block with a payment"
    );
    println!(
        "relay tip {tip}; rehearsing with transaction {txid} (index {} of {}) of mainnet block {height}",
        txids.len() - 1,
        txids.len()
    );

    let salt = random_salt();
    let commitment = zolar_claim::payment_commitment(&internal_txid(&txid)?, &salt)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&a.salt_out)
        .with_context(|| format!("creating {}", a.salt_out.display()))?;
    std::io::Write::write_all(&mut file, hex::encode(salt).as_bytes())?;

    let escrow = escrow_client::accounts::Escrow::find_pda(&payee.pubkey(), commitment).0;
    let vault = associated_token_account(&escrow, &mint);
    let create = CreateEscrowBuilder::new()
        .payee(payee.pubkey())
        .escrow(escrow)
        .payee_token(associated_token_account(&payee.pubkey(), &mint))
        .vault(vault)
        .relay(relay)
        .commitment(commitment)
        .payout(payer.pubkey())
        .amount(a.amount)
        .bond_required(0)
        .expiry_height(tip + 10)
        .claim_mode(0)
        .instruction();
    let signature = send(
        &mut rpc,
        &[
            create_ata_idempotent(&payee.pubkey(), &escrow, &mint),
            create,
        ],
        &payee,
    )?;
    println!("Escrow {escrow} created: {signature}");

    let chains = Chains {
        rpc_url: a.rpc_url.clone(),
        explorer_url,
        relay,
        peers: seeders(a.network),
        network: a.network,
    };
    let claim = reveal_claim(&Claim {
        chains: &chains,
        keypair: a.payer_keypair,
        escrow_address: escrow,
        txid: &txid,
        salt,
    })?;
    println!(
        "rehearsal claim {claim} of {}",
        display_txid(&internal_txid(&txid)?)
    );
    Ok(())
}
