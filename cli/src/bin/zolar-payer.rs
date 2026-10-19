//! Payer tool: build the PCZT through zcash-devtool, check the Escrow, pay, wait for finality, claim.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use solana_signer::Signer;
use zolar_cli::{
    DEVNET_RPC, Network, default_explorer, default_relay,
    devtool::Devtool,
    payer::{self, Chains, Check, Claim},
    read_salt,
    solana::{load_keypair, parse_address},
};

#[derive(Parser)]
#[command(
    name = "zolar-payer",
    about = "Payer side of a Zolar swap (Zcash testnet or mainnet, Solana devnet)"
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
    /// The Header Relay (default: the devnet relay of the chosen Zcash network).
    #[arg(long, env = "ZOLAR_RELAY", global = true)]
    relay: Option<String>,
    /// The block explorer (default: the one of the chosen Zcash network).
    #[arg(long, env = "ZOLAR_EXPLORER", global = true)]
    explorer: Option<String>,
    /// The Zcash network of the payment: testnet or mainnet.
    #[arg(long, env = "ZOLAR_NETWORK", default_value = "testnet", global = true)]
    network: Network,
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
    /// The most tokens the Payer agreed to lock as a Bond (0: the Escrow must require none).
    #[arg(long, default_value_t = 0)]
    max_bond: u64,
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
        /// The Payee's unified address (of the chosen network).
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
    /// Lock the Bond the Escrow requires (`pay` does this itself before it broadcasts).
    DepositBond {
        #[arg(long)]
        escrow: String,
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
    let network = cli.net.network;
    let relay = cli
        .net
        .relay
        .clone()
        .unwrap_or_else(|| default_relay(network).to_string());
    let chains = Chains {
        rpc_url: cli.net.rpc_url.clone(),
        explorer_url: cli
            .net
            .explorer
            .clone()
            .unwrap_or_else(|| default_explorer(network).to_string()),
        relay: parse_address(&relay)?,
        peers: zolar_cli::seeders(network),
        network,
    };
    let wallet = || -> Result<solana_address::Address> {
        let path = cli
            .net
            .keypair
            .as_ref()
            .context("--keypair (or ZOLAR_PAYER_KEYPAIR) is required")?;
        Ok(load_keypair(path)?.pubkey())
    };
    let keypair_path = || -> Result<PathBuf> {
        cli.net
            .keypair
            .clone()
            .context("--keypair (or ZOLAR_PAYER_KEYPAIR) is required")
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
                max_bond: terms.max_bond,
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
                max_bond: terms.max_bond,
                finalized_wait: std::time::Duration::from_secs(terms.finalized_wait_secs),
            })?;
            payer::deposit_bond(&chains, &keypair_path()?, parse_address(&escrow)?)?;
            let height = payer::pay(&chains, &zcash.tool(), &checked, &pczt, &work_dir)?;
            println!("payment {} mined in block {height}", checked.txid);
        }
        Command::DepositBond { escrow } => {
            payer::deposit_bond(&chains, &keypair_path()?, parse_address(&escrow)?)?;
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
