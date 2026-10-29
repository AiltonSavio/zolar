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
use escrow_client::{accounts::Escrow, programs::ESCROW_ID};
use serde::{Deserialize, Serialize};
use solana_address::Address;
use tiny_http::{Method, Request, Server};
use zolar_relayer::{chain::Cluster, rpc::HttpRpc};

use crate::{
    Network,
    checkout::{
        self, CheckoutStarted, EscrowCreated, OrderTerms, ProposePayment, Reply, Stage,
        StartCheckout,
    },
    devtool::Devtool,
    payer::{self, Chains, Check, Claim, PrivateClaimArgs},
    prover::Prover,
    settlement::{self, Event},
    solana::{parse_address, read_relay},
    zkool::Zkool,
};

/// Seconds the customer has to approve a checkout (the merchant's order lapses about then too).
const APPROVAL_SECS: u64 = 600;

pub enum PaymentWallet {
    Devtool(Devtool),
    Zkool(Zkool),
}

pub struct Config {
    pub port: u16,
    /// Shop pages that may ask this wallet to pay (exact origins).
    pub allowed_origins: Vec<String>,
    pub network: Network,
    /// Optional P2P hosts; headers and blocks remain verified against the trusted Relay.
    pub peers: Vec<String>,
    pub rpc_url: String,
    /// The Header Relay this wallet trusts; an order naming another is refused.
    pub relay: Address,
    /// Pays the Solana fees of the claim. The item goes to the payout wallet of the order.
    pub fee_payer: PathBuf,
    pub payment_wallet: PaymentWallet,
    pub prover: Prover,
    pub data_dir: PathBuf,
    /// The most an order may cost, in zatoshis.
    pub max_price_zats: u64,
    /// `scripts/check-txid-absent.sh`, run after the claim; none skips the check.
    pub txid_check: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
enum PaymentState {
    #[default]
    Unsent,
    Signing,
    BroadcastUncertain,
    BroadcastAcknowledged,
    Settled,
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
    /// Relay tip and Finality Depth when the payment went out.
    #[serde(default)]
    sent: Option<(u32, u16)>,
    #[serde(default)]
    payment: PaymentState,
    #[serde(default)]
    claim_signature: Option<String>,
    #[serde(default)]
    retry_at: u64,
    #[serde(default)]
    claim_submitted_at: u64,
    #[serde(default)]
    zcash_address: Option<String>,
}

impl Job {
    fn needs_recovery(&self) -> bool {
        (self.broadcasting || self.payment != PaymentState::Unsent)
            && self.payment != PaymentState::Settled
            && !matches!(self.stage, Stage::Done { .. } | Stage::Rejected)
    }
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
    _data_lock: checkout::DataDirLock,
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
        if matches!(config.payment_wallet, PaymentWallet::Zkool(_)) {
            ensure!(
                config.network == Network::Testnet,
                "external wallet demo requires testnet"
            );
        }
        let data_lock = checkout::lock_data_dir(&config.data_dir)?;
        checkout::private_dir(&config.data_dir.join("jobs"))?;
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
            _data_lock: data_lock,
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
        let jobs: Vec<Job> = self
            .jobs
            .lock()
            .expect("jobs lock")
            .values()
            .cloned()
            .collect();
        for job in jobs {
            if job.needs_recovery() {
                // Legacy broadcasting=true includes failed jobs and means uncertain, not delivered.
                if self
                    .update(&job.id, |j| {
                        if j.payment == PaymentState::Unsent {
                            j.payment = PaymentState::BroadcastUncertain;
                        }
                        j.stage = Stage::Recovering;
                        j.retry_at = 0;
                    })
                    .is_err()
                {
                    continue;
                }
                let _ = self.queue.lock().expect("queue lock").send(job.id);
            } else if !job.stage.is_terminal() && job.stage != Stage::AwaitingApproval {
                let _ = self.set_stage(
                    &job.id,
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
        self.try_update(id, |j| {
            f(j);
            Ok(())
        })
    }

    fn try_update(&self, id: &str, f: impl FnOnce(&mut Job) -> Result<()>) -> Result<Job> {
        let mut jobs = self.jobs.lock().expect("jobs lock");
        let mut next = jobs.get(id).context("no such job")?.clone();
        f(&mut next)?;
        checkout::save_private(&self.job_path(id), &serde_json::to_vec_pretty(&next)?)?;
        jobs.insert(id.to_string(), next.clone());
        Ok(next)
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
        let connection = match &self.config.payment_wallet {
            PaymentWallet::Zkool(wallet) => {
                let c = wallet.identity()?;
                ensure!(
                    request.zcash_address.as_deref() == Some(c.address.as_str())
                        && request.zcash_account == Some(c.account),
                    "the connected Zkool account changed; reconnect before paying"
                );
                Some(c)
            }
            PaymentWallet::Devtool(_) => None,
        };
        let merchant_url = merchant_base(&request.merchant_url)?;
        let terms = get_terms(&merchant_url, &request.order_id)?;
        check_order(&terms, &self.config)?;
        let existing: Vec<Job> = self
            .jobs
            .lock()
            .expect("jobs lock")
            .values()
            .cloned()
            .collect();
        for job in existing {
            self.expire_if_stale(&job)?;
        }
        let mut jobs = self.jobs.lock().expect("jobs lock");
        if connection.is_some() {
            ensure!(
                !jobs.values().any(|j| j.needs_recovery()
                    || !matches!(
                        j.stage,
                        Stage::Done { .. } | Stage::Rejected | Stage::Failed { .. }
                    )),
                "finish or reject the existing Zkool checkout before starting another"
            );
        }
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
            sent: None,
            payment: PaymentState::Unsent,
            claim_signature: None,
            retry_at: 0,
            claim_submitted_at: 0,
            zcash_address: connection.map(|c| c.address),
        };
        checkout::save_private(&self.job_path(&id), &serde_json::to_vec_pretty(&job)?)?;
        jobs.insert(id.clone(), job.clone());
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
        if self.expire_if_stale(&job)? {
            bail!("this approval expired; start the checkout again");
        }
        self.try_update(&job.id, |j| {
            ensure!(
                j.stage == Stage::AwaitingApproval,
                "this checkout was already decided"
            );
            ensure!(
                now() < j.created_at.saturating_add(APPROVAL_SECS),
                "this approval expired"
            );
            ensure!(
                digest == j.digest && j.digest == j.terms.digest(),
                "the terms changed since this page was shown"
            );
            j.stage = if approve {
                Stage::BuildingPayment
            } else {
                Stage::Rejected
            };
            Ok(())
        })?;
        if approve {
            self.queue
                .lock()
                .expect("queue lock")
                .send(job.id)
                .context("the wallet's worker stopped")?;
        }
        Ok(())
    }

    fn chains(&self) -> Chains {
        Chains {
            rpc_url: self.config.rpc_url.clone(),
            relay: self.config.relay,
            peers: if self.config.peers.is_empty() {
                crate::seeders(self.config.network)
            } else {
                self.config.peers.clone()
            },
            network: self.config.network,
        }
    }

    /// Runs one approved job to its end. Called from the single worker thread, so one checkout at a
    /// time uses the Zcash wallet and the prover.
    pub fn run(&self, id: &str) {
        let Some(job) = self.get(id) else {
            return;
        };
        if job.payment == PaymentState::Settled
            || matches!(job.stage, Stage::Done { .. } | Stage::Rejected)
        {
            return;
        }
        if let Err(e) = self.run_steps(id) {
            println!("job {id}: recovery/checkout error: {e:#}");
            let _ = self.update(id, |j| {
                if j.needs_recovery() {
                    j.stage = Stage::Recovering;
                    j.retry_at = now().saturating_add(10);
                } else {
                    j.stage = Stage::Failed {
                        reason: "Checkout stopped before payment; nothing was paid.".into(),
                    };
                }
            });
        }
    }

    fn retry_due(&self) {
        let ids: Vec<String> = self
            .jobs
            .lock()
            .expect("jobs lock")
            .values()
            .filter(|j| j.needs_recovery() && j.stage == Stage::Recovering && j.retry_at <= now())
            .map(|j| j.id.clone())
            .collect();
        for id in ids {
            self.run(&id);
        }
    }

    fn finish_settlement(&self, job: &Job, event: Event, signature: String) -> Result<()> {
        let escrow = job.escrow.as_deref().context("no Escrow recorded")?;
        let stage = match event {
            Event::Claimed => Stage::Done {
                escrow: escrow.to_string(),
                claim: signature,
                txid_found_on_solana: self
                    .txid_found(job.txid.as_deref().context("no payment recorded")?, escrow),
            },
            Event::Refunded => Stage::Failed {
                reason: "The merchant refunded the Escrow; no ticket was delivered.".into(),
            },
            Event::Created => bail!("creation is not settlement"),
        };
        self.update(&job.id, |j| {
            j.stage = stage;
            j.payment = PaymentState::Settled;
        })?;
        Ok(())
    }

    /// Resumes the original prepared package if signing was interrupted; after signed bytes are
    /// saved, only those bytes may be sent. An uncertain send may already have mined.
    fn recover_payment(&self, job: &Job, chains: &Chains) -> Result<bool> {
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        let address = parse_address(job.escrow.as_deref().context("no Escrow recorded")?)?;
        if let Some((owner, data)) = rpc
            .account_with_owner(&address)
            .map_err(anyhow::Error::msg)?
        {
            ensure!(owner == ESCROW_ID, "saved Escrow has another owner");
            let escrow = crate::solana::read_escrow(&data)?;
            let txid = crate::internal_txid(job.txid.as_deref().context("no payment recorded")?)?;
            let salt = crate::parse_salt(job.salt.as_deref().context("no salt recorded")?)?;
            ensure!(
                Escrow::find_pda(&escrow.payee, escrow.commitment).0 == address
                    && escrow.commitment == zolar_claim::payment_commitment(&txid, &salt)?
                    && escrow.relay == chains.relay
                    && escrow.payout == parse_address(&job.terms.payout)?
                    && escrow.mint == parse_address(&job.terms.item.mint)?
                    && escrow.amount >= job.terms.item.amount
                    && escrow.bond_required == 0
                    && Some(escrow.expiry_height) == job.expiry,
                "saved Escrow differs from the approved payment"
            );
        } else {
            // Readable funded state admits a Claim even when history is unavailable. Absence only
            // establishes settlement with a successful finalized instruction, never by itself.
            if let Some((event, signature)) = settlement::history(&rpc, &address)?.settled {
                self.finish_settlement(job, event, signature)?;
                return Ok(true);
            }
            bail!("Escrow absent; finalized settlement remains unresolved");
        }
        if job.payment != PaymentState::BroadcastAcknowledged {
            let signed = self
                .work_dir(&job.id)
                .join(match self.config.payment_wallet {
                    PaymentWallet::Devtool(_) => "combined.pczt",
                    PaymentWallet::Zkool(_) => "created.pczt",
                });
            let replay = (|| -> Result<()> {
                let summary = zolar_pczt::summary(&std::fs::read(&signed)?)?;
                ensure!(
                    Some(summary.txid.as_str()) == job.txid.as_deref()
                        && Some(summary.expiry_height) == job.expiry,
                    "saved signed payment identity differs"
                );
                let checked = payer::check_escrow(&Check {
                    chains,
                    escrow_address: address,
                    payer_wallet: parse_address(&job.terms.payout)?,
                    pczt: &signed,
                    salt: crate::parse_salt(job.salt.as_deref().context("no salt recorded")?)?,
                    accept_freeze_authority: false,
                    terms: payer::Terms {
                        mint: parse_address(&job.terms.item.mint)?,
                        min_amount: job.terms.item.amount,
                        max_bond: 0,
                        accept_private_only: true,
                    },
                    finalized_wait: Duration::from_secs(5),
                })?;
                ensure!(job.digest == job.terms.digest(), "approved terms changed");
                if let PaymentWallet::Zkool(wallet) = &self.config.payment_wallet {
                    let work = self.work_dir(&job.id);
                    if !work.join("signed.tx").exists() {
                        ensure!(
                            job.payment == PaymentState::Signing,
                            "the saved broadcast payment is missing; refusing to sign a replacement"
                        );
                        let tip = payer::check_before_broadcast(chains, &checked)?;
                        wallet.sign(&checked, &work, tip)?;
                    }
                    self.update(&job.id, |j| {
                        j.broadcasting = true;
                        j.payment = PaymentState::BroadcastUncertain;
                    })?;
                }
                self.broadcast(chains, &checked, &self.work_dir(&job.id))?;
                self.update(&job.id, |j| j.payment = PaymentState::BroadcastAcknowledged)?;
                Ok(())
            })();
            if let Err(e) = replay {
                if self
                    .get(&job.id)
                    .is_some_and(|current| current.payment == PaymentState::Signing)
                {
                    // signTx cannot broadcast. Retry the original signing operation instead of
                    // blocking the worker waiting for a payment that has never been submitted.
                    return Err(e);
                }
                println!(
                    "job {}: exact payment replay not acknowledged: {e:#}; checking for its Claim",
                    job.id
                );
            }
        }
        Ok(false)
    }

    fn broadcast(
        &self,
        chains: &Chains,
        checked: &payer::Checked,
        work: &std::path::Path,
    ) -> Result<()> {
        match &self.config.payment_wallet {
            PaymentWallet::Devtool(devtool) => {
                payer::broadcast(chains, devtool, checked, &work.join("combined.pczt"))
            }
            PaymentWallet::Zkool(wallet) => {
                let tip = payer::check_before_broadcast(chains, checked)?;
                wallet.broadcast(checked, work, tip)
            }
        }
    }

    fn run_steps(&self, id: &str) -> Result<()> {
        let job = self.get(id).context("no such job")?;
        let chains = self.chains();
        let work = self.work_dir(id);
        checkout::private_dir(&work)?;
        if job.needs_recovery() {
            if self.recover_payment(&job, &chains)? {
                return Ok(());
            }
        } else {
            self.pay(&job, &chains, &work.join("created.pczt"))?;
        }
        let job = self.get(id).context("no such job")?;
        let escrow = job.escrow.clone().context("no Escrow recorded")?;
        let txid = job.txid.clone().context("no transaction recorded")?;
        let salt = crate::parse_salt(job.salt.as_deref().context("no salt recorded")?)?;
        if let Some(signature) = &job.claim_signature {
            let rpc = HttpRpc::new(&chains.rpc_url);
            match settlement::transaction(&rpc, signature) {
                Ok(Some(tx))
                    if settlement::event(&tx, &parse_address(&escrow)?) == Some(Event::Claimed) =>
                {
                    return self.finish_settlement(&job, Event::Claimed, signature.clone());
                }
                Err(e) => println!("job {id}: saved Claim lookup unavailable: {e:#}"),
                _ => {}
            }
            ensure!(
                now() >= job.claim_submitted_at.saturating_add(60),
                "Claim awaits finality"
            );
            // A confirmed submission can roll off a fork. Once the wait has elapsed, a live
            // Escrow permits another Claim for the same payment; the program prevents double pay.
            ensure!(
                crate::rpc::finalized_account(&chains.rpc_url, &parse_address(&escrow)?)?.is_some(),
                "Escrow absent; Claim outcome remains unresolved"
            );
            self.update(id, |j| j.claim_signature = None)?;
        }
        let (sent_tip, finality_depth) = job.sent.unwrap_or((0, 10));
        self.set_stage(
            id,
            Stage::WaitingForFinality {
                escrow: escrow.clone(),
                tip: 0,
                final_height: 0,
                expiry: job.expiry.unwrap_or(0),
                sent_tip,
                finality_depth,
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
        self.update(id, |j| {
            j.stage = Stage::Claimed {
                escrow: escrow.clone(),
                claim: claim.clone(),
            };
            j.claim_signature = Some(claim);
            j.claim_submitted_at = now();
        })?;
        // Submission confirmation is not yet a permanent settlement. The worker reconciles the
        // finalized instruction before marking success, including after a crash here.
        bail!("Claim submitted; waiting for finalized settlement")
    }

    /// Builds the payment for the approved terms, has the merchant lock the item, checks the Escrow
    /// against the approved terms, signs and broadcasts.
    fn pay(&self, job: &Job, chains: &Chains, pczt: &std::path::Path) -> Result<()> {
        let id = &job.id;
        let terms = &job.terms;
        match &self.config.payment_wallet {
            PaymentWallet::Devtool(devtool) => payer::create_pczt(
                chains,
                devtool,
                &terms.zcash_address,
                terms.item.price_zats,
                &terms.memo,
                18,
                pczt,
            )?,
            PaymentWallet::Zkool(wallet) => {
                let connection = wallet.connection()?;
                ensure!(
                    job.zcash_address.as_deref() == Some(connection.address.as_str()),
                    "the approved Zkool account changed"
                );
                let mut rpc = HttpRpc::new(&chains.rpc_url);
                let relay = read_relay(&mut rpc, &chains.relay)?;
                let tip = relay.view()?.tip_height();
                wallet.prepare(
                    &terms.zcash_address,
                    terms.item.price_zats,
                    &terms.memo,
                    tip,
                    &self.work_dir(id),
                )?;
            }
        }
        let txid = zolar_pczt::summary(&std::fs::read(pczt)?)?.txid;
        self.update(id, |j| {
            j.stage = Stage::LockingItem;
            j.txid = Some(txid);
        })?;
        // The merchant gets a copy without the wallet's viewing key, note values or change.
        let payee_copy = zolar_pczt::redact_for_payee(&std::fs::read(pczt)?)?;
        let created = propose(&job.merchant_url, &terms.order_id, &payee_copy)?;
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
        match &self.config.payment_wallet {
            PaymentWallet::Devtool(devtool) => {
                payer::sign(devtool, &checked, pczt, &self.work_dir(id))?;
            }
            PaymentWallet::Zkool(wallet) => {
                let tip = payer::check_before_broadcast(chains, &checked)?;
                // Signing has no broadcast side effect in the external wallet. Persist intent
                // first so a lost response or crash resumes the original prepared package.
                self.update(id, |j| j.payment = PaymentState::Signing)?;
                wallet.sign(&checked, &self.work_dir(id), tip)?;
            }
        }
        self.update(id, |j| {
            j.stage = Stage::Paying {
                escrow: created.escrow.clone(),
            };
            j.broadcasting = true;
            j.payment = PaymentState::BroadcastUncertain;
        })?;
        self.broadcast(chains, &checked, &self.work_dir(id))?;
        self.update(id, |j| j.payment = PaymentState::BroadcastAcknowledged)?;
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        if let Ok(relay) = read_relay(&mut rpc, &self.config.relay)
            && let Ok(view) = relay.view()
        {
            let sent = (view.tip_height(), view.finality_depth());
            self.update(id, |j| j.sent = Some(sent))?;
        }
        Ok(())
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

    /// Ends a checkout nobody approved in time, so the page stops waiting. Nothing was paid.
    fn expire_if_stale(&self, job: &Job) -> Result<bool> {
        if job.stage != Stage::AwaitingApproval || now() < job.created_at + APPROVAL_SECS {
            return Ok(false);
        }
        let updated = self.update(&job.id, |j| {
            if j.stage == Stage::AwaitingApproval
                && now() >= j.created_at.saturating_add(APPROVAL_SECS)
            {
                j.stage = Stage::Failed {
                    reason: "the approval expired; nothing was paid".into(),
                };
            }
        })?;
        Ok(matches!(updated.stage, Stage::Failed { .. }))
    }

    fn view(&self, id: &str, token: Option<&str>) -> Result<serde_json::Value> {
        let job = self.get(id).context("no such job")?;
        ensure!(
            token == Some(job.token.as_str()),
            "this job belongs to another page"
        );
        self.expire_if_stale(&job)?;
        let job = self.get(id).context("no such job")?;
        let mut stage = job.stage.clone();
        if let Stage::Failed { reason } = &mut stage {
            *reason = if job.payment == PaymentState::Settled {
                "The merchant refunded the Escrow; no ticket was delivered."
            } else {
                "Checkout stopped before payment; nothing was paid."
            }
            .to_string();
        }
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

    /// Restore read access only for the same allowed shop and configured external account.
    /// An unresolved payment takes priority over receipts. Approval capability is never returned.
    fn resume_for_account(&self, origin: &str, address: &str) -> Option<serde_json::Value> {
        self.jobs
            .lock()
            .expect("jobs lock")
            .values()
            .filter(|j| {
                j.origin == origin
                    && j.zcash_address.as_deref() == Some(address)
                    && (j.payment != PaymentState::Unsent
                        || matches!(
                            j.stage,
                            Stage::BuildingPayment
                                | Stage::LockingItem
                                | Stage::CheckingEscrow { .. }
                        ))
                    && j.stage != Stage::Rejected
            })
            .max_by_key(|j| {
                (
                    j.needs_recovery()
                        || !matches!(
                            j.stage,
                            Stage::Done { .. } | Stage::Failed { .. } | Stage::Rejected
                        ),
                    j.created_at,
                    j.id.clone(),
                )
            })
            .map(|j| serde_json::json!({"job":j.id,"token":j.token}))
    }

    fn approval_page(&self, secret: &str) -> Result<String> {
        let job = self.job_by_secret(secret).context("no such approval")?;
        let t = &job.terms;
        let e = checkout::escape;
        let zec = t.item.price_zats as f64 / 1e8;
        let decided = job.stage != Stage::AwaitingApproval;
        let short = |s: &str| {
            if s.chars().count() <= 14 {
                s.to_string()
            } else {
                let head: String = s.chars().take(8).collect();
                let tail: String = s
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("{head}…{tail}")
            }
        };
        let mark = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(include_bytes!("wallet-mark.png"))
        );
        let fields = [
            (
                "wallet_name",
                match self.config.payment_wallet {
                    PaymentWallet::Zkool(_) => "Zkool connection".to_string(),
                    PaymentWallet::Devtool(_) => "Local test wallet".to_string(),
                },
            ),
            (
                "wallet_account",
                e(job.zcash_address.as_deref().unwrap_or("CLI test account")),
            ),
            ("mark", mark),
            ("address_short", e(&short(&t.zcash_address))),
            ("mint_short", e(&short(&t.item.mint))),
            ("payout_short", e(&short(&t.payout))),
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
            (Method::Options, ["checkout"] | ["jobs", _] | ["status"] | ["resume"]) if shop.is_some() => {
                Ok(checkout::json(204, &serde_json::json!({})))
            }
            (Method::Post, ["checkout"]) => match shop.as_deref() {
                Some(o) => checkout::read_json(request, 4096)
                    .and_then(|r| self.start(o, r))
                    .map(|s| checkout::json(201, &s)),
                None => Err(anyhow::anyhow!("this site may not use the wallet")),
            },
            // Lets a shop page tell whether the customer runs a wallet it may use.
            (Method::Get, ["status"]) if shop.is_some() => match &self.config.payment_wallet {
                PaymentWallet::Zkool(wallet) => wallet.identity().map(|c| checkout::json(200,
                    &serde_json::json!({"wallet":c.name,"network":c.network,
                        "relay":self.config.relay.to_string(),"address":c.address,"account":c.account}))),
                PaymentWallet::Devtool(_) => Ok(checkout::json(200, &serde_json::json!({
                    "wallet": "zolar-wallet",
                    "network": network_name(self.config.network),
                    "relay": self.config.relay.to_string(),
                }))),
            },
            (Method::Get, ["resume"]) if shop.is_some() => match &self.config.payment_wallet {
                PaymentWallet::Zkool(wallet) => wallet.identity().map(|identity|
                    checkout::json(200,&self.resume_for_account(shop.as_deref().expect("allowed origin"),&identity.address))),
                PaymentWallet::Devtool(_) => Ok(checkout::json(200,&serde_json::Value::Null)),
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
    let service = match wallet.config.payment_wallet {
        PaymentWallet::Zkool(_) => "zolar-connect",
        PaymentWallet::Devtool(_) => "zolar-wallet",
    };
    println!("{service} listening on http://{bind}");
    let worker = wallet.clone();
    std::thread::spawn(move || {
        loop {
            match queue.recv_timeout(Duration::from_secs(1)) {
                Ok(id) => worker.run(&id),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            worker.retry_due();
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
            peers: vec![],
            rpc_url: "http://127.0.0.1:1".into(),
            relay: Address::new_from_array([4; 32]),
            fee_payer: dir.join("fee.json"),
            payment_wallet: PaymentWallet::Devtool(Devtool {
                binary: dir.join("devtool"),
                wallet_dir: dir.join("w"),
                identity: dir.join("id"),
            }),
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
            merchant: "Shielded Night".into(),
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
        wallet_with_config(dir, stage, broadcasting, config(dir))
    }

    fn wallet_with_config(
        dir: &std::path::Path,
        stage: Stage,
        broadcasting: bool,
        configuration: Config,
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
            sent: None,
            payment: PaymentState::Unsent,
            claim_signature: None,
            retry_at: 0,
            claim_submitted_at: 0,
            zcash_address: None,
        };
        std::fs::create_dir_all(dir.join("jobs")).unwrap();
        std::fs::write(
            dir.join("jobs/job1.json"),
            serde_json::to_vec(&job).unwrap(),
        )
        .unwrap();
        let (w, rx) = Wallet::new(configuration).unwrap();
        (w, rx, job)
    }

    #[test]
    fn resume_is_read_only_and_scoped_to_the_shop_and_external_account_with_active_payment_priority()
     {
        let dir = tempdir();
        let (w, rx, _) = wallet_with_job(&dir, Stage::Recovering, true);
        w.update("job1", |j| {
            j.payment = PaymentState::BroadcastUncertain;
            j.zcash_address = Some("utest1selected".into());
            j.created_at = 1;
        })
        .unwrap();
        let mut other = w.get("job1").unwrap();
        other.id = "later-receipt".into();
        other.token = "read-receipt".into();
        other.created_at = 2;
        other.payment = PaymentState::Settled;
        other.stage = Stage::Done {
            escrow: "E".into(),
            claim: "C".into(),
            txid_found_on_solana: Some(false),
        };
        w.jobs.lock().unwrap().insert(other.id.clone(), other);
        let resumed = w
            .resume_for_account("http://localhost:5175", "utest1selected")
            .unwrap();
        assert_eq!(resumed, serde_json::json!({"job":"job1","token":"tok"}));
        assert_eq!(resumed.as_object().unwrap().len(), 2);
        assert!(
            w.resume_for_account("http://other-shop", "utest1selected")
                .is_none()
        );
        assert!(
            w.resume_for_account("http://localhost:5175", "utest1other-account")
                .is_none()
        );
        assert!(
            rx.try_recv().is_ok(),
            "startup had queued the saved paid job"
        );
        assert!(
            rx.try_recv().is_err(),
            "resuming never queues payment or approval"
        );
        w.update("job1", |j| {
            j.payment = PaymentState::Unsent;
            j.broadcasting = false;
        })
        .unwrap();
        assert_eq!(
            w.resume_for_account("http://localhost:5175", "utest1selected")
                .unwrap()["job"],
            "later-receipt"
        );
        w.update("job1", |j| {
            j.stage = Stage::CheckingEscrow { escrow: "E".into() };
        })
        .unwrap();
        assert_eq!(
            w.resume_for_account("http://localhost:5175", "utest1selected")
                .unwrap()["job"],
            "job1",
            "an already approved prepayment job must win over a newer receipt"
        );
        drop(w);
        std::fs::remove_dir_all(dir).unwrap();
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
    fn an_approval_left_too_long_ends_the_checkout_with_nothing_paid() {
        let dir = tempdir();
        let (w, rx, job) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        w.update("job1", |j| j.created_at = now() - APPROVAL_SECS - 1)
            .unwrap();
        let v = w.view("job1", Some("tok")).unwrap();
        assert_eq!(v["stage"], "failed");
        assert!(v["reason"].as_str().unwrap().contains("nothing was paid"));
        assert!(w.decide("secret", true, &job.digest).is_err());
        assert!(rx.try_recv().is_err(), "nothing queued");
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
    fn configured_peers_replace_discovery_without_changing_the_trusted_network_or_relay() {
        let dir = std::env::temp_dir().join(format!("zolar-peer-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut c = config(&dir);
        let relay = c.relay;
        c.peers = vec!["127.0.0.1:18233".into(), "example.test:18233".into()];
        let (w, _) = Wallet::new(c).unwrap();
        let chains = w.chains();
        assert_eq!(chains.peers, ["127.0.0.1:18233", "example.test:18233"]);
        assert_eq!(chains.network, Network::Testnet);
        assert_eq!(chains.relay, relay);
        drop(w);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_signing_intent_survives_restart_with_or_without_saved_signed_bytes() {
        for saved in [false, true] {
            let dir = tempdir();
            let (wallet, _, _) = wallet_with_job(
                &dir,
                Stage::CheckingEscrow {
                    escrow: "escrow".into(),
                },
                false,
            );
            let work = wallet.work_dir("job1");
            checkout::private_dir(&work).unwrap();
            std::fs::write(work.join("prepared.zkool"), b"original prepared package").unwrap();
            if saved {
                std::fs::write(work.join("signed.tx"), b"original signed transaction").unwrap();
            }
            wallet
                .update("job1", |j| {
                    j.payment = PaymentState::Signing;
                    j.txid = Some("fixed payment identity".into());
                })
                .unwrap();
            drop(wallet);
            let (wallet, queue) = Wallet::new(config(&dir)).unwrap();
            assert_eq!(queue.try_recv().unwrap(), "job1");
            let resumed = wallet.get("job1").unwrap();
            assert_eq!(resumed.stage, Stage::Recovering);
            assert_eq!(resumed.payment, PaymentState::Signing);
            assert_eq!(resumed.txid.as_deref(), Some("fixed payment identity"));
            assert_eq!(
                std::fs::read(work.join("prepared.zkool")).unwrap(),
                b"original prepared package"
            );
            if saved {
                assert_eq!(
                    std::fs::read(work.join("signed.tx")).unwrap(),
                    b"original signed transaction"
                );
            }
            drop(wallet);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_payment_already_sent_is_claimed_after_a_restart() {
        let dir = tempdir();
        let (_w, rx, _) = wallet_with_job(&dir, Stage::Paying { escrow: "E".into() }, true);
        assert_eq!(rx.try_recv().unwrap(), "job1", "resumed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_claim_error_after_payment_is_retried_after_restart() {
        let dir = tempdir();
        let (w, rx, _) = wallet_with_job(&dir, Stage::Paying { escrow: "E".into() }, true);
        assert_eq!(rx.try_recv().unwrap(), "job1");
        w.update("job1", |j| {
            j.escrow = Some(Address::new_from_array([5; 32]).to_string());
            j.txid = Some("00".repeat(32));
            j.salt = Some("00".repeat(32));
            j.expiry = Some(10);
        })
        .unwrap();
        w.run("job1");
        drop(w);
        let (_restarted, retry) = Wallet::new(config(&dir)).unwrap();
        let resumed = retry.try_recv();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(
            resumed.unwrap(),
            "job1",
            "a paid job must remain recoverable"
        );
    }

    #[test]
    fn legacy_paid_failures_are_migrated_without_exposing_their_diagnostics() {
        let dir = tempdir();
        let (w, rx, _) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        drop(w);
        drop(rx);
        let path = dir.join("jobs/job1.json");
        let mut old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for field in [
            "payment",
            "claim_signature",
            "retry_at",
            "claim_submitted_at",
        ] {
            old.as_object_mut().unwrap().remove(field);
        }
        old["broadcasting"] = serde_json::json!(true);
        old["stage"] =
            serde_json::json!({"stage":"failed","reason":"private txid deadbeef salt c0ffee"});
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let (w, rx) = Wallet::new(config(&dir)).unwrap();
        assert_eq!(rx.try_recv().unwrap(), "job1");
        let view = w.view("job1", Some("tok")).unwrap().to_string();
        assert!(view.contains("recovering"));
        assert!(!view.contains("deadbeef") && !view.contains("c0ffee"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn runtime_recovery_retries_the_saved_identity_without_building_another_payment() {
        let dir = tempdir();
        let (w, rx, _) = wallet_with_job(&dir, Stage::Paying { escrow: "E".into() }, true);
        rx.try_recv().unwrap();
        w.update("job1", |j| {
            j.escrow = Some(Address::new_from_array([5; 32]).to_string());
            j.txid = Some("00".repeat(32));
            j.salt = Some("00".repeat(32));
            j.expiry = Some(10);
        })
        .unwrap();
        w.run("job1");
        assert_eq!(w.get("job1").unwrap().stage, Stage::Recovering);
        w.update("job1", |j| j.retry_at = 0).unwrap();
        w.retry_due();
        let job = w.get("job1").unwrap();
        assert_eq!(job.stage, Stage::Recovering);
        assert!(job.retry_at > now());
        assert_eq!(job.txid, Some("00".repeat(32)));
        assert!(!w.work_dir("job1").join("created.pczt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_approval_requests_queue_the_checkout_only_once() {
        let dir = tempdir();
        let (w, rx, job) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let w = w.clone();
                let digest = job.digest.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    w.decide("secret", true, &digest).is_ok()
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|ok| *ok)
                .count(),
            1
        );
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec!["job1"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_persistence_never_publishes_an_approval_and_only_one_service_writes_a_directory() {
        let dir = tempdir();
        let (w, rx, job) = wallet_with_job(&dir, Stage::AwaitingApproval, false);
        assert!(Wallet::new(config(&dir)).is_err());
        std::fs::remove_file(w.job_path("job1")).unwrap();
        std::fs::create_dir(w.job_path("job1")).unwrap();
        assert!(w.decide("secret", true, &job.digest).is_err());
        assert_eq!(w.get("job1").unwrap().stage, Stage::AwaitingApproval);
        assert!(rx.try_recv().is_err());
        std::fs::remove_dir(w.job_path("job1")).unwrap();
        drop(w);
        assert!(
            Wallet::new(config(&dir)).is_ok(),
            "the lock is released when the service exits"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_dropped_claim_can_be_retried_for_a_live_escrow_when_history_is_unavailable() {
        use crate::test_support as fixture;
        let dir = tempdir();
        let (escrow, data) = fixture::escrow(
            Address::new_from_array([6; 32]),
            Address::new_from_array([2; 32]),
            Address::new_from_array([3; 32]),
            Address::new_from_array([4; 32]),
            &"00".repeat(32),
            [0; 32],
            100,
        );
        let rpc = fixture::Rpc::new(move |request| match request["method"].as_str().unwrap() {
            "getAccountInfo" => Ok(fixture::account(ESCROW_ID, &data)),
            _ => {
                Err(serde_json::json!({"code":-32004,"message":"transaction history unavailable"}))
            }
        });
        let mut configuration = config(&dir);
        configuration.rpc_url = rpc.url.clone();
        let (w, rx, _) = wallet_with_config(
            &dir,
            Stage::Paying {
                escrow: escrow.to_string(),
            },
            true,
            configuration,
        );
        rx.try_recv().unwrap();
        w.update("job1", |j| {
            j.escrow = Some(escrow.to_string());
            j.txid = Some("00".repeat(32));
            j.salt = Some("00".repeat(32));
            j.expiry = Some(100);
            j.payment = PaymentState::BroadcastAcknowledged;
            j.claim_signature = Some("dropped-signature".into());
            j.claim_submitted_at = 0;
        })
        .unwrap();
        w.run("job1");
        let job = w.get("job1").unwrap();
        assert!(
            job.claim_signature.is_none(),
            "an unresolvable submission must not prevent another Claim"
        );
        assert_eq!(
            job.stage,
            Stage::Recovering,
            "the unavailable fee key still needs recovery"
        );
        let requests = rpc.requests.lock().unwrap();
        assert!(requests.iter().any(|r| r["method"] == "getTransaction"));
        assert!(
            !requests
                .iter()
                .any(|r| r["method"] == "getSignaturesForAddress"
                    || r["method"] == "sendTransaction")
        );
        drop(requests);
        drop(w);
        drop(rpc);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_claim_that_landed_before_a_restart_is_reconciled_without_paying_again() {
        use crate::test_support as fixture;
        let dir = tempdir();
        let escrow = Address::new_from_array([5; 32]);
        let rpc = fixture::Rpc::new(move |request| match request["method"].as_str().unwrap() {
            "getAccountInfo" => Ok(serde_json::json!({"value":null})),
            "getSignaturesForAddress" => Ok(
                serde_json::json!([{"signature":"claim","err":null},{"signature":"create","err":null}]),
            ),
            "getTransaction" => {
                let signature = request["params"][0].as_str().unwrap();
                Ok(fixture::transaction(
                    signature,
                    escrow,
                    if signature == "claim" { 4 } else { 0 },
                ))
            }
            other => panic!("settled recovery must not call {other}"),
        });
        let mut configuration = config(&dir);
        configuration.rpc_url = rpc.url.clone();
        let (w, rx, _) = wallet_with_config(
            &dir,
            Stage::Paying {
                escrow: escrow.to_string(),
            },
            true,
            configuration,
        );
        rx.try_recv().unwrap();
        w.update("job1", |j| {
            j.escrow = Some(escrow.to_string());
            j.txid = Some("00".repeat(32));
            j.salt = Some("00".repeat(32));
        })
        .unwrap();
        w.run("job1");
        assert!(matches!(w.get("job1").unwrap().stage,Stage::Done{claim,..} if claim == "claim"));
        drop(w);
        drop(rpc);
        std::fs::remove_dir_all(dir).unwrap();
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
        assert!(!page.contains("{{"), "every placeholder is filled");
        assert!(!page.contains("Zolar Wallet"));
        assert!(
            !page.contains("data:image/png;base64,"),
            "authorization is not a new branded wallet"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
