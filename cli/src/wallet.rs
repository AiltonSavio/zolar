//! `zolar-wallet`: the customer's local helper for a Zolar checkout, standing in for a Zcash wallet
//! that supports Zolar.
//!
//! It listens on 127.0.0.1 only. A shop page on an allowed origin may ask it to pay an order
//! (`POST /checkout`); nothing is paid until the customer approves the order's terms on the wallet's
//! own page (`/approve/<secret>`), which only this service can submit. After the approval the wallet
//! works with the merchant directly: it builds the payment, has the merchant lock the item, checks the
//! Escrow against the approved terms, pays, waits for finality, proves on this machine and claims.
//! The page only reads the stage (`GET /jobs/<id>`), never the transaction ID or the salt.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use solana_address::Address;
use tiny_http::{Method, Request, Server};
use zolar_relayer::rpc::HttpRpc;

use crate::{
    Network,
    checkout::{
        self, CheckoutStarted, EscrowCreated, OrderTerms, ProposePayment, Reply, Stage,
        StartCheckout,
    },
    devtool::Devtool,
    payer::{self, Chains, Check, Claim, PrivateClaimArgs},
    prover::Prover,
    solana::{parse_address, read_relay},
};

pub struct Config {
    pub port: u16,
    /// Shop pages that may ask this wallet to pay (exact origins).
    pub allowed_origins: Vec<String>,
    pub network: Network,
    pub rpc_url: String,
    /// The Header Relay this wallet trusts; an order naming another is refused.
    pub relay: Address,
    /// Pays the Solana fees of the claim. The item goes to the payout wallet of the order.
    pub fee_payer: PathBuf,
    pub devtool: Devtool,
    pub prover: Prover,
    pub data_dir: PathBuf,
    /// The most an order may cost, in zatoshis.
    pub max_price_zats: u64,
    /// `scripts/check-txid-absent.sh`, run after the claim; none skips the check.
    pub txid_check: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Job {
    id: String,
    token: String,
    approve_secret: String,
    origin: String,
    merchant_url: String,
    terms: OrderTerms,
    digest: String,
    stage: Stage,
    created_at: u64,
    txid: Option<String>,
    salt: Option<String>,
    escrow: Option<String>,
    expiry: Option<u32>,
    /// Set right before the payment is broadcast: from then on the job must end with a claim.
    #[serde(default)]
    broadcasting: bool,
}

/// What the page may read about a job.
#[derive(Serialize)]
struct JobView<'a> {
    job: &'a str,
    item: &'a str,
    price_zats: u64,
    payout: &'a str,
    #[serde(flatten)]
    stage: &'a Stage,
}

pub struct Wallet {
    config: Config,
    jobs: Mutex<HashMap<String, Job>>,
    queue: Mutex<mpsc::Sender<String>>,
    relay_cache: Mutex<Option<(Instant, u32, u32)>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn network_name(network: Network) -> &'static str {
    match network {
        Network::Testnet => "testnet",
        Network::Mainnet => "mainnet",
    }
}

/// An approved order must name this wallet's network and relay, a sane price and real addresses.
fn check_order(terms: &OrderTerms, config: &Config) -> Result<()> {
    ensure!(
        terms.network == network_name(config.network),
        "the order is for Zcash {}, this wallet pays on {}",
        terms.network,
        network_name(config.network)
    );
    ensure!(
        terms.relay == config.relay.to_string(),
        "the order trusts relay {}, this wallet trusts {}",
        terms.relay,
        config.relay
    );
    ensure!(
        terms.item.price_zats > 0 && terms.item.price_zats <= config.max_price_zats,
        "the price {} zatoshis is outside this wallet's limit of {}",
        terms.item.price_zats,
        config.max_price_zats
    );
    ensure!(terms.item.amount > 0, "the order delivers nothing");
    parse_address(&terms.item.mint).context("the item's mint is not a Solana address")?;
    parse_address(&terms.payout).context("the payout is not a Solana address")?;
    ensure!(
        !terms.memo.is_empty() && terms.memo.len() <= 200,
        "the memo challenge is missing or too long"
    );
    Ok(())
}

fn merchant_base(url: &str) -> Result<String> {
    let url = url.trim_end_matches('/');
    ensure!(
        url.starts_with("http://") || url.starts_with("https://"),
        "the merchant URL must be http(s)"
    );
    Ok(url.to_string())
}

fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get_terms(merchant_url: &str, order_id: &str) -> Result<OrderTerms> {
    ensure!(
        order_id.chars().all(|c| c.is_ascii_alphanumeric()) && order_id.len() <= 64,
        "bad order id"
    );
    let url = format!("{merchant_url}/orders/{order_id}/terms");
    let mut response = http()
        .get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    ensure!(
        response.status() == 200,
        "the merchant answered {} for the order",
        response.status()
    );
    let terms: OrderTerms = response.body_mut().read_json()?;
    ensure!(
        terms.order_id == order_id,
        "the merchant sent another order"
    );
    Ok(terms)
}

fn propose(merchant_url: &str, order_id: &str, pczt: &[u8]) -> Result<EscrowCreated> {
    let url = format!("{merchant_url}/orders/{order_id}/escrow");
    let body = ProposePayment {
        pczt: base64::engine::general_purpose::STANDARD.encode(pczt),
    };
    let mut response = http()
        .post(&url)
        .send_json(&body)
        .with_context(|| format!("POST {url}"))?;
    let status = response.status().as_u16();
    let value: serde_json::Value = response.body_mut().read_json()?;
    if status != 201 {
        bail!(
            "the merchant refused the payment: {}",
            value["error"].as_str().unwrap_or("no reason given")
        );
    }
    Ok(serde_json::from_value(value)?)
}

impl Wallet {
    /// Loads saved jobs, and returns the wallet and the receiver of the job queue for [`serve`].
    pub fn new(config: Config) -> Result<(Arc<Self>, mpsc::Receiver<String>)> {
        std::fs::create_dir_all(config.data_dir.join("jobs"))?;
        let (tx, rx) = mpsc::channel();
        let mut jobs = HashMap::new();
        for entry in std::fs::read_dir(config.data_dir.join("jobs"))?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let job: Job = serde_json::from_slice(&std::fs::read(&path)?)
                    .with_context(|| format!("reading {}", path.display()))?;
                jobs.insert(job.id.clone(), job);
            }
        }
        let wallet = Arc::new(Self {
            config,
            jobs: Mutex::new(jobs),
            queue: Mutex::new(tx),
            relay_cache: Mutex::new(None),
        });
        wallet.resume();
        Ok((wallet, rx))
    }

    /// A job interrupted after its payment went out must still claim, or the merchant takes the item
    /// back while the customer's ZEC is gone. Earlier interruptions sent nothing.
    fn resume(&self) {
        let ids: Vec<(String, bool, bool)> = self
            .jobs
            .lock()
            .expect("jobs lock")
            .values()
            .map(|j| (j.id.clone(), j.stage.is_terminal(), j.broadcasting))
            .collect();
        for (id, terminal, broadcasting) in ids {
            if terminal {
                continue;
            }
            if broadcasting {
                println!("job {id}: resuming the claim of a payment already sent");
                let _ = self.queue.lock().expect("queue lock").send(id);
            } else if self
                .get(&id)
                .is_some_and(|j| j.stage != Stage::AwaitingApproval)
            {
                let _ = self.set_stage(
                    &id,
                    Stage::Failed {
                        reason: "the wallet stopped before paying; nothing was sent".into(),
                    },
                );
            }
        }
    }

    fn job_path(&self, id: &str) -> PathBuf {
        self.config.data_dir.join("jobs").join(format!("{id}.json"))
    }

    fn work_dir(&self, id: &str) -> PathBuf {
        self.config.data_dir.join("jobs").join(id)
    }

    fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().expect("jobs lock").get(id).cloned()
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) -> Result<Job> {
        let mut jobs = self.jobs.lock().expect("jobs lock");
        let job = jobs.get_mut(id).context("no such job")?;
        f(job);
        let copy = job.clone();
        drop(jobs);
        let path = self.job_path(id);
        let tmp = path.with_extension("json.tmp");
        // The job file holds the salt and the transaction ID: private to this user.
        std::fs::write(&tmp, serde_json::to_vec_pretty(&copy)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(tmp, path)?;
        Ok(copy)
    }

    fn set_stage(&self, id: &str, stage: Stage) -> Result<Job> {
        self.update(id, |j| j.stage = stage)
    }

    fn approve_url(&self, secret: &str) -> String {
        format!("http://127.0.0.1:{}/approve/{secret}", self.config.port)
    }

    fn own_origins(&self) -> [String; 2] {
        [
            format!("http://127.0.0.1:{}", self.config.port),
            format!("http://localhost:{}", self.config.port),
        ]
    }

    fn start(&self, origin: &str, request: StartCheckout) -> Result<CheckoutStarted> {
        let merchant_url = merchant_base(&request.merchant_url)?;
        let terms = get_terms(&merchant_url, &request.order_id)?;
        check_order(&terms, &self.config)?;
        let id = checkout::random_id(8);
        let job = Job {
            id: id.clone(),
            token: checkout::random_id(24),
            approve_secret: checkout::random_id(24),
            origin: origin.to_string(),
            merchant_url,
            digest: terms.digest(),
            terms,
            stage: Stage::AwaitingApproval,
            created_at: now(),
            txid: None,
            salt: None,
            escrow: None,
            expiry: None,
            broadcasting: false,
        };
        self.jobs
            .lock()
            .expect("jobs lock")
            .insert(id.clone(), job.clone());
        self.update(&id, |_| {})?;
        Ok(CheckoutStarted {
            job: id,
            token: job.token,
            approve_url: self.approve_url(&job.approve_secret),
        })
    }

    fn job_by_secret(&self, secret: &str) -> Option<Job> {
        self.jobs
            .lock()
            .expect("jobs lock")
            .values()
            .find(|j| j.approve_secret == secret)
            .cloned()
    }

    fn decide(&self, secret: &str, approve: bool, digest: &str) -> Result<()> {
        let job = self.job_by_secret(secret).context("no such approval")?;
        ensure!(
            job.stage == Stage::AwaitingApproval,
            "this checkout was already decided"
        );
        ensure!(
            now() < job.created_at + 600,
            "this approval expired; start the checkout again"
        );
        ensure!(
            digest == job.digest,
            "the terms changed since this page was shown"
        );
        if approve {
            self.set_stage(&job.id, Stage::BuildingPayment)?;
            self.queue
                .lock()
                .expect("queue lock")
                .send(job.id)
                .context("the wallet's worker stopped")?;
        } else {
            self.set_stage(&job.id, Stage::Rejected)?;
        }
        Ok(())
    }

    fn chains(&self) -> Chains {
        Chains {
            rpc_url: self.config.rpc_url.clone(),
            relay: self.config.relay,
            peers: crate::seeders(self.config.network),
            network: self.config.network,
        }
    }

    /// Runs one approved job to its end. Called from the single worker thread, so one checkout at a
    /// time uses the Zcash wallet and the prover.
    pub fn run(&self, id: &str) {
        if let Err(e) = self.run_steps(id) {
            println!("job {id}: failed: {e:#}");
            let _ = self.set_stage(
                id,
                Stage::Failed {
                    reason: format!("{e:#}"),
                },
            );
        }
    }

    fn run_steps(&self, id: &str) -> Result<()> {
        let job = self.get(id).context("no such job")?;
        let chains = self.chains();
        let work = self.work_dir(id);
        std::fs::create_dir_all(&work)?;
        let pczt = work.join("created.pczt");
        if !job.broadcasting {
            self.pay(&job, &chains, &pczt)?;
        }
        let job = self.get(id).context("no such job")?;
        let escrow = job.escrow.clone().context("no Escrow recorded")?;
        let txid = job.txid.clone().context("no transaction recorded")?;
        let salt = crate::parse_salt(job.salt.as_deref().context("no salt recorded")?)?;
        self.set_stage(
            id,
            Stage::WaitingForFinality {
                escrow: escrow.clone(),
                tip: 0,
                final_height: 0,
                expiry: job.expiry.unwrap_or(0),
            },
        )?;
        let on_proving = || {
            let _ = self.set_stage(
                id,
                Stage::Proving {
                    escrow: escrow.clone(),
                },
            );
        };
        let claim = payer::private_claim(&PrivateClaimArgs {
            claim: Claim {
                chains: &chains,
                keypair: self.config.fee_payer.clone(),
                payout: parse_address(&job.terms.payout)?,
                escrow_address: parse_address(&escrow)?,
                txid: &txid,
                salt,
            },
            prover: &self.config.prover,
            work_dir: crate::prover::default_work_dir(),
            max_attempts: 3,
            on_proving: Some(&on_proving),
        })?;
        self.set_stage(
            id,
            Stage::Claimed {
                escrow: escrow.clone(),
                claim: claim.clone(),
            },
        )?;
        let found = self.txid_found(&txid, &escrow);
        self.set_stage(
            id,
            Stage::Done {
                escrow,
                claim,
                txid_found_on_solana: found,
            },
        )?;
        Ok(())
    }

    /// Builds the payment for the approved terms, has the merchant lock the item, checks the Escrow
    /// against the approved terms, signs and broadcasts.
    fn pay(&self, job: &Job, chains: &Chains, pczt: &std::path::Path) -> Result<()> {
        let id = &job.id;
        let terms = &job.terms;
        payer::create_pczt(
            chains,
            &self.config.devtool,
            &terms.zcash_address,
            terms.item.price_zats,
            &terms.memo,
            18,
            pczt,
        )?;
        let txid = zolar_pczt::summary(&std::fs::read(pczt)?)?.txid;
        self.update(id, |j| {
            j.stage = Stage::LockingItem;
            j.txid = Some(txid);
        })?;
        let created = propose(&job.merchant_url, &terms.order_id, &std::fs::read(pczt)?)?;
        let job = self.update(id, |j| {
            j.stage = Stage::CheckingEscrow {
                escrow: created.escrow.clone(),
            };
            j.escrow = Some(created.escrow.clone());
            j.salt = Some(created.salt.clone());
            j.expiry = Some(created.expiry_height);
        })?;
        let checked = payer::check_escrow(&Check {
            chains,
            escrow_address: parse_address(&created.escrow)?,
            payer_wallet: parse_address(&terms.payout)?,
            pczt,
            salt: crate::parse_salt(&created.salt)?,
            accept_freeze_authority: false,
            terms: payer::Terms {
                mint: parse_address(&terms.item.mint)?,
                min_amount: terms.item.amount,
                max_bond: 0,
                accept_private_only: true,
            },
            finalized_wait: Duration::from_secs(120),
        })?;
        ensure!(
            job.digest == job.terms.digest(),
            "the approved terms were altered"
        );
        let signed = payer::sign(&self.config.devtool, &checked, pczt, &self.work_dir(id))?;
        self.update(id, |j| {
            j.stage = Stage::Paying {
                escrow: created.escrow.clone(),
            };
            j.broadcasting = true;
        })?;
        payer::broadcast(chains, &self.config.devtool, &checked, &signed)
    }

    fn txid_found(&self, txid: &str, escrow: &str) -> Option<bool> {
        let script = self.config.txid_check.as_ref()?;
        // Fresh claims need a moment before every RPC node serves them.
        std::thread::sleep(Duration::from_secs(5));
        let status = std::process::Command::new(script)
            .arg(txid)
            .arg("--address")
            .arg(escrow)
            .env("ZOLAR_RPC_URL", &self.config.rpc_url)
            .stdout(std::process::Stdio::null())
            .status()
            .ok()?;
        match status.code() {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        }
    }

    /// The live tip and final height while a job waits for finality, read at most every 5 seconds.
    fn relay_heights(&self) -> Option<(u32, u32)> {
        let mut cache = self.relay_cache.lock().expect("relay cache lock");
        if let Some((at, tip, fin)) = *cache
            && at.elapsed() < Duration::from_secs(5)
        {
            return Some((tip, fin));
        }
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        let relay = read_relay(&mut rpc, &self.config.relay).ok()?;
        let view = relay.view().ok()?;
        let heights = (view.tip_height(), view.finalized_height());
        *cache = Some((Instant::now(), heights.0, heights.1));
        Some(heights)
    }

    fn view(&self, id: &str, token: Option<&str>) -> Result<serde_json::Value> {
        let job = self.get(id).context("no such job")?;
        ensure!(
            token == Some(job.token.as_str()),
            "this job belongs to another page"
        );
        let mut stage = job.stage.clone();
        if let Stage::WaitingForFinality {
            tip, final_height, ..
        } = &mut stage
            && let Some((t, f)) = self.relay_heights()
        {
            *tip = t;
            *final_height = f;
        }
        Ok(serde_json::to_value(JobView {
            job: &job.id,
            item: &job.terms.item.name,
            price_zats: job.terms.item.price_zats,
            payout: &job.terms.payout,
            stage: &stage,
        })?)
    }

    fn approval_page(&self, secret: &str) -> Result<String> {
        let job = self.job_by_secret(secret).context("no such approval")?;
        let t = &job.terms;
        let e = checkout::escape;
        let zec = t.item.price_zats as f64 / 1e8;
        let decided = job.stage != Stage::AwaitingApproval;
        let fields = [
            ("origin", e(&job.origin)),
            ("merchant_url", e(&job.merchant_url)),
            ("merchant", e(&t.merchant)),
            ("item", e(&t.item.name)),
            ("amount", t.item.amount.to_string()),
            ("mint", e(&t.item.mint)),
            ("zec", format!("{zec}")),
            ("network", e(&t.network)),
            ("address", e(&t.zcash_address)),
            ("payout", e(&t.payout)),
            ("digest", e(&job.digest)),
            ("decided", decided.to_string()),
        ];
        let mut page = include_str!("wallet_approve.html").to_string();
        for (name, value) in fields {
            page = page.replace(&format!("{{{{{name}}}}}"), &value);
        }
        Ok(page)
    }

    pub fn handle(&self, mut request: Request) {
        if let Err(e) =
            checkout::check_loopback_host(checkout::header(&request, "Host"), self.config.port)
        {
            let _ = request.respond(checkout::error(421, e));
            return;
        }
        let origin = checkout::header(&request, "Origin").map(str::to_string);
        let reply = self.route(&mut request, origin.as_deref());
        let _ = request.respond(reply);
    }

    fn route(&self, request: &mut Request, origin: Option<&str>) -> Reply {
        let method = request.method().clone();
        let url = request.url().split('?').next().unwrap_or("").to_string();
        let parts: Vec<&str> = url.trim_matches('/').split('/').collect();
        let shop =
            checkout::allowed_origin(origin, &self.config.allowed_origins).map(str::to_string);
        let own = origin.is_some_and(|o| self.own_origins().iter().any(|s| s == o));
        let token = checkout::header(request, "Authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_string);

        let result: Result<Reply> = match (&method, parts.as_slice()) {
            // Shop pages: start a checkout, read its stage.
            (Method::Options, ["checkout"] | ["jobs", _]) if shop.is_some() => {
                Ok(checkout::json(204, &serde_json::json!({})))
            }
            (Method::Post, ["checkout"]) => match shop.as_deref() {
                Some(o) => checkout::read_json(request, 4096)
                    .and_then(|r| self.start(o, r))
                    .map(|s| checkout::json(201, &s)),
                None => Err(anyhow::anyhow!("this site may not use the wallet")),
            },
            (Method::Get, ["jobs", id]) if shop.is_some() => self
                .view(id, token.as_deref())
                .map(|v| checkout::json(200, &v)),
            // The wallet's own approval page; only that page (this origin) may submit the decision.
            (Method::Get, ["approve", secret]) if origin.is_none() || own => {
                self.approval_page(secret).map(checkout::html)
            }
            (Method::Post, ["approve", secret]) if own => {
                #[derive(Deserialize)]
                struct Decision {
                    approve: bool,
                    digest: String,
                }
                checkout::read_json::<Decision>(request, 1024)
                    .and_then(|d| self.decide(secret, d.approve, &d.digest))
                    .map(|_| checkout::json(200, &serde_json::json!({ "ok": true })))
            }
            _ => Err(anyhow::anyhow!("not found")),
        };
        let reply = result.unwrap_or_else(|e| {
            let status = if e.to_string() == "not found" {
                404
            } else {
                400
            };
            checkout::error(status, format!("{e:#}"))
        });
        if shop.is_some() {
            checkout::with_cors(reply, shop.as_deref())
        } else {
            reply
        }
    }
}

/// Serves until the process ends, with one worker running approved checkouts in order.
pub fn serve(wallet: Arc<Wallet>, queue: mpsc::Receiver<String>) -> Result<()> {
    let bind = format!("127.0.0.1:{}", wallet.config.port);
    let server = Arc::new(Server::http(&bind).map_err(|e| anyhow::anyhow!("binding {bind}: {e}"))?);
    println!("zolar-wallet listening on http://{bind}");
    let worker = wallet.clone();
    std::thread::spawn(move || {
        while let Ok(id) = queue.recv() {
            worker.run(&id);
        }
    });
    let handlers: Vec<_> = (0..4)
        .map(|_| {
            let server = server.clone();
            let wallet = wallet.clone();
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    wallet.handle(request);
                }
            })
        })
        .collect();
    for h in handlers {
        if h.join().is_err() {
            bail!("a request handler panicked");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkout::Item;

    fn config(dir: &std::path::Path) -> Config {
        Config {
            port: 7878,
            allowed_origins: vec!["http://localhost:5175".into()],
            network: Network::Testnet,
            rpc_url: "http://127.0.0.1:1".into(),
            relay: Address::new_from_array([4; 32]),
            fee_payer: dir.join("fee.json"),
            devtool: Devtool {
                binary: dir.join("devtool"),
                wallet_dir: dir.join("w"),
                identity: dir.join("id"),
            },
            prover: Prover {
                script: dir.join("prove.sh"),
                data_dir: dir.join("data"),
                depth: 8,
                mem_max: "2500M".into(),
            },
            data_dir: dir.to_path_buf(),
            max_price_zats: 1_000_000,
            txid_check: None,
        }
    }

    fn terms() -> OrderTerms {
        OrderTerms {
            order_id: "abc123".into(),
            merchant: "Proof of Party".into(),
            item: Item {
                id: "ga".into(),
                name: "General admission".into(),
                mint: Address::new_from_array([3; 32]).to_string(),
                amount: 1,
                price_zats: 100_000,
            },
            zcash_address: "utest1merchant".into(),
            memo: "zolar order abc123 ff".into(),
            payout: Address::new_from_array([2; 32]).to_string(),
            relay: Address::new_from_array([4; 32]).to_string(),
            network: "testnet".into(),
        }
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("zolar-wallet-test-{}", checkout::random_id(8)));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn wallet_with_job(
        dir: &std::path::Path,
        stage: Stage,
        broadcasting: bool,
    ) -> (Arc<Wallet>, mpsc::Receiver<String>, Job) {
        let t = terms();
        let job = Job {
            id: "job1".into(),
            token: "tok".into(),
            approve_secret: "secret".into(),
            origin: "http://localhost:5175".into(),
            merchant_url: "http://127.0.0.1:1".into(),
            digest: t.digest(),
            terms: t,
            stage,
            created_at: now(),
            txid: None,
            salt: None,
            escrow: None,
            expiry: None,
            broadcasting,
        };
        std::fs::create_dir_all(dir.join("jobs")).unwrap();
        std::fs::write(
            dir.join("jobs/job1.json"),
            serde_json::to_vec(&job).unwrap(),
        )
        .unwrap();
        let (w, rx) = Wallet::new(config(dir)).unwrap();
        (w, rx, job)
    }

    #[test]
    fn an_order_for_another_relay_network_or_price_is_refused() {
        let dir = tempdir();
        let c = config(&dir);
        check_order(&terms(), &c).unwrap();
        let mut t = terms();
        t.relay = Address::new_from_array([9; 32]).to_string();
        assert!(
            check_order(&t, &c)
                .unwrap_err()
                .to_string()
                .contains("relay")
        );
        let mut t = terms();
        t.network = "mainnet".into();
        assert!(check_order(&t, &c).is_err());
        let mut t = terms();
        t.item.price_zats = 2_000_000;
        assert!(
            check_order(&t, &c)
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
        let mut t = terms();
        t.payout = "not an address".into();
        assert!(check_order(&t, &c).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_is_paid_before_approval_and_altered_terms_are_refused() {
        let dir = tempdir();
        let (w, rx, job) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        let err = w.decide("secret", true, "another digest").unwrap_err();
        assert!(err.to_string().contains("terms changed"));
        assert!(rx.try_recv().is_err(), "nothing queued");
        assert!(w.decide("wrong secret", true, &job.digest).is_err());
        w.decide("secret", true, &job.digest).unwrap();
        assert_eq!(rx.try_recv().unwrap(), "job1");
        assert!(
            w.decide("secret", true, &job.digest)
                .unwrap_err()
                .to_string()
                .contains("already")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rejected_checkout_is_never_queued() {
        let dir = tempdir();
        let (w, rx, job) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        w.decide("secret", false, &job.digest).unwrap();
        assert!(rx.try_recv().is_err());
        assert_eq!(w.get("job1").unwrap().stage, Stage::Rejected);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_payment_already_sent_is_claimed_after_a_restart() {
        let dir = tempdir();
        let (_w, rx, _) = wallet_with_job(&dir, Stage::Paying { escrow: "E".into() }, true);
        assert_eq!(rx.try_recv().unwrap(), "job1", "resumed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_job_stopped_before_paying_ends_as_failed_with_nothing_sent() {
        let dir = tempdir();
        let (w, rx, _) = wallet_with_job(&dir, Stage::LockingItem, false);
        assert!(rx.try_recv().is_err());
        match w.get("job1").unwrap().stage {
            Stage::Failed { reason } => assert!(reason.contains("nothing was sent")),
            s => panic!("{s:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_page_reads_only_its_own_job_and_never_the_txid_or_salt() {
        let dir = tempdir();
        let (w, _rx, _) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        w.update("job1", |j| {
            j.txid = Some("deadbeef".into());
            j.salt = Some("c0ffee".into());
        })
        .unwrap();
        assert!(w.view("job1", Some("other")).is_err());
        assert!(w.view("job1", None).is_err());
        let v = w.view("job1", Some("tok")).unwrap().to_string();
        assert!(!v.contains("deadbeef") && !v.contains("c0ffee"), "{v}");
        assert!(v.contains("awaiting_approval"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_approval_page_shows_the_terms_escaped() {
        let dir = tempdir();
        let (w, _rx, _) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        w.update("job1", |j| j.terms.item.name = "<script>x</script>".into())
            .unwrap();
        let page = w.approval_page("secret").unwrap();
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("<script>x"));
        assert!(page.contains("0.001"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
