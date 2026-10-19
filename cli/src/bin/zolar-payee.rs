//! Payee tool: read the Payer's PCZT, check it with a viewing key, commit to it and lock the tokens.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use zolar_cli::{
    DEVNET_RELAY, DEVNET_RPC, TESTNET_EXPLORER,
    payee::{CreateEscrow, RefundArgs, create_escrow, refund},
    solana::parse_address,
};

#[derive(Parser)]
#[command(
    name = "zolar-payee",
    about = "Payee side of a Zolar swap (Zcash testnet, Solana devnet)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Decrypt the PCZT with the viewing key, show the amount and expiry, commit to the transaction
    /// ID with a fresh salt and lock the tokens in an Escrow.
    CreateEscrow {
        /// The unsigned PCZT the Payer built.
        #[arg(long)]
        pczt: PathBuf,
        /// File holding the Payee's unified full viewing key (testnet).
        #[arg(long)]
        ufvk_file: PathBuf,
        /// The memo the Payer must have put in the payment: a fresh challenge for this trade.
        #[arg(long)]
        expect_memo: String,
        /// The least the payment must pay this viewing key, in zatoshis.
        #[arg(long)]
        min_zats: u64,
        /// The Payer's Solana wallet, which receives the tokens.
        #[arg(long)]
        payout: String,
        /// The SPL token mint.
        #[arg(long)]
        mint: String,
        /// Tokens to lock, in the mint's smallest unit.
        #[arg(long)]
        amount: u64,
        /// The Payee's Solana keypair file (pays and signs).
        #[arg(long, env = "ZOLAR_PAYEE_KEYPAIR")]
        keypair: PathBuf,
        /// Where to write the salt for the Payer (must not exist).
        #[arg(long)]
        salt_out: PathBuf,
        /// Bond the Payer must lock before a Claim can settle, in tokens of the mint. A Payer who
        /// never pays forfeits it to the Payee at the refund.
        #[arg(long, default_value_t = 0)]
        bond: u64,
        /// Refuse the Reveal Claim (which publishes the transaction ID): only a Private Claim settles.
        #[arg(long)]
        private_only: bool,
        #[arg(long, env = "ZOLAR_RPC_URL", default_value = DEVNET_RPC)]
        rpc_url: String,
        #[arg(long, env = "ZOLAR_RELAY", default_value = DEVNET_RELAY)]
        relay: String,
        #[arg(long, env = "ZOLAR_EXPLORER", default_value = TESTNET_EXPLORER)]
        explorer: String,
    },
    /// Take an unclaimed Escrow back, Bond included: request the refund once the relay tip is above
    /// the expiry height plus Finality Depth (or 24 hours after creation), then refund after the
    /// 15 minute claim window.
    Refund {
        #[arg(long)]
        escrow: String,
        /// The Payee's Solana keypair file (pays and signs).
        #[arg(long, env = "ZOLAR_PAYEE_KEYPAIR")]
        keypair: PathBuf,
        /// Wait for the tip, the Halt backstop and the claim window instead of stopping.
        #[arg(long)]
        wait: bool,
        #[arg(long, env = "ZOLAR_RPC_URL", default_value = DEVNET_RPC)]
        rpc_url: String,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::CreateEscrow {
            pczt,
            ufvk_file,
            expect_memo,
            min_zats,
            payout,
            mint,
            amount,
            keypair,
            salt_out,
            bond,
            private_only,
            rpc_url,
            relay,
            explorer,
        } => {
            create_escrow(&CreateEscrow {
                rpc_url,
                explorer_url: explorer,
                keypair,
                relay: parse_address(&relay)?,
                pczt,
                ufvk_file,
                expect_memo,
                min_zats,
                payout: parse_address(&payout)?,
                mint: parse_address(&mint)?,
                amount,
                salt_out,
                bond,
                private_only,
            })?;
        }
        Command::Refund {
            escrow,
            keypair,
            wait,
            rpc_url,
        } => {
            refund(&RefundArgs {
                rpc_url,
                keypair,
                escrow: parse_address(&escrow)?,
                wait,
            })?;
        }
    }
    Ok(())
}
