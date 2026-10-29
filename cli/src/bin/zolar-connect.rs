//! Keyless checkout/proof connector for the existing Zkool testnet wallet.
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use zolar_cli::{
    DEVNET_RPC, Network, default_relay,
    prover::Prover,
    solana::parse_address,
    wallet::{self, Config, PaymentWallet, Wallet},
    zkool::Zkool,
};

#[derive(Parser)]
#[command(
    name = "zolar-connect",
    about = "Connect an existing Zkool testnet wallet to a checkout; keeps no Zcash spending keys"
)]
struct Cli {
    #[arg(long, default_value_t = 7878)]
    port: u16,
    #[arg(long = "allow-origin", required = true)]
    allowed_origins: Vec<String>,
    #[arg(
        long,
        env = "ZOLAR_ZKOOL_URL",
        default_value = "http://127.0.0.1:8000/graphql"
    )]
    zkool_url: String,
    /// An account-scoped write JWT for the customer's existing Zkool wallet API.
    #[arg(long, env = "ZOLAR_ZKOOL_TOKEN", hide_env_values = true)]
    zkool_token: String,
    #[arg(long, env = "ZOLAR_ZKOOL_ACCOUNT")]
    zkool_account: u32,
    /// Optional responsive Zcash testnet P2P host[:port]; repeat to supply a peer pool.
    #[arg(long = "zcash-peer")]
    zcash_peers: Vec<String>,
    #[arg(long, env = "ZOLAR_RPC_URL", default_value = DEVNET_RPC)]
    rpc_url: String,
    #[arg(long, env = "ZOLAR_RELAY")]
    relay: Option<String>,
    /// Solana devnet fee key only; the connected Solana address receives the ticket.
    #[arg(long, env = "ZOLAR_PAYER_KEYPAIR")]
    fee_payer: PathBuf,
    #[arg(long, env = "ZOLAR_PROVE_SCRIPT")]
    prove_script: PathBuf,
    #[arg(long, env = "ZOLAR_DATA")]
    zolar_data: PathBuf,
    #[arg(long, env = "ZOLAR_MEM_MAX", default_value = "2500M")]
    mem_max: String,
    #[arg(long, env = "ZOLAR_CONNECT_DATA", default_value = ".zolar-connect")]
    data_dir: PathBuf,
    #[arg(long, default_value_t = 1_000_000)]
    max_price_zats: u64,
    #[arg(long, env = "ZOLAR_TXID_CHECK")]
    txid_check: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let zkool = Zkool::new(&cli.zkool_url, cli.zkool_token, cli.zkool_account)?;
    let connection = zkool.connection()?;
    println!(
        "Connected to {} testnet account {}",
        connection.name, connection.account
    );
    let relay = cli
        .relay
        .unwrap_or_else(|| default_relay(Network::Testnet).to_string());
    let (connector, queue) = Wallet::new(Config {
        port: cli.port,
        allowed_origins: cli.allowed_origins,
        network: Network::Testnet,
        peers: cli.zcash_peers,
        rpc_url: cli.rpc_url,
        relay: parse_address(&relay)?,
        fee_payer: cli.fee_payer,
        payment_wallet: PaymentWallet::Zkool(zkool),
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
    wallet::serve(connector, queue)
}
