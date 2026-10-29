//! Adapter to the existing Zkool wallet's released GraphQL API (zkool-v6.35.0).
//! Zkool owns the spending keys. The connector retains the unchanged prepared package and asks
//! Zkool to sign only after approval and an independent check of the funded Solana Escrow.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use base64::Engine;
use bincode::Decode;
use serde::Serialize;
use serde_json::{Value, json};
use zcash_keys::address::UnifiedAddress;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BranchId, TEST_NETWORK};

use crate::{checkout, payer::Checked};

/// 120-block ordinary NU7 expiry, plus at most 24 blocks of Relay lag. No transaction rewriting.
pub const MAX_EXPIRY_AHEAD: u32 = 144;
pub const MAX_RELAY_LAG: u32 = 24;
const MAX_PACKAGE: usize = 8 * 1024 * 1024;
const MIN_REMAINING_BLOCKS: u32 = 6;
const BACKSTOP_MARGIN_SECS: i64 = 30 * 60;

/// bincode 2 standard, in the exact order of Zkool's api/pay.rs::PcztPackage.
/// Decode the whole package, reject trailing bytes, and pass the original bytes back to the wallet.
#[derive(Decode)]
#[cfg_attr(test, derive(bincode::Encode))]
struct Package {
    pczt: Vec<u8>,
    n_spends: [usize; 4],
    sapling_indices: Vec<usize>,
    orchard_indices: Vec<usize>,
    ironwood_indices: Vec<usize>,
    can_sign: bool,
    can_broadcast: bool,
    price: Option<f64>,
    category: Option<u32>,
    is_issuance: bool,
}

pub struct Zkool {
    endpoint: String,
    // Account-scoped delegated signing capability: never sent to the shop, persisted in a job,
    // logged, or placed in a URL. Zkool verifies the JWT's ES256 signature.
    token: String,
    account: u32,
}

#[derive(Clone, Serialize)]
pub struct Connection {
    pub name: &'static str,
    pub account: u32,
    pub address: String,
    pub network: &'static str,
    pub height: u32,
}

impl Zkool {
    pub fn new(endpoint: &str, token: String, account: u32) -> Result<Self> {
        let url = url::Url::parse(endpoint).context("invalid Zkool endpoint")?;
        ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1") | Some("[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/graphql",
            "Zkool must use a literal loopback HTTP /graphql endpoint without URL credentials"
        );
        ensure!(
            account > 0 && account <= i32::MAX as u32,
            "invalid Zkool account"
        );
        validate_token(&token, account, unix_time())?;
        Ok(Self {
            endpoint: url.to_string(),
            token,
            account,
        })
    }

    fn query(&self, query: &str, variables: Value) -> Result<Value> {
        validate_token(&self.token, self.account, unix_time())?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .proxy(None)
            .max_redirects(0)
            .build()
            .into();
        let mut response = agent
            .post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", self.token))
            .send_json(json!({ "query": query, "variables": variables }))
            .context("Zkool wallet request failed")?;
        ensure!(
            response.status() == 200,
            "Zkool returned a non-success response"
        );
        let reply: Value = response
            .body_mut()
            .read_json()
            .context("invalid Zkool response")?;
        ensure!(
            reply
                .get("errors")
                .is_none_or(|e| e.is_null() || e.as_array().is_some_and(Vec::is_empty)),
            "Zkool refused the request; check account permissions, sync and spendable balance in Zkool"
        );
        reply
            .get("data")
            .filter(|d| d.is_object())
            .cloned()
            .context("Zkool returned no data")
    }

    pub fn connection(&self) -> Result<Connection> {
        let mut connection = self.identity()?;
        connection.height = self.current_height()?;
        Ok(connection)
    }

    /// Discover the account without waiting for a chain backend response.
    pub fn identity(&self) -> Result<Connection> {
        let data = self.query(
            "query($account:Int!){apiVersion addressByAccount(idAccount:$account,pools:8){ironwood}}",
            json!({"account":self.account}),
        )?;
        ensure!(data["apiVersion"] == "1.0", "unsupported Zkool API version");
        let address = data["addressByAccount"]["ironwood"]
            .as_str()
            .context("Zkool shared no account address")?;
        ensure!(
            UnifiedAddress::decode(&TEST_NETWORK, address).is_ok(),
            "Zkool must be running on Zcash testnet (--coin 1)"
        );
        Ok(Connection {
            name: "Zkool",
            account: self.account,
            address: address.into(),
            network: "testnet",
            height: 0,
        })
    }

    pub fn current_height(&self) -> Result<u32> {
        let data = self.query("{currentHeight}", json!({}))?;
        height(&data["currentHeight"])
    }

    pub fn prepare(
        &self,
        address: &str,
        zats: u64,
        memo: &str,
        relay_tip: u32,
        work: &Path,
    ) -> Result<()> {
        let connection = self.connection()?;
        ensure!(
            connection.height >= relay_tip && connection.height - relay_tip <= MAX_RELAY_LAG,
            "the Header Relay must be within 24 blocks of the Zkool testnet tip"
        );
        // Zkool's broadcastTx does not lock notes. Sync before the next preparation, and the
        // connector admits one checkout at a time until the previous payment is settled.
        self.query(
            "mutation($account:Int!){synchronizeAccount(idAccount:$account,fast:false)}",
            json!({"account":self.account}),
        )?;
        let data = self.query(
            "query($account:Int!,$payment:Payment!){prepareSend(idAccount:$account,payment:$payment)}",
            json!({"account":self.account,"payment":{
                "recipients":[{"address":address,"amount":zec_amount(zats),"memo":memo}],
                "srcPools":14,"recipientPaysFee":false,"confirmations":1
            }}),
        )?;
        let encoded = data["prepareSend"]
            .as_str()
            .context("Zkool returned no prepared payment")?;
        ensure!(
            encoded.len() <= MAX_PACKAGE * 2,
            "Zkool package exceeds the size limit"
        );
        let bytes = hex::decode(encoded).context("Zkool returned a malformed package")?;
        let pczt = package_pczt(&bytes)?;
        let summary = zolar_pczt::summary(&pczt)?;
        let live_tip = self.current_height()?;
        check_heights(relay_tip, live_tip, summary.expiry_height)?;
        // Store before sharing anything with the merchant. The original package stays local.
        checkout::save_private(&work.join("prepared.zkool"), &bytes)?;
        checkout::save_private(&work.join("created.pczt"), &pczt)?;
        Ok(())
    }

    pub fn sign(&self, checked: &Checked, work: &Path, relay_tip: u32) -> Result<()> {
        self.check_ready(checked, relay_tip)?;
        let package = std::fs::read(work.join("prepared.zkool"))?;
        let pczt = package_pczt(&package)?;
        let summary = zolar_pczt::summary(&pczt)?;
        ensure!(
            summary.txid == checked.txid && summary.expiry_height == checked.expiry_height,
            "the prepared Zkool package differs from the checked Escrow"
        );
        let data = self.query(
            "query($account:Int!,$pczt:String!){signTx(idAccount:$account,pczt:$pczt)}",
            json!({"account":self.account,"pczt":hex::encode(package)}),
        )?;
        let encoded = data["signTx"]
            .as_str()
            .context("Zkool returned no signed transaction")?;
        ensure!(
            encoded.len() <= MAX_PACKAGE * 2,
            "signed transaction exceeds the size limit"
        );
        let raw = hex::decode(encoded).context("invalid signed Zcash transaction")?;
        verify_signed(&raw, &checked.txid, checked.expiry_height)?;
        // Persist signed bytes before any broadcast; an uncertain response never creates a payment.
        checkout::save_private(&work.join("signed.tx"), &raw)
    }

    pub fn check_ready(&self, checked: &Checked, relay_tip: u32) -> Result<()> {
        check_backstop(checked.escrow.created_at, unix_time())?;
        let live_tip = self.connection()?.height;
        check_heights(relay_tip, live_tip, checked.expiry_height)?;
        ensure!(
            checked.expiry_height >= live_tip.saturating_add(MIN_REMAINING_BLOCKS),
            "not enough blocks remain to send this Zkool payment safely"
        );
        Ok(())
    }

    pub fn broadcast(&self, checked: &Checked, work: &Path, relay_tip: u32) -> Result<()> {
        self.check_ready(checked, relay_tip)?;
        let raw = std::fs::read(work.join("signed.tx"))?;
        verify_signed(&raw, &checked.txid, checked.expiry_height)?;
        let data = self.query(
            "query($tx:String!){broadcastTx(txBytes:$tx)}",
            json!({"tx":hex::encode(raw)}),
        )?;
        let txid = data["broadcastTx"]
            .as_str()
            .context("Zkool did not acknowledge broadcast")?;
        ensure!(
            txid.eq_ignore_ascii_case(&checked.txid),
            "Zkool acknowledged a different payment"
        );
        Ok(())
    }
}

fn height(value: &Value) -> Result<u32> {
    value
        .as_u64()
        .filter(|h| *h > 0 && *h <= u32::MAX as u64)
        .map(|h| h as u32)
        .context("Zkool has no valid testnet height")
}

fn unix_time() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn validate_token(token: &str, account: u32, now: i64) -> Result<()> {
    let parts: Vec<_> = token.split('.').collect();
    ensure!(
        parts.len() == 3 && !parts[2].is_empty(),
        "Zkool requires an account-scoped JWT"
    );
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .context("invalid Zkool JWT")?,
    )?;
    ensure!(
        claims["sub"].as_u64() == Some(u64::from(account)) && claims["write"] == true,
        "use a write-enabled JWT scoped to this Zkool account; admin tokens are refused"
    );
    ensure!(
        claims["exp"]
            .as_i64()
            .is_some_and(|e| e > now.saturating_add(60)),
        "the Zkool connection token expired or expires too soon"
    );
    Ok(())
}

fn zec_amount(zats: u64) -> String {
    format!("{}.{:08}", zats / 100_000_000, zats % 100_000_000)
}

fn check_heights(relay_tip: u32, live_tip: u32, expiry: u32) -> Result<()> {
    ensure!(
        live_tip >= relay_tip && live_tip - relay_tip <= MAX_RELAY_LAG,
        "the Header Relay is too far behind the Zkool wallet"
    );
    ensure!(
        expiry >= live_tip.saturating_add(MIN_REMAINING_BLOCKS)
            && expiry <= relay_tip.saturating_add(MAX_EXPIRY_AHEAD),
        "the wallet payment expiry is outside the supported 144-block window"
    );
    Ok(())
}

fn check_backstop(created_at: i64, now: i64) -> Result<()> {
    ensure!(
        now >= created_at
            && now.saturating_add(BACKSTOP_MARGIN_SECS) < created_at.saturating_add(86_400),
        "the Escrow is too close to its refund backstop to authorize a payment"
    );
    Ok(())
}

fn package_pczt(bytes: &[u8]) -> Result<Vec<u8>> {
    let (p, consumed): (Package, _) = bincode::decode_from_slice(
        bytes,
        bincode::config::standard().with_limit::<MAX_PACKAGE>(),
    )
    .context("unsupported Zkool package format")?;
    ensure!(
        consumed == bytes.len()
            && p.can_sign
            && !p.can_broadcast
            && !p.is_issuance
            && p.n_spends[0] == 0,
        "Zkool must prepare an unsigned shielded ZEC payment"
    );
    // Consume all schema fields deliberately. The original signing metadata is retained unchanged.
    let _ = (
        p.sapling_indices,
        p.orchard_indices,
        p.ironwood_indices,
        p.price,
        p.category,
    );
    let inspection = zolar_pczt::zkool_inspection_pczt(&p.pczt)?;
    ensure!(
        zolar_pczt::shielded_v6(&inspection)?,
        "the demo requires a shielded v6 Zcash payment"
    );
    Ok(inspection)
}

pub fn verify_signed(raw: &[u8], expected: &str, expiry: u32) -> Result<()> {
    let mut remaining = raw;
    let tx = Transaction::read(&mut remaining, BranchId::Nu7)
        .context("invalid signed Zcash transaction")?;
    ensure!(
        remaining.is_empty()
            && tx.txid().to_string() == expected
            && u32::from(tx.expiry_height()) == expiry,
        "signed payment differs from the checked transaction"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(account: u32, write: bool, exp: i64) -> String {
        format!(
            "e30.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&json!({"sub":account,"write":write,"exp":exp})).unwrap()
            )
        )
    }

    #[test]
    fn amounts_preserve_every_zatoshi() {
        assert_eq!(zec_amount(1), "0.00000001");
        assert_eq!(zec_amount(100_000), "0.00100000");
        assert_eq!(zec_amount(u64::MAX), "184467440737.09551615");
    }

    #[test]
    fn delegated_access_is_account_scoped_and_cannot_be_redirected() {
        for endpoint in [
            "http://example.com/graphql",
            "http://localhost/graphql",
            "http://127.0.0.1.evil.test/graphql",
            "http://user:pass@127.0.0.1/graphql",
            "http://127.0.0.1/graphql?token=secret",
        ] {
            assert!(Zkool::new(endpoint, token(1, true, i64::MAX), 1).is_err());
        }
        assert!(Zkool::new("http://127.0.0.1:8000/graphql", token(1, true, i64::MAX), 1).is_ok());
        for t in [
            token(0, true, i64::MAX),
            token(2, true, i64::MAX),
            token(1, false, i64::MAX),
            token(1, true, 0),
        ] {
            assert!(Zkool::new("http://127.0.0.1:8000/graphql", t, 1).is_err());
        }
    }

    #[test]
    fn loopback_wallet_credentials_never_use_environment_proxies() {
        const CHILD: &str = "ZOLAR_ZKOOL_PROXY_REGRESSION_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "zkool::tests::loopback_wallet_credentials_never_use_environment_proxies",
                    "--exact",
                ])
                .env(CHILD, "1");
            for key in [
                "HTTP_PROXY",
                "http_proxy",
                "HTTPS_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
            ] {
                command.env(key, "http://127.0.0.1:9");
            }
            command.env("NO_PROXY", "").env("no_proxy", "");
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/graphql", server.server_addr());
        let thread = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("direct wallet request");
            let body = json!({"data":{"apiVersion":"1.0","currentHeight":4542685,"addressByAccount":{
                "ironwood":"utest1v64tshwxwwsjrajxvg65mde8s6lr7vhaw6q5jeaeq7hdk952pmswe64hhn6qw58pmnwhtpkgzz6hj8nn0z4lyh909n9upuq9cvxqht2a"}}});
            request
                .respond(tiny_http::Response::from_string(body.to_string()))
                .unwrap();
        });
        let wallet = Zkool::new(&endpoint, token(1, true, i64::MAX), 1).unwrap();
        let connected = wallet.identity();
        thread.join().unwrap();
        assert_eq!(connected.unwrap().account, 1);
    }

    #[test]
    fn standard_wallet_expiry_and_freshness_are_checked_separately() {
        assert!(check_heights(100, 124, 244).is_ok());
        for (relay, live, expiry) in [
            (100, 125, 245),
            (100, 99, 219),
            (100, 120, 125),
            (100, 100, 3556),
        ] {
            assert!(check_heights(relay, live, expiry).is_err());
        }
        assert!(check_backstop(100, 200).is_ok());
        assert!(check_backstop(100, 100 + 86_400 - 1800).is_err());
        assert!(check_backstop(100, 99).is_err());
    }

    #[test]
    fn malformed_packages_and_raw_transactions_never_reach_broadcast() {
        assert!(package_pczt(&[]).is_err());
        assert!(package_pczt(&[255; 64]).is_err());
        assert!(verify_signed(&[], &"00".repeat(32), 0).is_err());
    }

    #[test]
    fn prepared_package_is_decoded_without_modifying_wallet_effects() {
        let pczt =
            include_bytes!("../../crates/zolar-pczt/tests/fixtures/payment-zkool-unsigned.pczt")
                .to_vec();
        let package = Package {
            pczt: pczt.clone(),
            n_spends: [0, 0, 0, 1],
            sapling_indices: vec![],
            orchard_indices: vec![],
            ironwood_indices: vec![0],
            can_sign: true,
            can_broadcast: false,
            price: None,
            category: None,
            is_issuance: false,
        };
        let bytes = bincode::encode_to_vec(package, bincode::config::standard()).unwrap();
        let expected =
            include_bytes!("../../crates/zolar-pczt/tests/fixtures/payment-unsigned.pczt");
        assert_eq!(
            zolar_pczt::summary(&package_pczt(&bytes).unwrap()).unwrap(),
            zolar_pczt::summary(expected).unwrap()
        );
        let mut padded = bytes.clone();
        padded.push(0);
        assert!(package_pczt(&padded).is_err());
        assert_eq!(
            zolar_pczt::summary(&package_pczt(&bytes).unwrap())
                .unwrap()
                .txid,
            zolar_pczt::summary(expected).unwrap().txid
        );
    }

    #[test]
    #[ignore = "requires an unsigned package prepared by the released external wallet"]
    fn released_wallet_unsigned_package_compatibility() {
        let path = std::env::var("ZOLAR_UNSIGNED_COMPATIBILITY_PACKAGE").unwrap();
        let encoded = std::fs::read_to_string(path).unwrap();
        let package = hex::decode(encoded.trim()).unwrap();
        let inspection = package_pczt(&package).unwrap();
        let summary = zolar_pczt::summary(&inspection).unwrap();
        assert_eq!(summary.tx_version, 6);
        let redacted = zolar_pczt::redact_for_payee(&inspection).unwrap();
        assert_eq!(zolar_pczt::summary(&redacted).unwrap(), summary);
        println!(
            "REAL ZKOOL: released-wallet package decoded; native shielded v6 effects and expiry preserved through redaction"
        );
    }

    #[test]
    fn signed_transaction_identity_is_checked_locally_before_sending() {
        let raw = hex::decode(
            include_str!("../../crates/zolar-pczt/tests/fixtures/mainnet-v6-16394087.hex").trim(),
        )
        .unwrap();
        let tx = Transaction::read(&raw[..], BranchId::Nu7).unwrap();
        let id = tx.txid().to_string();
        let expiry = u32::from(tx.expiry_height());
        verify_signed(&raw, &id, expiry).unwrap();
        assert!(verify_signed(&raw, &"00".repeat(32), expiry).is_err());
        assert!(verify_signed(&raw, &id, expiry.saturating_add(1)).is_err());
        let mut padded = raw;
        padded.push(0);
        assert!(verify_signed(&padded, &id, expiry).is_err());
    }
}
