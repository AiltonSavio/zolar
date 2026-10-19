//! `zolar-relayer init`: create and seal a Header Relay from a Checkpoint near the live testnet tip.
//! `zolar-relayer run`: keep that relay at the tip.
//!
//! Configuration comes from environment variables; key files are only read by this process.
use std::{
    env, process,
    str::FromStr,
    thread::sleep,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use solana_address::Address;
use solana_keypair::{read_keypair_file, Keypair};
use solana_signer::Signer;
use zolar_relayer::{
    chain::RelayClient,
    follower::{ChainError, Event, Follower, HeaderSource, Step},
    p2p::{sha256d, Network, RawHeader},
    peers::PeerPool,
    rpc::HttpRpc,
    setup::{checkpoint_from, create_relay, CONTEXT},
};

/// Where `init` starts scanning when no start is given (hash in explorer order): testnet block
/// 4,495,260 (NU7) and mainnet block 3,522,600 (last of the ticket 05 fixture).
fn default_start(network: Network) -> (u32, &'static str) {
    match network {
        Network::Testnet => (
            4_495_260,
            "00000005c6568f0bc6f3ce796a6daf312edaeb01a846a387680ba28703f35ac5",
        ),
        Network::Mainnet => (
            3_522_600,
            "00000000002e353a4e70cc01ff11d38f9a2c870e34fea4350e14750bab4b9b68",
        ),
    }
}
const STATUS_EVERY: Duration = Duration::from_secs(600);

fn var(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("{name} is not set"))
}

fn var_or<T: FromStr>(name: &str, default: T) -> Result<T, String> {
    match env::var(name) {
        Ok(v) => v.parse().map_err(|_| format!("{name} is not valid: {v}")),
        Err(_) => Ok(default),
    }
}

fn payer() -> Result<Keypair, String> {
    let path = var("ZOLAR_PAYER_KEYPAIR")?;
    read_keypair_file(&path).map_err(|e| format!("cannot read the key file at {path}: {e}"))
}

fn network() -> Result<Network, String> {
    var_or("ZOLAR_NETWORK", Network::Testnet)
}

fn peers(network: Network) -> PeerPool {
    let hosts = match env::var("ZOLAR_PEERS") {
        Ok(list) => list.split(',').map(|h| h.trim().to_string()).collect(),
        Err(_) => network.seeders().iter().map(|s| s.to_string()).collect(),
    };
    PeerPool::new(hosts, network)
}

fn display(hash: &[u8; 32]) -> String {
    let mut reversed = *hash;
    reversed.reverse();
    hex::encode(reversed)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn log(line: impl AsRef<str>) {
    println!("t={} {}", now(), line.as_ref());
}

fn print_event(event: Event) {
    log(match event {
        Event::Tip { height, hash } => format!("tip height={height} hash={}", display(&hash)),
        Event::Uploaded { height, hash } => {
            format!("uploaded height={height} hash={}", display(&hash))
        }
        Event::Verified { height, hash } => {
            format!("verified height={height} hash={}", display(&hash))
        }
        Event::Accepted {
            height,
            hash,
            tip_height,
            signatures,
        } => format!(
            "accepted height={height} hash={} relay_tip={tip_height} txs={}",
            display(&hash),
            signatures.join(",")
        ),
        Event::Known { height, hash } => {
            format!("already-known height={height} hash={}", display(&hash))
        }
        Event::Rejected {
            height,
            hash,
            code,
            reason,
            peer,
        } => format!(
            "REJECTED height={height} hash={} reason=\"{reason}\" code={code} peer={peer}",
            display(&hash)
        ),
        Event::Unlinked { peer } => {
            format!("peer {peer} sent headers that do not link to the relay")
        }
        Event::Halted { reason } => {
            format!("HALTED reason={reason}: the relay accepts no more headers")
        }
        Event::Warn(m) => format!("warn {m}"),
    });
}

fn run() -> Result<(), String> {
    let rpc_url = var("ZOLAR_RPC_URL")?;
    let payer = payer()?;
    let relay = Address::from_str(&var("ZOLAR_RELAY")?).map_err(|e| format!("ZOLAR_RELAY: {e}"))?;
    let poll = Duration::from_secs(var_or("ZOLAR_POLL_SECS", 10u64)?);
    let payer_address = payer.pubkey();
    let network = network()?;
    let status_rpc = HttpRpc::new(&rpc_url);

    log(format!(
        "relayer start network={network:?} relay={relay} payer={payer_address} rpc={rpc_url}"
    ));
    let chain = RelayClient::new(HttpRpc::new(&rpc_url), payer, relay);
    let mut follower = Follower::new(chain, peers(network), print_event);
    let mut failures = 0u32;
    let mut last_status = Instant::now() - STATUS_EVERY;
    loop {
        if last_status.elapsed() >= STATUS_EVERY {
            last_status = Instant::now();
            match status_rpc.balance(&payer_address) {
                Ok(lamports) => log(format!("payer balance_lamports={lamports}")),
                Err(e) => log(format!("warn balance: {e}")),
            }
        }
        match follower.step() {
            Ok(Step::Progress { .. }) => failures = 0,
            Ok(Step::Idle | Step::Rejected) => {
                failures = 0;
                sleep(poll);
            }
            Ok(Step::Halted) => sleep(Duration::from_secs(60)),
            Err(ChainError::Transient(e)) => {
                failures += 1;
                let wait = Duration::from_secs((1u64 << failures.min(6)).min(60));
                log(format!("retry in {}s after: {e}", wait.as_secs()));
                sleep(wait);
            }
        }
    }
}

/// Walks the live chain from a known block to its tip and returns the newest headers with heights.
fn newest_headers(
    pool: &mut PeerPool,
    mut height: u32,
    mut hash: [u8; 32],
    keep: usize,
) -> Result<Vec<(u32, RawHeader)>, String> {
    let mut recent: Vec<(u32, RawHeader)> = Vec::new();
    let mut unlinked = 0;
    let mut failures = 0;
    loop {
        let batch = match pool.headers_after(&[hash]) {
            Ok(batch) => batch,
            Err(e) => {
                failures += 1;
                if failures > 400 {
                    return Err(format!("no peer answered: {e}"));
                }
                log(format!("warn no peer answered ({e}); trying again"));
                sleep(Duration::from_secs(3));
                continue;
            }
        };
        let Some(first) = batch.headers.first() else {
            if recent.is_empty() {
                return Err("the start block has no successors on this peer".into());
            }
            return Ok(recent);
        };
        if first[4..36] != hash {
            pool.penalize(&batch.peer);
            unlinked += 1;
            if unlinked > 30 {
                return Err("peers keep answering from another branch".into());
            }
            continue;
        }
        for header in batch.headers {
            if header[4..36] != hash {
                return Err("headers do not link".into());
            }
            hash = sha256d(&header);
            height += 1;
            recent.push((height, header));
        }
        if recent.len() > keep * 2 {
            recent.drain(..recent.len() - keep);
        }
        if recent.last().is_some_and(|(h, _)| h % 2000 < 160) {
            log(format!("scanning height={height}"));
        }
    }
}

fn init() -> Result<(), String> {
    let rpc_url = var("ZOLAR_RPC_URL")?;
    let payer = payer()?;
    let finality_depth: u16 = var_or("ZOLAR_FINALITY_DEPTH", 10)?;
    let lag: usize = var_or("ZOLAR_CHECKPOINT_LAG", 20)?;
    let nonce: u64 = var_or("ZOLAR_RELAY_NONCE", now())?;
    let network = network()?;
    let (default_height, default_hash) = default_start(network);
    let start_height: u32 = var_or("ZOLAR_START_HEIGHT", default_height)?;
    let start_hash = var_or("ZOLAR_START_HASH", default_hash.to_string())?;
    let mut start: [u8; 32] = hex::decode(&start_hash)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("ZOLAR_START_HASH must be 64 hex characters")?;
    start.reverse();

    let mut pool = peers(network);
    let recent = newest_headers(&mut pool, start_height, start, CONTEXT + lag + 1)?;
    let tip = recent.last().map(|(h, _)| *h).unwrap();
    let end = recent.len() - lag;
    if end < CONTEXT {
        return Err(format!("only {} headers found", recent.len()));
    }
    let window: Vec<RawHeader> = recent[end - CONTEXT..end].iter().map(|(_, h)| *h).collect();
    let checkpoint = checkpoint_from(&window, recent[end - 1].0, finality_depth, nonce, network)?;
    log(format!(
        "{network:?} live tip height={tip}; checkpoint height={} hash={} finality_depth={finality_depth} nonce={nonce}",
        checkpoint.height,
        display(&checkpoint.hash)
    ));

    let mut rpc = HttpRpc::new(&rpc_url);
    let (relay, signatures) = create_relay(&mut rpc, &payer, &checkpoint)?;
    log(format!(
        "relay created and sealed address={relay} creator={}",
        payer.pubkey()
    ));
    for signature in signatures {
        log(format!("tx {signature}"));
    }
    println!("ZOLAR_RELAY={relay}");
    Ok(())
}

fn main() {
    let result = match env::args().nth(1).as_deref() {
        Some("run") => run(),
        Some("init") => init(),
        _ => Err(
            "usage: zolar-relayer <init|run> (settings come from ZOLAR_* environment variables)"
                .into(),
        ),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}
