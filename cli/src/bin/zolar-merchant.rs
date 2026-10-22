//! Merchant service for a Zolar checkout: orders, Escrows for proposed payments, refunds of abandoned
//! orders. See `zolar_cli::merchant`.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use zolar_cli::{
    DEVNET_RPC, Network,
    checkout::Item,
    default_relay,
    merchant::{self, Config, Merchant},
    solana::parse_address,
};

#[derive(Parser)]
#[command(
    name = "zolar-merchant",
    about = "Merchant (Payee) service of a Zolar checkout"
)]
struct Cli {
    /// Address to listen on.
    #[arg(long, env = "ZOLAR_MERCHANT_BIND", default_value = "127.0.0.1:7979")]
    bind: String,
    /// The merchant's name, shown in the customer's wallet.
    #[arg(long, default_value = "Proof of Party")]
    name: String,
    #[arg(long, env = "ZOLAR_NETWORK", default_value = "testnet")]
    network: Network,
    #[arg(long, env = "ZOLAR_RPC_URL", default_value = DEVNET_RPC)]
    rpc_url: String,
    /// The Header Relay Escrows trust (default: the devnet relay of the chosen Zcash network).
    #[arg(long, env = "ZOLAR_RELAY")]
    relay: Option<String>,
    /// The merchant's Solana keypair file: it holds the items and signs Escrows and refunds.
    #[arg(long, env = "ZOLAR_PAYEE_KEYPAIR")]
    keypair: PathBuf,
    /// The merchant's Zcash unified full viewing key, to check proposed payments.
    #[arg(long, env = "ZOLAR_MERCHANT_UFVK_FILE")]
    ufvk_file: PathBuf,
    /// The merchant's Zcash unified address (a file holding it).
    #[arg(long, env = "ZOLAR_MERCHANT_ADDRESS_FILE")]
    address_file: PathBuf,
    /// JSON array of items: {"id", "name", "mint", "amount", "price_zats"}.
    #[arg(long, env = "ZOLAR_MERCHANT_CATALOG")]
    catalog: PathBuf,
    /// Where orders are kept.
    #[arg(long, env = "ZOLAR_MERCHANT_DATA", default_value = ".zolar-merchant")]
    data_dir: PathBuf,
    /// Shop page origin allowed to create orders (repeat for more), e.g. http://localhost:5175.
    #[arg(long = "allow-origin", required = true)]
    allowed_origins: Vec<String>,
    #[arg(long, default_value_t = 10)]
    max_open_orders: usize,
    #[arg(long, default_value_t = 1)]
    max_orders_per_payout: usize,
    /// Seconds an order waits for its payment proposal.
    #[arg(long, default_value_t = 600)]
    order_ttl_secs: u64,
    /// Seconds between passes of the claim and refund worker.
    #[arg(long, default_value_t = 30)]
    sweep_secs: u64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let relay = cli
        .relay
        .clone()
        .unwrap_or_else(|| default_relay(cli.network).to_string());
    let items: Vec<Item> = serde_json::from_slice(
        &std::fs::read(&cli.catalog)
            .with_context(|| format!("reading {}", cli.catalog.display()))?,
    )
    .context("the catalog is not a JSON array of items")?;
    for item in &items {
        parse_address(&item.mint).with_context(|| format!("item {}: mint", item.id))?;
    }
    let zcash_address = std::fs::read_to_string(&cli.address_file)
        .with_context(|| format!("reading {}", cli.address_file.display()))?
        .trim()
        .to_string();
    let merchant = Merchant::new(Config {
        name: cli.name,
        network: cli.network,
        rpc_url: cli.rpc_url,
        relay: parse_address(&relay)?,
        keypair: cli.keypair,
        ufvk_file: cli.ufvk_file,
        zcash_address,
        items,
        data_dir: cli.data_dir,
        allowed_origins: cli.allowed_origins,
        max_open_orders: cli.max_open_orders,
        max_orders_per_payout: cli.max_orders_per_payout,
        order_ttl: Duration::from_secs(cli.order_ttl_secs),
    })?;
    merchant::serve(merchant, &cli.bind, 4, Duration::from_secs(cli.sweep_secs))
}
