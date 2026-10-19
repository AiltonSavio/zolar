//! Payer tool: build the PCZT through zcash-devtool, check the Escrow, pay, wait for finality, claim.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use solana_signer::Signer;
use zolar_cli::{
    DEVNET_RELAY, DEVNET_RPC, TESTNET_EXPLORER,
    devtool::Devtool,
    payer::{self, Chains, Check, Claim},
    read_salt,
    solana::{load_keypair, parse_address},
};
use zolar_relayer::p2p::SEEDERS;

#[derive(Parser)]
#[command(
    name = "zolar-payer",
    about = "Payer side of a Zolar swap (Zcash testnet, Solana devnet)"
)]
struct Cli {
    #[command(flatten)]
    net: Net,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Net {
    #[arg(long, env = "ZOLAR_RPC_URL", default_value = DEVNET_RPC, global = true)]
    rpc_url: String,
    #[arg(long, env = "ZOLAR_RELAY", default_value = DEVNET_RELAY, global = true)]
    relay: String,
    #[arg(long, env = "ZOLAR_EXPLORER", default_value = TESTNET_EXPLORER, global = true)]
    explorer: String,
    /// The Payer's Solana keypair file; its address is the payout wallet.
    #[arg(long, env = "ZOLAR_PAYER_KEYPAIR", global = true)]
    keypair: Option<PathBuf>,
}

#[derive(Args)]
struct Zcash {
    /// The zcash-devtool binary (built with the `--expiry-height` patch).
    #[arg(long, env = "ZOLAR_DEVTOOL")]
    devtool: PathBuf,
    /// The Payer's zcash-devtool wallet directory.
    #[arg(long, env = "ZOLAR_ZCASH_WALLET")]
    wallet_dir: PathBuf,
    /// The age identity file of that wallet.
    #[arg(long, env = "ZOLAR_ZCASH_IDENTITY")]
    identity: PathBuf,
}

impl Zcash {
    fn tool(&self) -> Devtool {
        Devtool {
            binary: self.devtool.clone(),
            wallet_dir: self.wallet_dir.clone(),
            identity: self.identity.clone(),
        }
    }
}

#[derive(Args)]
struct Terms {
    /// The token mint the Payer agreed to be paid in. Required: the Escrow's own mint is not trusted.
    #[arg(long)]
    mint: String,
    /// The least tokens (smallest unit of the mint) the Payer agreed to be paid.
    #[arg(long)]
    min_amount: u64,
    /// Seconds to wait for the Escrow to be finalized on Solana before paying.
    #[arg(long, default_value_t = 120)]
    finalized_wait_secs: u64,
}

#[derive(Subcommand)]
enum Command {
    /// Build the unsigned PCZT for the Payee to inspect.
    CreatePczt {
        #[command(flatten)]
        zcash: Zcash,
        /// The Payee's unified address (testnet).
        #[arg(long)]
        address: String,
        /// Zatoshis to pay.
        #[arg(long)]
        value: u64,
        /// The challenge memo the Payee asked for.
        #[arg(long)]
        memo: String,
        /// Blocks above the relay tip until the payment expires (1 to 20).
        #[arg(long, default_value_t = 18)]
        expiry_blocks: u32,
        #[arg(long)]
        output: PathBuf,
    },
    /// Read-only: rebuild the Final Block Tree from Zcash P2P headers and compare it with the relay.
    VerifyTree,
    /// Read-only: verify the Escrow matches this PCZT and salt.
    Check {
        #[command(flatten)]
        terms: Terms,
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        salt_file: PathBuf,
        #[arg(long)]
        accept_freeze_authority: bool,
    },
    /// Check the Escrow, then prove, sign and broadcast, and wait until the payment is mined.
    Pay {
        #[command(flatten)]
        zcash: Zcash,
        #[command(flatten)]
        terms: Terms,
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        salt_file: PathBuf,
        #[arg(long)]
        accept_freeze_authority: bool,
        /// Directory for the proven, signed and combined PCZTs.
        #[arg(long)]
        work_dir: PathBuf,
    },
    /// Wait until the payment's block is in the Final Block Tree and send the Reveal Claim.
    Claim {
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        salt_file: PathBuf,
        /// The block the payment was mined in (default: ask the explorer).
        #[arg(long)]
        block_height: Option<u32>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let chains = Chains {
        rpc_url: cli.net.rpc_url.clone(),
        explorer_url: cli.net.explorer.clone(),
        relay: parse_address(&cli.net.relay)?,
        peers: SEEDERS.iter().map(|s| s.to_string()).collect(),
    };
    let wallet = || -> Result<solana_address::Address> {
        let path = cli
            .net
            .keypair
            .as_ref()
            .context("--keypair (or ZOLAR_PAYER_KEYPAIR) is required")?;
        Ok(load_keypair(path)?.pubkey())
    };
    match cli.command {
        Command::CreatePczt {
            zcash,
            address,
            value,
            memo,
            expiry_blocks,
            output,
        } => payer::create_pczt(
            &chains,
            &zcash.tool(),
            &address,
            value,
            &memo,
            expiry_blocks,
            &output,
        )?,
        Command::VerifyTree => {
            let mut rpc = zolar_relayer::rpc::HttpRpc::new(&chains.rpc_url);
            let relay = zolar_cli::solana::read_relay(&mut rpc, &chains.relay)?;
            payer::rebuild_tree(&chains, &relay)?;
        }
        Command::Check {
            terms,
            escrow,
            pczt,
            salt_file,
            accept_freeze_authority,
        } => {
            payer::check_escrow(&Check {
                chains: &chains,
                escrow_address: parse_address(&escrow)?,
                payer_wallet: wallet()?,
                pczt: &pczt,
                salt: read_salt(&salt_file)?,
                accept_freeze_authority,
                expected_mint: parse_address(&terms.mint)?,
                min_amount: terms.min_amount,
                finalized_wait: std::time::Duration::from_secs(terms.finalized_wait_secs),
            })?;
        }
        Command::Pay {
            zcash,
            terms,
            escrow,
            pczt,
            salt_file,
            accept_freeze_authority,
            work_dir,
        } => {
            let checked = payer::check_escrow(&Check {
                chains: &chains,
                escrow_address: parse_address(&escrow)?,
                payer_wallet: wallet()?,
                pczt: &pczt,
                salt: read_salt(&salt_file)?,
                accept_freeze_authority,
                expected_mint: parse_address(&terms.mint)?,
                min_amount: terms.min_amount,
                finalized_wait: std::time::Duration::from_secs(terms.finalized_wait_secs),
            })?;
            let height = payer::pay(&chains, &zcash.tool(), &checked, &pczt, &work_dir)?;
            println!("payment {} mined in block {height}", checked.txid);
        }
        Command::Claim {
            escrow,
            pczt,
            salt_file,
            block_height,
        } => {
            let txid = zolar_pczt::summary(&std::fs::read(&pczt)?)?.txid;
            let height = match block_height {
                Some(h) => h,
                None => payer::wait_until_mined(&chains, &txid)?,
            };
            payer::reveal_claim(&Claim {
                chains: &chains,
                keypair: cli
                    .net
                    .keypair
                    .clone()
                    .context("--keypair (or ZOLAR_PAYER_KEYPAIR) is required")?,
                escrow_address: parse_address(&escrow)?,
                txid: &txid,
                block_height: height,
                salt: read_salt(&salt_file)?,
            })?;
        }
    }
    Ok(())
}
