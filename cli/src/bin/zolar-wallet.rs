//! The customer's local helper for a Zolar checkout, standing in for a Zcash wallet that supports
//! Zolar. See `zolar_cli::wallet`.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use zolar_cli::{
    DEVNET_RPC, Network, default_relay,
    devtool::Devtool,
    prover::Prover,
    solana::parse_address,
    wallet::{self, Config, Wallet},
};

/// `circuits/scripts/prove.sh` relative to the repository this binary was built from.
const DEFAULT_PROVE_SCRIPT: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../circuits/scripts/prove.sh");

#[derive(Parser)]
#[command(
    name = "zolar-wallet",
    about = "Local wallet helper that pays Zolar checkouts (listens on 127.0.0.1 only)"
)]
struct Cli {
    #[arg(long, env = "ZOLAR_WALLET_PORT", default_value_t = 7878)]
    port: u16,
    /// Shop page origin that may ask this wallet to pay (repeat for more), e.g. http://localhost:5175.
    #[arg(long = "allow-origin", required = true)]
    allowed_origins: Vec<String>,
    #[arg(long, env = "ZOLAR_NETWORK", default_value = "testnet")]
    network: Network,
    #[arg(long, env = "ZOLAR_RPC_URL", default_value = DEVNET_RPC)]
    rpc_url: String,
    /// The Header Relay this wallet trusts (default: the devnet relay of the chosen Zcash network).
    #[arg(long, env = "ZOLAR_RELAY")]
    relay: Option<String>,
    /// Solana keypair file that pays the claim's fees (the item goes to the order's payout wallet).
    #[arg(long, env = "ZOLAR_PAYER_KEYPAIR")]
    fee_payer: PathBuf,
    /// The zcash-devtool binary (built with the `--expiry-height` patch).
    #[arg(long, env = "ZOLAR_DEVTOOL")]
    devtool: PathBuf,
    /// The zcash-devtool wallet directory that pays.
    #[arg(long, env = "ZOLAR_ZCASH_WALLET")]
    wallet_dir: PathBuf,
    /// The age identity file of that wallet.
    #[arg(long, env = "ZOLAR_ZCASH_IDENTITY")]
    identity: PathBuf,
    #[arg(long, env = "ZOLAR_PROVE_SCRIPT", default_value = DEFAULT_PROVE_SCRIPT)]
    prove_script: PathBuf,
    /// Directory with the circuit artifacts (`build/d8/`); see circuits/README.md.
    #[arg(long, env = "ZOLAR_DATA")]
    zolar_data: PathBuf,
    #[arg(long, env = "ZOLAR_MEM_MAX", default_value = "2500M")]
    mem_max: String,
    /// Where checkouts are kept (they hold salts: keep it private).
    #[arg(long, env = "ZOLAR_WALLET_DATA", default_value = ".zolar-wallet")]
    data_dir: PathBuf,
    /// The most one checkout may cost, in zatoshis (default 0.01 ZEC).
    #[arg(long, default_value_t = 1_000_000)]
    max_price_zats: u64,
    /// `scripts/check-txid-absent.sh`, run after each claim to show the privacy check on the page.
    #[arg(long, env = "ZOLAR_TXID_CHECK")]
    txid_check: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let relay = cli
        .relay
        .clone()
        .unwrap_or_else(|| default_relay(cli.network).to_string());
    let (wallet, queue) = Wallet::new(Config {
        port: cli.port,
        allowed_origins: cli.allowed_origins,
        network: cli.network,
        rpc_url: cli.rpc_url,
        relay: parse_address(&relay)?,
        fee_payer: cli.fee_payer,
        devtool: Devtool {
            binary: cli.devtool,
            wallet_dir: cli.wallet_dir,
            identity: cli.identity,
        },
        prover: Prover {
            script: cli.prove_script,
            data_dir: cli.zolar_data,
            depth: 8,
            mem_max: cli.mem_max,
        },
        data_dir: cli.data_dir,
        max_price_zats: cli.max_price_zats,
        txid_check: cli.txid_check,
    })?;
    wallet::serve(wallet, queue)
}
