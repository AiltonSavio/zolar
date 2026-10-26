use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, sync::Arc};
use vending_prover::{
    backend::{ChainBackend, ShopConfig, StaticShops},
    engine::Sp1Engine,
    PaymentBackend, ProofEngine, ProveRequest,
};

#[derive(Parser)]
#[command(
    name = "zolar-vend-prove",
    about = "Keyless Zcash-testnet → Solana-devnet vending prover"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, global = true)]
    txid: Option<String>,
    #[arg(long, global = true)]
    machine: Option<String>,
    #[arg(long, global = true)]
    nonce: Option<String>,
    #[arg(long, global = true, default_value = "https://api.devnet.solana.com")]
    rpc: String,
    #[arg(long, global = true)]
    shops: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        default_value = "https://zcash-testnet.chainsafe.dev"
    )]
    lightwallet: String,
    #[arg(long, global = true, value_delimiter = ',')]
    peers: Vec<String>,
    #[arg(long, global = true)]
    witness: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        default_value = "crates/vending-guest/elf/vending-guest"
    )]
    elf: PathBuf,
    #[arg(long, global = true, conflicts_with = "prove")]
    execute: bool,
    #[arg(long, global = true, conflicts_with = "execute")]
    prove: bool,
    #[arg(long, global = true, default_value = ".")]
    out: PathBuf,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "127.0.0.1:8088")]
        bind: std::net::SocketAddr,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // Refuse before network setup or any large SP1 allocation.
    if cli.prove {
        vending_prover::engine::check_proving_memory(&std::fs::read_to_string("/proc/meminfo")?)?;
    }
    let engine = Arc::new(Sp1Engine {
        elf: cli.elf,
        prove: cli.prove,
    });
    if let Some(path) = cli.witness {
        ensure!(
            cli.command.is_none(),
            "--witness is only available for one-shot checks"
        );
        let witness = serde_json::from_slice(&std::fs::read(path)?)?;
        write_result(engine.run(witness).await?, cli.out)?;
        return Ok(());
    }
    let shops: Vec<ShopConfig> = serde_json::from_slice(&std::fs::read(cli.shops.context(
        "--shops public metadata file is required until the machine account decoder is integrated",
    )?)?)?;
    let peers = if cli.peers.is_empty() {
        vending_prover::chain::p2p::Network::Testnet
            .seeders()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        cli.peers
    };
    let backend = Arc::new(
        ChainBackend::new(
            Arc::new(StaticShops::new(shops)?),
            cli.rpc,
            cli.lightwallet,
            peers,
        )
        .await?,
    );
    if let Some(Command::Serve { bind }) = cli.command {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        eprintln!(
            "listening on http://{bind} ({})",
            if cli.prove {
                "Groth16"
            } else {
                "execute only; no claimable proof"
            }
        );
        axum::serve(listener, vending_prover::router(backend, engine)).await?;
        return Ok(());
    }
    let txid = cli.txid.context("--txid required")?;
    let machine = cli.machine.context("--machine required")?;
    let nonce = match cli.nonce {
        Some(n) => n,
        None => backend.discover_nonce(&txid, &machine).await?,
    };
    let w = backend
        .prepare(&ProveRequest {
            txid,
            machine,
            reservation_nonce: nonce,
        })
        .await?;
    write_result(engine.run(w).await?, cli.out)
}
fn write_result(result: vending_prover::ProofOutput, out: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&out)?;
    std::fs::write(
        out.join("public_values.bin"),
        hex::decode(&result.public_values)?,
    )?;
    if let Some(ref proof) = result.proof {
        std::fs::write(out.join("proof.bin"), hex::decode(proof)?)?;
    }
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
    println!(
        "mode={} cycles={} public_values=249 proof={}",
        result.mode,
        result.cycles,
        result.proof.is_some()
    );
    if let Some(vkey) = result.vkey_hash {
        println!("vkey_hash={vkey}");
    }
    Ok(())
}
