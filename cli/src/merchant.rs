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
use serde::{Deserialize, Serialize};
use solana_address::Address;
use solana_signer::Signer;
use tiny_http::{Method, Request, Server};
use zolar_relayer::{chain::Cluster, rpc::HttpRpc};

use crate::{
    Network,
    checkout::{
        self, CreateOrder, EscrowCreated, Item, OrderStatus, OrderTerms, OrderView, ProposePayment,
        Reply,
    },
    payee::{CreateEscrow, RefundArgs, create_escrow, refund},
    solana::{associated_token_account, load_keypair, parse_address, parse_token_account},
};

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
struct Order {
    terms: OrderTerms,
    status: OrderStatus,
    escrow: Option<String>,
    created_at: u64,
    /// A payment is being checked and the Escrow created; a second proposal must wait.
    #[serde(default)]
    proposing: bool,
}

impl Order {
    fn holds_inventory(&self) -> bool {
        matches!(self.status, OrderStatus::Open | OrderStatus::Escrowed)
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
    orders: Mutex<HashMap<String, Order>>,
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
        std::fs::create_dir_all(config.data_dir.join("orders"))?;
        let mut orders = HashMap::new();
        for entry in std::fs::read_dir(config.data_dir.join("orders"))?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let mut order: Order = serde_json::from_slice(&std::fs::read(&path)?)
                    .with_context(|| format!("reading {}", path.display()))?;
                // A proposal cut short by a restart never finished; the customer may propose again.
                order.proposing = false;
                orders.insert(order.terms.order_id.clone(), order);
            }
        }
        Ok(Arc::new(Self {
            config,
            orders: Mutex::new(orders),
        }))
    }

    fn order_dir(&self) -> PathBuf {
        self.config.data_dir.join("orders")
    }

    fn save(&self, order: &Order) -> Result<()> {
        let path = self
            .order_dir()
            .join(format!("{}.json", order.terms.order_id));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(order)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Order)) -> Result<Order> {
        let mut orders = self.orders.lock().expect("orders lock");
        let order = orders.get_mut(id).context("no such order")?;
        f(order);
        let copy = order.clone();
        drop(orders);
        self.save(&copy)?;
        Ok(copy)
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
        };
        orders.insert(order_id, order.clone());
        drop(orders);
        self.save(&order)?;
        Ok(terms)
    }

    fn get(&self, id: &str) -> Option<Order> {
        self.orders.lock().expect("orders lock").get(id).cloned()
    }

    /// Checks the customer's unsigned payment against the order with the viewing key and locks the
    /// item in a private-only Escrow paying the order's payout wallet.
    pub fn propose(&self, id: &str, proposal: ProposePayment) -> Result<EscrowCreated> {
        let pczt = base64::engine::general_purpose::STANDARD
            .decode(proposal.pczt.trim())
            .context("pczt is not base64")?;
        ensure!(pczt.len() < 1_000_000, "pczt too large");
        let order = {
            let mut orders = self.orders.lock().expect("orders lock");
            let order = orders.get_mut(id).context("no such order")?;
            ensure!(
                order.status == OrderStatus::Open,
                "the order is {:?}, not open",
                order.status
            );
            ensure!(
                now() < order.created_at + self.config.order_ttl.as_secs(),
                "the order lapsed; start a new checkout"
            );
            ensure!(
                !order.proposing,
                "a payment for this order is being checked"
            );
            order.proposing = true;
            order.clone()
        };
        let result = self.lock_item(&order, &pczt);
        let escrow = result.as_ref().ok().map(|c| c.escrow.clone());
        self.update(id, |o| {
            o.proposing = false;
            if let Some(escrow) = escrow {
                o.status = OrderStatus::Escrowed;
                o.escrow = Some(escrow);
            }
        })?;
        result
    }

    fn lock_item(&self, order: &Order, pczt: &[u8]) -> Result<EscrowCreated> {
        let id = &order.terms.order_id;
        let pczt_path = self.order_dir().join(format!("{id}.pczt"));
        std::fs::write(&pczt_path, pczt)?;
        let salt_path = self.order_dir().join(format!("{id}.salt"));
        // A salt file left by a failed attempt would block create_escrow; its Escrow never existed
        // unless the transaction landed, in which case the order is no longer open on retry.
        let _ = std::fs::remove_file(&salt_path);
        let created = create_escrow(&CreateEscrow {
            network: self.config.network,
            rpc_url: self.config.rpc_url.clone(),
            keypair: self.config.keypair.clone(),
            relay: self.config.relay,
            pczt: pczt_path,
            ufvk_file: self.config.ufvk_file.clone(),
            expect_memo: order.terms.memo.clone(),
            min_zats: order.terms.item.price_zats,
            payout: parse_address(&order.terms.payout)?,
            mint: parse_address(&order.terms.item.mint)?,
            amount: order.terms.item.amount,
            salt_out: salt_path.clone(),
            bond: 0,
            private_only: true,
        })?;
        let salt = std::fs::read_to_string(&salt_path)?.trim().to_string();
        Ok(EscrowCreated {
            escrow: created.escrow.to_string(),
            salt,
            expiry_height: created.expiry_height,
            signature: created.signature,
        })
    }

    /// One pass of the worker: lapse stale open orders, mark claimed Escrows, take back the items of
    /// Escrows whose payment can no longer arrive.
    pub fn sweep(&self) {
        let orders: Vec<Order> = self
            .orders
            .lock()
            .expect("orders lock")
            .values()
            .cloned()
            .collect();
        let mut rpc = HttpRpc::new(&self.config.rpc_url);
        for order in orders {
            let id = order.terms.order_id.clone();
            match order.status {
                OrderStatus::Open
                    if !order.proposing
                        && now() >= order.created_at + self.config.order_ttl.as_secs() =>
                {
                    let _ = self.update(&id, |o| o.status = OrderStatus::Expired);
                }
                OrderStatus::Escrowed => {
                    let Some(escrow) = order.escrow.as_deref().and_then(|e| parse_address(e).ok())
                    else {
                        continue;
                    };
                    match rpc.account(&escrow) {
                        // Only this merchant can refund, and a refund is recorded when it is sent:
                        // an Escrow that closed otherwise was claimed.
                        Ok(None) => {
                            let _ = self.update(&id, |o| o.status = OrderStatus::Claimed);
                            println!("order {id}: claimed");
                        }
                        Ok(Some(_)) => self.try_refund(&id, escrow),
                        Err(e) => println!("order {id}: cannot read the Escrow: {e}"),
                    }
                }
                _ => {}
            }
        }
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
                let _ = self.update(id, |o| o.status = OrderStatus::Refunded);
                println!("order {id}: item taken back");
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
}
