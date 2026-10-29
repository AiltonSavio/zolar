//! The checkout protocol shared by `zolar-merchant` (the Payee's service) and `zolar-wallet` (the
//! Payer's local helper), and the HTTP plumbing both use.
//!
//! A shop page creates an order at the merchant, then asks the customer's wallet to pay it. The wallet
//! fetches the order's terms from the merchant itself, asks the customer to approve them on its own
//! page, and from then on talks to the merchant directly: the page never sees the transaction ID or
//! the salt.

use std::io::Read;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Request, Response};

pub(crate) fn private_dir(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    Ok(())
}

/// Each persisted service directory has exactly one writer, including across processes.
pub(crate) struct DataDirLock(std::fs::File);

impl Drop for DataDirLock {
    fn drop(&mut self) {
        // Unlock explicitly: a concurrent fork can briefly inherit the open file description
        // before exec closes it, so closing only our descriptor need not release the lock yet.
        let _ = self.0.unlock();
    }
}

pub(crate) fn lock_data_dir(path: &std::path::Path) -> Result<DataDirLock> {
    use std::os::unix::fs::OpenOptionsExt;
    private_dir(path)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path.join(".lock"))?;
    file.try_lock()
        .context("service data directory is already in use")?;
    Ok(DataDirLock(file))
}

/// Atomically replaces a private state file. Callers serialize this with their state lock and
/// publish the new in-memory value only after this succeeds, before performing external effects.
pub(crate) fn save_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let tmp = path.with_extension(format!("tmp-{}", random_id(8)));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

/// What the merchant sells. `amount` is in the mint's smallest unit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub mint: String,
    pub amount: u64,
    pub price_zats: u64,
}

/// An order as the merchant offers it. Everything the customer approves is in here.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderTerms {
    pub order_id: String,
    pub merchant: String,
    pub item: Item,
    /// The merchant's Zcash unified address.
    pub zcash_address: String,
    /// The challenge the payment's memo must carry; new for every order.
    pub memo: String,
    /// The Solana wallet the item goes to.
    pub payout: String,
    /// The Header Relay the Escrow will trust.
    pub relay: String,
    /// `testnet` or `mainnet` (Zcash).
    pub network: String,
}

impl OrderTerms {
    /// A digest of the terms, so an approval covers exactly these and nothing else.
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("terms serialize");
        hex::encode(Sha256::digest(bytes))
    }
}

/// Where an order stands, as the merchant reports it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    /// Created, no payment proposed yet.
    Open,
    /// The item is locked in an Escrow for the customer.
    Escrowed,
    /// The customer claimed the item.
    Claimed,
    /// The merchant took the item back after the payment did not arrive in time.
    Refunded,
    /// Never paid and no Escrow made before the order lapsed.
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderView {
    pub order_id: String,
    pub status: OrderStatus,
    pub escrow: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateOrder {
    pub item_id: String,
    pub payout: String,
}

/// Sent by the wallet: the unsigned payment for the order.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProposePayment {
    /// The PCZT, base64.
    pub pczt: String,
}

/// The merchant's answer to a proposed payment: the Escrow it created and the salt of its Payment
/// Commitment. Only the wallet receives this.
#[derive(Debug, Serialize, Deserialize)]
pub struct EscrowCreated {
    pub escrow: String,
    pub salt: String,
    pub expiry_height: u32,
    pub signature: String,
}

/// Stages of a checkout in the wallet, in order. The page shows these.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "stage")]
pub enum Stage {
    AwaitingApproval,
    Rejected,
    /// Payment identity is saved; the wallet is reconciling or retrying it without paying again.
    Recovering,
    BuildingPayment,
    LockingItem,
    CheckingEscrow {
        escrow: String,
    },
    Paying {
        escrow: String,
    },
    /// `sent_tip` is the relay tip when the payment went out: it is mined after it and final
    /// `finality_depth` blocks later.
    WaitingForFinality {
        escrow: String,
        tip: u32,
        final_height: u32,
        expiry: u32,
        sent_tip: u32,
        finality_depth: u16,
    },
    Proving {
        escrow: String,
    },
    Claimed {
        escrow: String,
        claim: String,
    },
    Done {
        escrow: String,
        claim: String,
        txid_found_on_solana: Option<bool>,
    },
    Failed {
        reason: String,
    },
}

impl Stage {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Done { .. } | Self::Failed { .. }
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartCheckout {
    pub merchant_url: String,
    pub order_id: String,
    #[serde(default)]
    pub zcash_address: Option<String>,
    #[serde(default)]
    pub zcash_account: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckoutStarted {
    pub job: String,
    /// Proves to the wallet that a status request comes from the page that started the job.
    pub token: String,
    /// The wallet's own approval page for this job; the page opens it in a window.
    pub approve_url: String,
}

/// A random hex string of `bytes` bytes, for ids, tokens and secret paths.
pub fn random_id(bytes: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// Refuses a request whose `Host` is not this loopback service: a page on another site that rebinds
/// its DNS name to 127.0.0.1 still sends its own name as `Host`.
pub fn check_loopback_host(host: Option<&str>, port: u16) -> Result<()> {
    let allowed = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    match host {
        Some(h) if allowed.iter().any(|a| a.eq_ignore_ascii_case(h)) => Ok(()),
        other => bail!(
            "refused Host {other:?}: only {} is served",
            allowed.join(" or ")
        ),
    }
}

/// The exact allowed origin the request comes from, if any. A request without `Origin` (a direct
/// navigation or a native client) has none.
pub fn allowed_origin<'a>(origin: Option<&str>, allowed: &'a [String]) -> Option<&'a str> {
    let origin = origin?;
    allowed
        .iter()
        .find(|a| a.as_str() == origin)
        .map(String::as_str)
}

pub fn read_json<T: serde::de::DeserializeOwned>(request: &mut Request, limit: usize) -> Result<T> {
    let mut body = Vec::new();
    request
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)?;
    if body.len() > limit {
        bail!("request body over {limit} bytes");
    }
    serde_json::from_slice(&body).context("request body is not the expected JSON")
}

pub type Reply = Response<std::io::Cursor<Vec<u8>>>;

fn h(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

pub fn json<T: Serialize>(status: u16, value: &T) -> Reply {
    let body = serde_json::to_vec(value).expect("reply serializes");
    Response::from_data(body)
        .with_status_code(status)
        .with_header(h("Content-Type", "application/json"))
        .with_header(h("Cache-Control", "no-store"))
}

pub fn error(status: u16, message: impl std::fmt::Display) -> Reply {
    json(status, &serde_json::json!({ "error": message.to_string() }))
}

/// A page of this service itself: no other site may frame it.
pub fn html(body: String) -> Reply {
    Response::from_data(body.into_bytes())
        .with_header(h("Content-Type", "text/html; charset=utf-8"))
        .with_header(h("Cache-Control", "no-store"))
        .with_header(h("X-Frame-Options", "DENY"))
        .with_header(h(
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; connect-src 'self'; frame-ancestors 'none'; form-action 'self'",
        ))
        .with_header(h("Referrer-Policy", "no-referrer"))
}

/// Adds the CORS headers for an allowed origin (never `*`).
pub fn with_cors(reply: Reply, origin: Option<&str>) -> Reply {
    match origin {
        Some(o) => reply
            .with_header(h("Access-Control-Allow-Origin", o))
            .with_header(h("Vary", "Origin"))
            .with_header(h("Access-Control-Allow-Methods", "GET, POST, OPTIONS"))
            .with_header(h(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization",
            ))
            .with_header(h("Access-Control-Max-Age", "600")),
        None => reply,
    }
}

/// Minimal HTML escaping for values shown on the wallet's approval page.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms() -> OrderTerms {
        OrderTerms {
            order_id: "o1".into(),
            merchant: "Shielded Night".into(),
            item: Item {
                id: "ga".into(),
                name: "General admission".into(),
                mint: "Mint111".into(),
                amount: 1,
                price_zats: 100_000,
            },
            zcash_address: "utest1abc".into(),
            memo: "zolar order o1 1234".into(),
            payout: "Payout111".into(),
            relay: "Relay111".into(),
            network: "testnet".into(),
        }
    }

    #[test]
    fn the_digest_changes_with_any_term() {
        let t = terms();
        let mut other = t.clone();
        other.item.price_zats += 1;
        assert_ne!(t.digest(), other.digest());
        let mut other = t.clone();
        other.payout = "Someone".into();
        assert_ne!(t.digest(), other.digest());
        assert_eq!(t.digest(), terms().digest());
    }

    #[test]
    fn only_the_loopback_host_on_this_port_is_served() {
        assert!(check_loopback_host(Some("127.0.0.1:7878"), 7878).is_ok());
        assert!(check_loopback_host(Some("localhost:7878"), 7878).is_ok());
        assert!(check_loopback_host(Some("evil.example:7878"), 7878).is_err());
        assert!(check_loopback_host(Some("127.0.0.1:7879"), 7878).is_err());
        assert!(check_loopback_host(Some("127.0.0.1"), 7878).is_err());
        assert!(check_loopback_host(None, 7878).is_err());
    }

    #[test]
    fn origins_match_exactly() {
        let allowed = vec!["http://localhost:5175".to_string()];
        assert_eq!(
            allowed_origin(Some("http://localhost:5175"), &allowed),
            Some("http://localhost:5175")
        );
        assert_eq!(
            allowed_origin(Some("http://localhost:5175.evil.example"), &allowed),
            None
        );
        assert_eq!(allowed_origin(Some("http://localhost"), &allowed), None);
        assert_eq!(allowed_origin(None, &allowed), None);
    }

    #[test]
    fn stages_serialize_with_a_tag() {
        let s = serde_json::to_value(Stage::WaitingForFinality {
            escrow: "E".into(),
            tip: 10,
            final_height: 5,
            expiry: 20,
            sent_tip: 8,
            finality_depth: 10,
        })
        .unwrap();
        assert_eq!(s["stage"], "waiting_for_finality");
        assert!(Stage::Failed { reason: "x".into() }.is_terminal());
        assert!(!Stage::Proving { escrow: "E".into() }.is_terminal());
    }

    #[test]
    fn approval_page_values_are_escaped() {
        assert_eq!(escape("<b>\"x\"&'"), "&lt;b&gt;&quot;x&quot;&amp;&#39;");
    }
}
