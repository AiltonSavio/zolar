//! Payer tool: build the PCZT through zcash-devtool, check the Escrow, pay, wait for finality, claim.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use solana_signer::Signer;
use zolar_cli::{
    DEVNET_RPC, Network, default_relay,
    devtool::Devtool,
    payer::{self, Chains, Check, Claim, PrivateClaimArgs},
    prover::Prover,
    read_salt,
    solana::{load_keypair, parse_address},
};

/// `circuits/scripts/prove.sh` relative to the repository this binary was built from.
const DEFAULT_PROVE_SCRIPT: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../circuits/scripts/prove.sh");

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
    /// Accept a private-only Escrow (no Reveal Claim). A payment mined in a block of more than 256
    /// transactions can then never be claimed, and the Payee gets the tokens back.
    #[arg(long)]
    accept_private_only: bool,
    /// Seconds to wait for the Escrow to be finalized on Solana before paying.
    #[arg(long, default_value_t = 120)]
    finalized_wait_secs: u64,
}

impl Terms {
    fn agreed(&self) -> Result<payer::Terms> {
        Ok(payer::Terms {
            mint: parse_address(&self.mint)?,
            min_amount: self.min_amount,
            max_bond: self.max_bond,
            accept_private_only: self.accept_private_only,
        })
    }
}

#[derive(Subcommand)]
enum Command {
    /// Build the unsigned PCZT, and the redacted copy the Payee inspects (`<output>` with the extension
    /// `.payee.pczt`).
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
    /// Check the Escrow, prove and sign, lock the Bond if one is required, then broadcast.
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
        #[command(flatten)]
        terms: Terms,
        #[arg(long)]
        escrow: String,
    },
    /// Wait until the payment's block is in the Final Block Tree and send the Reveal Claim. This puts
    /// the Zcash transaction ID and the salt on Solana; use `private-claim` to keep them off.
    Claim {
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        salt_file: PathBuf,
    },
    /// Wait until the payment's block is in the Final Block Tree, prove it on this machine and send a
    /// Private Claim: only the proof and a root reach Solana, never the transaction ID or block.
    PrivateClaim {
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        salt_file: PathBuf,
        /// `circuits/scripts/prove.sh` of this repository (runs the witness and the prover locally).
        #[arg(long, env = "ZOLAR_PROVE_SCRIPT", default_value = DEFAULT_PROVE_SCRIPT)]
        prove_script: PathBuf,
        /// Directory with the large circuit artifacts (`build/d8/`: wasm, R1CS, proving key,
        /// verification key). Not in git; see circuits/README.md.
        #[arg(long, env = "ZOLAR_DATA")]
        zolar_data: PathBuf,
        /// Memory cap for each proving step (the prover peaks at about 1.8 GiB).
        #[arg(long, env = "ZOLAR_MEM_MAX", default_value = "2500M")]
        mem_max: String,
        /// Directory for the proof's temporary files; the witness is deleted after proving (default:
        /// `$XDG_RUNTIME_DIR/zolar-work`, a tmpfs, else `.zolar-work`).
        #[arg(long, env = "ZOLAR_WORK_DIR")]
        work_dir: Option<PathBuf>,
        /// Times to prove again when the root ages out of the relay's last 128 while proving.
        #[arg(long, default_value_t = 3)]
        max_attempts: u32,
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
                terms: terms.agreed()?,
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
                terms: terms.agreed()?,
                finalized_wait: std::time::Duration::from_secs(terms.finalized_wait_secs),
            })?;
            let devtool = zcash.tool();
            // The Bond is locked only once a signed transaction exists: a failed proof or signature
            // must not leave it in an Escrow that will be refunded.
            let signed = payer::sign(&devtool, &checked, &pczt, &work_dir)?;
            payer::deposit_bond(
                &chains,
                &keypair_path()?,
                checked.escrow_address,
                &terms.agreed()?,
            )?;
            payer::broadcast(&chains, &devtool, &checked, &signed)?;
            println!(
                "payment {} broadcast; run `claim` or `private-claim` once its block is final",
                checked.txid
            );
        }
        Command::DepositBond { terms, escrow } => {
            payer::deposit_bond(
                &chains,
                &keypair_path()?,
                parse_address(&escrow)?,
                &terms.agreed()?,
            )?;
        }
        Command::Claim {
            escrow,
            pczt,
            salt_file,
        } => {
            let txid = zolar_pczt::summary(&std::fs::read(&pczt)?)?.txid;
            payer::reveal_claim(&Claim {
                chains: &chains,
                keypair: keypair_path()?,
                payout: wallet()?,
                escrow_address: parse_address(&escrow)?,
                txid: &txid,
                salt: read_salt(&salt_file)?,
            })?;
        }
        Command::PrivateClaim {
            escrow,
            pczt,
            salt_file,
            prove_script,
            zolar_data,
            mem_max,
            work_dir,
            max_attempts,
        } => {
            let txid = zolar_pczt::summary(&std::fs::read(&pczt)?)?.txid;
            let prover = Prover {
                script: prove_script,
                data_dir: zolar_data,
                depth: 8,
                mem_max,
            };
            payer::private_claim(&PrivateClaimArgs {
                claim: Claim {
                    chains: &chains,
                    keypair: keypair_path()?,
                    payout: wallet()?,
                    escrow_address: parse_address(&escrow)?,
                    txid: &txid,
                    salt: read_salt(&salt_file)?,
                },
                prover: &prover,
                work_dir: work_dir.unwrap_or_else(zolar_cli::prover::default_work_dir),
                max_attempts,
                on_proving: None,
            })?;
        }
    }
    Ok(())
}
