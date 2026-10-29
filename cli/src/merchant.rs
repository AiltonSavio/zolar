//! `zolar-merchant`: the Payee side of a checkout as an HTTP service.
//!
//! A shop page creates an order (`POST /orders`). The customer's wallet reads its terms
//! (`GET /orders/{id}/terms`) and, once the customer approved them, proposes the unsigned payment
//! (`POST /orders/{id}/escrow`); the merchant checks it with its viewing key and locks the item in a
//! private-only Escrow. A worker marks claimed orders and takes back the items of abandoned ones.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use escrow_client::{accounts::Escrow, programs::ESCROW_ID};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_signer::Signer;
use tiny_http::{Method, Request, Server};
use zolar_relayer::{
    chain::Cluster,
    rpc::{HttpRpc, PreparedTransaction},
};

use crate::{
    Network,
    checkout::{
        self, CreateOrder, EscrowCreated, Item, OrderStatus, OrderTerms, OrderView, ProposePayment,
        Reply,
    },
    payee::{CreateEscrow, RefundArgs, prepare_escrow, refund},
    settlement::{self, Event},
    solana::{associated_token_account, load_keypair, parse_address, parse_token_account},
};

#[derive(Clone)]
pub struct Config {
    pub name: String,
    pub network: Network,
    pub rpc_url: String,
    pub relay: Address,
    pub keypair: PathBuf,
    pub ufvk_file: PathBuf,
    pub zcash_address: String,
    pub items: Vec<Item>,
    pub data_dir: PathBuf,
    pub allowed_origins: Vec<String>,
    /// Orders that may hold or wait for an Escrow at once, all customers together.
    pub max_open_orders: usize,
    /// Open orders per payout wallet.
    pub max_orders_per_payout: usize,
    /// An order with no proposed payment lapses after this long.
    pub order_ttl: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Proposal {
    digest: String,
    salt: String,
    escrow: String,
    expiry_height: u32,
    transaction: Option<PreparedTransaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Order {
    terms: OrderTerms,
    status: OrderStatus,
    escrow: Option<String>,
    created_at: u64,
    /// A payment is being checked and the Escrow created; a second proposal must wait.
    #[serde(default)]
    proposing: bool,
    #[serde(default)]
    proposal: Option<Proposal>,
    #[serde(default)]
    recovery_version: u8,
    #[serde(default)]
    legacy_unresolved: bool,
}

impl Order {
    fn holds_inventory(&self) -> bool {
        self.legacy_unresolved || matches!(self.status, OrderStatus::Open | OrderStatus::Escrowed)
    }

    fn view(&self) -> OrderView {
        OrderView {
            order_id: self.terms.order_id.clone(),
            status: self.status,
            escrow: self.escrow.clone(),
        }
    }
}

pub struct Merchant {
    config: Config,
    _data_lock: checkout::DataDirLock,
    orders: Mutex<HashMap<String, Order>>,
    operations: Mutex<()>,
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

impl Merchant {
    pub fn new(config: Config) -> Result<Arc<Self>> {
        let data_lock = checkout::lock_data_dir(&config.data_dir)?;
        checkout::private_dir(&config.data_dir.join("orders"))?;
        let mut orders = HashMap::new();
        for entry in std::fs::read_dir(config.data_dir.join("orders"))?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let mut order: Order = serde_json::from_slice(&std::fs::read(&path)?)
                    .with_context(|| format!("reading {}", path.display()))?;
                // Old versions could have sent without saving the address. Preserve all legacy
                // identity files and never interpret the missing address as proof of no creation.
                order.proposing = false;
                if order.recovery_version == 0 && order.proposal.is_none() {
                    let base = config.data_dir.join("orders").join(&order.terms.order_id);
                    order.legacy_unresolved = base.with_extension("pczt").exists()
                        && !base.with_extension("salt").exists();
                }
                orders.insert(order.terms.order_id.clone(), order);
            }
        }
        Ok(Arc::new(Self {
            config,
            _data_lock: data_lock,
            orders: Mutex::new(orders),
            operations: Mutex::new(()),
        }))
    }

    fn order_dir(&self) -> PathBuf {
        self.config.data_dir.join("orders")
    }

    fn save(&self, order: &Order) -> Result<()> {
        let path = self
            .order_dir()
            .join(format!("{}.json", order.terms.order_id));
        checkout::save_private(&path, &serde_json::to_vec_pretty(order)?)
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Order)) -> Result<Order> {
        let mut orders = self.orders.lock().expect("orders lock");
        let mut next = orders.get(id).context("no such order")?.clone();
        f(&mut next);
        self.save(&next)?;
        orders.insert(id.to_string(), next.clone());
        Ok(next)
    }

    pub fn items(&self) -> Vec<serde_json::Value> {
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        let payee = load_keypair(&self.config.keypair).ok().map(|k| k.pubkey());
        self.config
            .items
            .iter()
            .map(|item| {
                let left = payee.and_then(|payee| {
                    let mint = parse_address(&item.mint).ok()?;
                    let account = rpc
                        .account(&associated_token_account(&payee, &mint))
                        .ok()??;
                    Some(parse_token_account(&account).ok()?.amount / item.amount.max(1))
                });
                serde_json::json!({ "item": item, "left": left })
            })
            .collect()
    }

    pub fn create_order(&self, request: CreateOrder) -> Result<OrderTerms> {
        let item = self
            .config
            .items
            .iter()
            .find(|i| i.id == request.item_id)
            .context("no such item")?
            .clone();
        parse_address(&request.payout).context("payout is not a Solana address")?;
        let mut orders = self.orders.lock().expect("orders lock");
        let open = orders.values().filter(|o| o.holds_inventory()).count();
        ensure!(
            open < self.config.max_open_orders,
            "too many checkouts in progress; try again in a few minutes"
        );
        let mine = orders
            .values()
            .filter(|o| o.holds_inventory() && o.terms.payout == request.payout)
            .count();
        ensure!(
            mine < self.config.max_orders_per_payout,
            "this wallet already has a checkout in progress"
        );
        let order_id = checkout::random_id(8);
        let terms = OrderTerms {
            memo: format!("zolar order {order_id} {}", checkout::random_id(4)),
            order_id: order_id.clone(),
            merchant: self.config.name.clone(),
            item,
            zcash_address: self.config.zcash_address.clone(),
            payout: request.payout,
            relay: self.config.relay.to_string(),
            network: network_name(self.config.network).to_string(),
        };
        let order = Order {
            terms: terms.clone(),
            status: OrderStatus::Open,
            escrow: None,
            created_at: now(),
            proposing: false,
            proposal: None,
            recovery_version: 1,
            legacy_unresolved: false,
        };
        self.save(&order)?;
        orders.insert(order_id, order);
        Ok(terms)
    }

    fn get(&self, id: &str) -> Option<Order> {
        self.orders.lock().expect("orders lock").get(id).cloned()
    }

    /// Checks the customer's unsigned payment against the order with the viewing key and locks the
    /// item in a private-only Escrow paying the order's payout wallet.
    pub fn propose(&self, id: &str, request: ProposePayment) -> Result<EscrowCreated> {
        let pczt = base64::engine::general_purpose::STANDARD
            .decode(request.pczt.trim())
            .context("pczt is not base64")?;
        ensure!(pczt.len() < 1_000_000, "pczt too large");
        let _operation = self.operations.lock().expect("operations lock");
        let mut order = self.get(id).context("no such order")?;
        ensure!(
            matches!(order.status, OrderStatus::Open | OrderStatus::Escrowed),
            "the order is {:?}, not open",
            order.status
        );
        ensure!(
            !order.legacy_unresolved,
            "legacy proposal identity is incomplete; reconcile it before accepting another payment"
        );
        let digest = hex::encode(Sha256::digest(&pczt));
        let base = self.order_dir().join(id);
        if order.proposal.is_none() && base.with_extension("salt").exists() {
            order = self.recover_legacy(&order)?;
        }
        if let Some(proposal) = &order.proposal {
            ensure!(
                proposal.digest == digest,
                "this order already has another payment proposal"
            );
        } else {
            ensure!(
                now()
                    < order
                        .created_at
                        .saturating_add(self.config.order_ttl.as_secs()),
                "the order lapsed; start a new checkout"
            );
            self.update(id, |o| o.proposing = true)?;
            let result = self.prepare_proposal(&order, &pczt, digest);
            if let Err(error) = result {
                self.update(id, |o| o.proposing = false)?;
                return Err(error);
            }
            order = self.get(id).context("no such order")?;
        }
        // Matching retries work past the order's intake TTL: its immutable proposal already exists.
        self.reconcile(&order)
    }

    fn create_args(&self, order: &Order) -> Result<CreateEscrow> {
        let base = self.order_dir().join(&order.terms.order_id);
        Ok(CreateEscrow {
            network: self.config.network,
            rpc_url: self.config.rpc_url.clone(),
            keypair: self.config.keypair.clone(),
            relay: self.config.relay,
            pczt: base.with_extension("pczt"),
            ufvk_file: self.config.ufvk_file.clone(),
            expect_memo: order.terms.memo.clone(),
            min_zats: order.terms.item.price_zats,
            payout: parse_address(&order.terms.payout)?,
            mint: parse_address(&order.terms.item.mint)?,
            amount: order.terms.item.amount,
            salt_out: base.with_extension("salt"),
            bond: 0,
            private_only: true,
        })
    }

    fn prepare_proposal(&self, order: &Order, pczt: &[u8], digest: String) -> Result<()> {
        let args = self.create_args(order)?;
        checkout::save_private(&args.pczt, pczt)?;
        let salt = crate::random_salt();
        let prepared = prepare_escrow(&args, salt)?;
        let proposal = Proposal {
            digest,
            salt: hex::encode(salt),
            escrow: prepared.escrow.to_string(),
            expiry_height: prepared.expiry_height,
            transaction: Some(prepared.transaction),
        };
        // Save the whole identity and signed wire before submission. A lost response cannot create
        // another commitment, and a pending proposal remains visible to the refund worker.
        self.update(&order.terms.order_id, |o| {
            o.escrow = Some(proposal.escrow.clone());
            o.proposal = Some(proposal);
            o.proposing = false;
        })?;
        Ok(())
    }

    fn recover_legacy(&self, order: &Order) -> Result<Order> {
        let args = self.create_args(order)?;
        let pczt = std::fs::read(&args.pczt)
            .context("legacy PCZT missing; cannot replace its identity")?;
        let salt = crate::parse_salt(std::fs::read_to_string(&args.salt_out)?.trim())?;
        let summary = zolar_pczt::summary(&pczt)?;
        let commitment =
            zolar_claim::payment_commitment(&crate::internal_txid(&summary.txid)?, &salt)?;
        let payee = load_keypair(&self.config.keypair)?.pubkey();
        let escrow = Escrow::find_pda(&payee, commitment).0;
        self.update(&order.terms.order_id, |o| {
            o.escrow = Some(escrow.to_string());
            o.proposal = Some(Proposal {
                digest: hex::encode(Sha256::digest(&pczt)),
                salt: hex::encode(salt),
                escrow: escrow.to_string(),
                expiry_height: summary.expiry_height,
                transaction: None,
            });
            o.recovery_version = 1;
            o.status = OrderStatus::Open;
        })
    }

    fn answer(&self, proposal: &Proposal, signature: String) -> EscrowCreated {
        EscrowCreated {
            escrow: proposal.escrow.clone(),
            salt: proposal.salt.clone(),
            expiry_height: proposal.expiry_height,
            signature,
        }
    }

    fn reconcile(&self, order: &Order) -> Result<EscrowCreated> {
        let id = &order.terms.order_id;
        let mut proposal = order.proposal.clone().context("no saved proposal")?;
        let escrow = parse_address(&proposal.escrow)?;
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        if let Some((owner, data)) =
            zolar_relayer::chain::Cluster::account_with_owner(&mut rpc, &escrow)
                .map_err(anyhow::Error::msg)?
        {
            ensure!(owner == ESCROW_ID, "Escrow address has another owner");
            let account = crate::solana::read_escrow(&data)?;
            let salt = crate::parse_salt(&proposal.salt)?;
            let summary = zolar_pczt::summary(&std::fs::read(self.create_args(order)?.pczt)?)?;
            ensure!(
                account.commitment
                    == zolar_claim::payment_commitment(
                        &crate::internal_txid(&summary.txid)?,
                        &salt
                    )?
                    && account.payout == parse_address(&order.terms.payout)?
                    && account.mint == parse_address(&order.terms.item.mint)?
                    && account.amount == order.terms.item.amount
                    && account.expiry_height == proposal.expiry_height
                    && account.relay == self.config.relay
                    && account.claim_mode == 1
                    && account.bond_required == 0,
                "on-chain Escrow differs from the saved order"
            );
            self.update(id, |o| o.status = OrderStatus::Escrowed)?;
            let signature = match proposal.transaction.as_ref() {
                Some(tx) => tx.signature.clone(),
                None => settlement::history(&rpc, &escrow)?
                    .created
                    .context("legacy creation awaits finalized history")?,
            };
            return Ok(self.answer(&proposal, signature));
        }
        let history = settlement::history(&rpc, &escrow)?;
        if let Some((event, _)) = history.settled {
            self.update(id, |o| {
                o.status = if event == Event::Claimed {
                    OrderStatus::Claimed
                } else {
                    OrderStatus::Refunded
                }
            })?;
            bail!("this order has already settled");
        }
        ensure!(
            history.created.is_none(),
            "Escrow closed; waiting for positive settlement evidence"
        );
        let tx = proposal
            .transaction
            .as_ref()
            .context("legacy creation remains uncertain; no new creation will be signed")?;
        if settlement::can_replace(&rpc, tx)? {
            let args = self.create_args(order)?;
            let pczt = std::fs::read(&args.pczt)?;
            ensure!(
                hex::encode(Sha256::digest(&pczt)) == proposal.digest,
                "saved proposal changed"
            );
            // Never create after the payment's expiry, even if the old Solana transaction expired.
            let relay = crate::solana::read_relay(&mut rpc, &self.config.relay)?;
            if relay.view()?.tip_height() >= proposal.expiry_height {
                self.update(id, |o| o.status = OrderStatus::Expired)?;
                bail!("the payment expired before its Escrow was created");
            }
            let prepared = prepare_escrow(&args, crate::parse_salt(&proposal.salt)?)?;
            ensure!(
                prepared.escrow == escrow,
                "replacement changed the Escrow identity"
            );
            proposal.transaction = Some(prepared.transaction);
            self.update(id, |o| o.proposal = Some(proposal.clone()))?;
        }
        let signature = rpc
            .submit_prepared(
                proposal
                    .transaction
                    .as_ref()
                    .context("no saved transaction")?,
            )
            .map_err(crate::solana::tx_error)?;
        self.update(id, |o| o.status = OrderStatus::Escrowed)?;
        Ok(self.answer(&proposal, signature))
    }

    /// Pending proposals survive the intake TTL and are reconciled before any refund decision.
    pub fn sweep(&self) {
        let _operation = self.operations.lock().expect("operations lock");
        let orders: Vec<Order> = self
            .orders
            .lock()
            .expect("orders lock")
            .values()
            .cloned()
            .collect();
        for mut order in orders {
            let id = order.terms.order_id.clone();
            if order.legacy_unresolved || (order.proposal.is_none() && order.escrow.is_some()) {
                if let Err(e) = self.reconcile_known_legacy(&order) {
                    println!("order {id}: legacy reconciliation unresolved: {e:#}");
                }
                continue;
            }
            if order.proposal.is_none()
                && self.order_dir().join(&id).with_extension("salt").exists()
            {
                match self.recover_legacy(&order) {
                    Ok(recovered) => order = recovered,
                    Err(e) => {
                        println!("order {id}: legacy recovery unresolved: {e:#}");
                        continue;
                    }
                }
            }
            if order
                .proposal
                .as_ref()
                .is_some_and(|p| p.transaction.is_none())
                && matches!(order.status, OrderStatus::Open | OrderStatus::Escrowed)
            {
                if let Err(e) = self.reconcile_known_legacy(&order) {
                    println!(
                        "order {}: legacy reconciliation: {e:#}",
                        order.terms.order_id
                    );
                }
            } else if order.proposal.is_some()
                && matches!(order.status, OrderStatus::Open | OrderStatus::Escrowed)
            {
                match self.reconcile(&order) {
                    Ok(_) => {
                        if let Some(escrow) =
                            order.escrow.as_deref().and_then(|s| parse_address(s).ok())
                        {
                            self.try_refund(&order.terms.order_id, escrow);
                        }
                    }
                    Err(e) => println!("order {}: reconciliation: {e:#}", order.terms.order_id),
                }
            } else if order.status == OrderStatus::Open
                && now()
                    >= order
                        .created_at
                        .saturating_add(self.config.order_ttl.as_secs())
            {
                let _ = self.update(&order.terms.order_id, |o| o.status = OrderStatus::Expired);
            }
        }
    }

    // Even when old proposal files are gone, a recorded address is enough to reconcile a
    // settlement or run the program's authorized refund. It never permits creating another Escrow.
    fn reconcile_known_legacy(&self, order: &Order) -> Result<()> {
        let escrow = parse_address(
            order
                .escrow
                .as_deref()
                .context("legacy Escrow address missing")?,
        )?;
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        if let Some((owner, data)) =
            zolar_relayer::chain::Cluster::account_with_owner(&mut rpc, &escrow)
                .map_err(anyhow::Error::msg)?
        {
            ensure!(owner == ESCROW_ID, "legacy Escrow has another owner");
            let account = crate::solana::read_escrow(&data)?;
            ensure!(
                account.payout == parse_address(&order.terms.payout)?
                    && account.mint == parse_address(&order.terms.item.mint)?
                    && account.amount == order.terms.item.amount
                    && account.relay == self.config.relay,
                "legacy Escrow differs from its saved terms"
            );
            self.try_refund(&order.terms.order_id, escrow);
        } else if let Some((event, _)) = settlement::history(&rpc, &escrow)?.settled {
            self.update(&order.terms.order_id, |o| {
                o.status = if event == Event::Claimed {
                    OrderStatus::Claimed
                } else {
                    OrderStatus::Refunded
                };
                o.legacy_unresolved = false;
            })?;
        }
        Ok(())
    }

    fn try_refund(&self, id: &str, escrow: Address) {
        let result = refund(&RefundArgs {
            rpc_url: self.config.rpc_url.clone(),
            keypair: self.config.keypair.clone(),
            escrow,
            wait: false,
        });
        match result {
            Ok(done) if done.refund_signature.is_some() => {
                println!("order {id}: refund submitted; waiting for finalized settlement");
            }
            Ok(_) => {}
            // Not refundable yet: the payment may still arrive, or the claim window is open.
            Err(e) if e.to_string().contains("not yet") => {}
            Err(e) => println!("order {id}: refund step failed: {e}"),
        }
    }

    pub fn handle(&self, mut request: Request) {
        let origin = checkout::header(&request, "Origin").map(str::to_string);
        let cors = checkout::allowed_origin(origin.as_deref(), &self.config.allowed_origins)
            .map(str::to_string);
        if origin.is_some() && cors.is_none() {
            let _ = request.respond(checkout::error(403, "origin not allowed"));
            return;
        }
        let reply = self.route(&mut request);
        let _ = request.respond(checkout::with_cors(reply, cors.as_deref()));
    }

    fn route(&self, request: &mut Request) -> Reply {
        let method = request.method().clone();
        let url = request.url().split('?').next().unwrap_or("").to_string();
        let parts: Vec<&str> = url.trim_matches('/').split('/').collect();
        let result: Result<Reply> = match (&method, parts.as_slice()) {
            (Method::Options, _) => Ok(checkout::json(204, &serde_json::json!({}))),
            (Method::Get, ["items"]) => Ok(checkout::json(200, &self.items())),
            (Method::Post, ["orders"]) => checkout::read_json(request, 4096)
                .and_then(|r| self.create_order(r))
                .map(|t| checkout::json(201, &t)),
            (Method::Get, ["orders", id]) => self
                .get(id)
                .map(|o| checkout::json(200, &o.view()))
                .context("no such order"),
            (Method::Get, ["orders", id, "terms"]) => self
                .get(id)
                .map(|o| checkout::json(200, &o.terms))
                .context("no such order"),
            (Method::Post, ["orders", id, "escrow"]) => checkout::read_json(request, 2_000_000)
                .and_then(|p| self.propose(id, p))
                .map(|c| checkout::json(201, &c)),
            _ => Err(anyhow::anyhow!("not found")),
        };
        result.unwrap_or_else(|e| {
            let status = if e.to_string() == "not found" || e.to_string() == "no such order" {
                404
            } else {
                400
            };
            checkout::error(status, format!("{e:#}"))
        })
    }
}

/// Serves until the process ends: `threads` request handlers and the sweep every `sweep_every`.
pub fn serve(
    merchant: Arc<Merchant>,
    bind: &str,
    threads: usize,
    sweep_every: Duration,
) -> Result<()> {
    let server = Arc::new(Server::http(bind).map_err(|e| anyhow::anyhow!("binding {bind}: {e}"))?);
    println!("zolar-merchant listening on http://{bind}");
    let worker = merchant.clone();
    std::thread::spawn(move || {
        loop {
            worker.sweep();
            std::thread::sleep(sweep_every);
        }
    });
    let handlers: Vec<_> = (0..threads.max(1))
        .map(|_| {
            let server = server.clone();
            let merchant = merchant.clone();
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    merchant.handle(request);
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

    fn merchant(dir: &std::path::Path) -> Arc<Merchant> {
        Merchant::new(Config {
            name: "Shielded Night".into(),
            network: Network::Testnet,
            rpc_url: "http://127.0.0.1:1".into(),
            relay: Address::new_from_array([4; 32]),
            keypair: dir.join("missing.json"),
            ufvk_file: dir.join("missing.ufvk"),
            zcash_address: "utest1merchant".into(),
            items: vec![Item {
                id: "ga".into(),
                name: "General admission".into(),
                mint: Address::new_from_array([3; 32]).to_string(),
                amount: 1,
                price_zats: 100_000,
            }],
            data_dir: dir.to_path_buf(),
            allowed_origins: vec!["http://localhost:5175".into()],
            max_open_orders: 3,
            max_orders_per_payout: 1,
            order_ttl: Duration::from_secs(600),
        })
        .unwrap()
    }

    fn tempdir() -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("zolar-merchant-test-{}", checkout::random_id(8)));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn wallet(n: u8) -> String {
        Address::new_from_array([n; 32]).to_string()
    }

    #[test]
    fn an_order_carries_a_fresh_memo_and_the_merchants_terms() {
        let dir = tempdir();
        let m = merchant(&dir);
        let a = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(1),
            })
            .unwrap();
        let b = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(2),
            })
            .unwrap();
        assert_ne!(a.memo, b.memo);
        assert_ne!(a.order_id, b.order_id);
        assert_eq!(a.zcash_address, "utest1merchant");
        assert_eq!(a.item.price_zats, 100_000);
        assert_eq!(a.network, "testnet");
        assert!(
            m.create_order(CreateOrder {
                item_id: "vip".into(),
                payout: wallet(3)
            })
            .is_err()
        );
        assert!(
            m.create_order(CreateOrder {
                item_id: "ga".into(),
                payout: "nope".into()
            })
            .is_err()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_orders_are_limited_overall_and_per_wallet() {
        let dir = tempdir();
        let m = merchant(&dir);
        m.create_order(CreateOrder {
            item_id: "ga".into(),
            payout: wallet(1),
        })
        .unwrap();
        let again = m.create_order(CreateOrder {
            item_id: "ga".into(),
            payout: wallet(1),
        });
        assert!(
            again
                .unwrap_err()
                .to_string()
                .contains("already has a checkout")
        );
        m.create_order(CreateOrder {
            item_id: "ga".into(),
            payout: wallet(2),
        })
        .unwrap();
        m.create_order(CreateOrder {
            item_id: "ga".into(),
            payout: wallet(3),
        })
        .unwrap();
        let full = m.create_order(CreateOrder {
            item_id: "ga".into(),
            payout: wallet(4),
        });
        assert!(full.unwrap_err().to_string().contains("too many"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn orders_survive_a_restart_and_a_lapsed_order_takes_no_payment() {
        let dir = tempdir();
        let m = merchant(&dir);
        let t = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(1),
            })
            .unwrap();
        drop(m);
        let m = merchant(&dir);
        assert_eq!(m.get(&t.order_id).unwrap().terms, t);
        m.update(&t.order_id, |o| o.created_at = 0).unwrap();
        let err = m
            .propose(
                &t.order_id,
                ProposePayment {
                    pczt: "AAAA".into(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("lapsed"), "{err}");
        m.sweep();
        assert_eq!(m.get(&t.order_id).unwrap().status, OrderStatus::Expired);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_closed_order_takes_no_payment() {
        let dir = tempdir();
        let m = merchant(&dir);
        let t = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(1),
            })
            .unwrap();
        m.update(&t.order_id, |o| o.status = OrderStatus::Claimed)
            .unwrap();
        let err = m
            .propose(
                &t.order_id,
                ProposePayment {
                    pczt: "AAAA".into(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("not open"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retrying_a_legacy_proposal_never_deletes_its_salt() {
        let dir = tempdir();
        let m = merchant(&dir);
        let order = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(1),
            })
            .unwrap();
        let pczt = include_bytes!("../../crates/zolar-pczt/tests/fixtures/payment-unsigned.pczt");
        let path = dir.join("orders").join(format!("{}.salt", order.order_id));
        let salt = "00".repeat(32);
        std::fs::write(&path, &salt).unwrap();
        std::fs::write(path.with_extension("pczt"), pczt).unwrap();
        let result = m.propose(
            &order.order_id,
            ProposePayment {
                pczt: base64::engine::general_purpose::STANDARD.encode(pczt),
            },
        );
        assert!(result.is_err(), "no real keys or network in this fixture");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            salt,
            "a prior send may have landed"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_lost_creation_response_and_restart_keep_one_escrow_and_reconcile_a_refund() {
        use crate::test_support as fixture;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempdir();
        let key = solana_keypair::Keypair::new();
        let payee = key.pubkey();
        std::fs::write(
            dir.join("payee.json"),
            serde_json::to_vec(&key.to_bytes().to_vec()).unwrap(),
        )
        .unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../crates/zolar-pczt/tests/fixtures/payee.ufvk"
            ),
            dir.join("payee.ufvk"),
        )
        .unwrap();
        let pczt = include_bytes!("../../crates/zolar-pczt/tests/fixtures/payment-unsigned.pczt");
        let summary = zolar_pczt::summary(pczt).unwrap();
        let relay = Address::new_from_array([4; 32]);
        let mint = Address::new_from_array([3; 32]);
        let payout = Address::new_from_array([1; 32]);
        let saved_path = Arc::new(Mutex::new(PathBuf::new()));
        let record = saved_path.clone();
        let chain = Arc::new(Mutex::new((
            None::<(Address, Vec<u8>)>,
            None::<String>,
            false,
        )));
        let world = chain.clone();
        let sends = Arc::new(AtomicUsize::new(0));
        let sent = sends.clone();
        let rpc = fixture::Rpc::new(move |request| {
            let mut state = world.lock().unwrap();
            let address = request["params"][0].as_str().unwrap_or("");
            match request["method"].as_str().unwrap() {
                "getLatestBlockhash" => Ok(
                    serde_json::json!({"context":{"slot":10},"value":{"blockhash":bs58::encode([7;32]).into_string(),"lastValidBlockHeight":1000}}),
                ),
                "getBlockHeight" => Ok(serde_json::json!(100)),
                "getAccountInfo" if address == relay.to_string() => Ok(fixture::account(
                    Address::new_from_array(header_relay_interface::HEADER_RELAY_PROGRAM),
                    &fixture::relay(summary.expiry_height - 18),
                )),
                "getAccountInfo" => {
                    if let Some((escrow, data)) = &state.0
                        && address == escrow.to_string()
                    {
                        return Ok(if state.2 {
                            serde_json::json!({"value":null})
                        } else {
                            fixture::account(ESCROW_ID, data)
                        });
                    }
                    // Before creation the derived Escrow is absent; all other reads are inventory.
                    if !record.lock().unwrap().as_os_str().is_empty() {
                        let saved: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(&*record.lock().unwrap()).unwrap(),
                        )
                        .unwrap();
                        if saved["proposal"]["escrow"].as_str() == Some(address) {
                            return Ok(serde_json::json!({"value":null}));
                        }
                    }
                    Ok(fixture::account(
                        crate::solana::TOKEN_PROGRAM,
                        &fixture::token(payee, mint, 100),
                    ))
                }
                "getSignaturesForAddress" => {
                    if state.2 {
                        Ok(serde_json::json!([
                            {"signature":"refund","err":null},{"signature":state.1.as_ref().unwrap(),"err":null}
                        ]))
                    } else if state.0.is_some() {
                        Err(
                            serde_json::json!({"code":-32004,"message":"history unavailable while Escrow is funded"}),
                        )
                    } else {
                        Ok(serde_json::json!([]))
                    }
                }
                "getTransaction" => {
                    let escrow = state.0.as_ref().unwrap().0;
                    Ok(fixture::transaction(
                        address,
                        escrow,
                        if address == "refund" { 3 } else { 0 },
                    ))
                }
                "sendTransaction" => {
                    sent.fetch_add(1, Ordering::SeqCst);
                    // This is read before simulating a landed transaction: identity and wire must
                    // already be durable when the first side effect starts.
                    let saved: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&*record.lock().unwrap()).unwrap())
                            .unwrap();
                    let proposal = &saved["proposal"];
                    assert_eq!(proposal["transaction"]["wire"], request["params"][0]);
                    let salt = crate::parse_salt(proposal["salt"].as_str().unwrap()).unwrap();
                    let (escrow, data) = fixture::escrow(
                        payee,
                        payout,
                        mint,
                        relay,
                        &summary.txid,
                        salt,
                        summary.expiry_height,
                    );
                    assert_eq!(proposal["escrow"], escrow.to_string());
                    state.0 = Some((escrow, data));
                    state.1 = Some(
                        proposal["transaction"]["signature"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                    );
                    Err(
                        serde_json::json!({"code":-32000,"message":"response lost after accepting transaction"}),
                    )
                }
                other => panic!("unexpected RPC {other}"),
            }
        });
        let config = Config {
            name: "test".into(),
            network: Network::Testnet,
            rpc_url: rpc.url.clone(),
            relay,
            keypair: dir.join("payee.json"),
            ufvk_file: dir.join("payee.ufvk"),
            zcash_address: "utest1merchant".into(),
            items: vec![Item {
                id: "ga".into(),
                name: "ticket".into(),
                mint: mint.to_string(),
                amount: 1,
                price_zats: 2_500_000,
            }],
            data_dir: dir.clone(),
            allowed_origins: vec![],
            max_open_orders: 10,
            max_orders_per_payout: 1,
            order_ttl: Duration::from_secs(600),
        };
        let m = Merchant::new(config.clone()).unwrap();
        let order = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: payout.to_string(),
            })
            .unwrap();
        m.update(&order.order_id, |o| o.terms.memo = "zolar t02 spike".into())
            .unwrap();
        *saved_path.lock().unwrap() = dir.join("orders").join(format!("{}.json", order.order_id));
        let proposal = || ProposePayment {
            pczt: base64::engine::general_purpose::STANDARD.encode(pczt),
        };
        assert!(m.propose(&order.order_id, proposal()).is_err());
        let saved = m.get(&order.order_id).unwrap().proposal.unwrap();
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        drop(m);
        let m = Merchant::new(config).unwrap();
        m.update(&order.order_id, |o| o.created_at = 0).unwrap();
        let recovered = m.propose(&order.order_id, proposal()).unwrap();
        assert_eq!(recovered.escrow, saved.escrow);
        assert_eq!(recovered.salt, saved.salt);
        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "an identical retry must not create another Escrow"
        );
        assert!(
            m.propose(
                &order.order_id,
                ProposePayment {
                    pczt: base64::engine::general_purpose::STANDARD.encode(b"different")
                }
            )
            .unwrap_err()
            .to_string()
            .contains("another payment")
        );
        chain.lock().unwrap().2 = true;
        m.sweep();
        assert_eq!(
            m.get(&order.order_id).unwrap().status,
            OrderStatus::Refunded,
            "closed alone must not be called Claimed"
        );
        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "a closed creation cannot be recreated"
        );
        drop(m);
        drop(rpc);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_expired_legacy_orphan_is_reserved_and_refund_checked_without_history() {
        use crate::test_support as fixture;
        let dir = tempdir();
        let original = merchant(&dir);
        let mut configuration = original.config.clone();
        drop(original);
        let key = solana_keypair::Keypair::new();
        let payee = key.pubkey();
        let key_path = dir.join("payee.json");
        std::fs::write(
            &key_path,
            serde_json::to_vec(&key.to_bytes().to_vec()).unwrap(),
        )
        .unwrap();
        let pczt = include_bytes!("../../crates/zolar-pczt/tests/fixtures/payment-unsigned.pczt");
        let summary = zolar_pczt::summary(pczt).unwrap();
        let relay = configuration.relay;
        let (address, data) = fixture::escrow(
            payee,
            Address::new_from_array([1; 32]),
            Address::new_from_array([3; 32]),
            relay,
            &summary.txid,
            [0; 32],
            summary.expiry_height,
        );
        let mut account = Escrow::from_bytes(&data).unwrap();
        account.created_at = now() as i64;
        let data = borsh::to_vec(&account).unwrap();
        let rpc = fixture::Rpc::new(move |request| {
            if request["method"] == "getAccountInfo" {
                if request["params"][0] == relay.to_string() {
                    return Ok(fixture::account(
                        Address::new_from_array(header_relay_interface::HEADER_RELAY_PROGRAM),
                        &fixture::relay(summary.expiry_height - 18),
                    ));
                }
                assert_eq!(request["params"][0], address.to_string());
                return Ok(fixture::account(ESCROW_ID, &data));
            }
            Err(serde_json::json!({"code":-32004,"message":"history pruned"}))
        });
        configuration.rpc_url = rpc.url.clone();
        configuration.keypair = key_path;
        let m = Merchant::new(configuration.clone()).unwrap();
        let order = m
            .create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(1),
            })
            .unwrap();
        m.update(&order.order_id, |o| {
            o.recovery_version = 0;
            o.status = OrderStatus::Expired;
            o.created_at = 0;
        })
        .unwrap();
        let base = dir.join("orders").join(&order.order_id);
        std::fs::write(base.with_extension("pczt"), pczt).unwrap();
        std::fs::write(base.with_extension("salt"), "00".repeat(32)).unwrap();
        drop(m);
        let m = Merchant::new(configuration).unwrap();
        m.sweep();
        let recovered = m.get(&order.order_id).unwrap();
        assert_eq!(recovered.status, OrderStatus::Open);
        assert_eq!(recovered.escrow, Some(address.to_string()));
        assert!(recovered.proposal.is_some());
        assert!(
            m.create_order(CreateOrder {
                item_id: "ga".into(),
                payout: wallet(1)
            })
            .is_err(),
            "the uncertain inventory stays reserved"
        );
        let requests = rpc.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|r| r["method"] == "getAccountInfo" && r["params"][0] == relay.to_string()),
            "the authorized refund path was reached"
        );
        assert!(
            !requests.iter().any(|r| r["method"] != "getAccountInfo"),
            "history cannot block refund checks on readable state"
        );
        drop(requests);
        drop(m);
        drop(rpc);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
