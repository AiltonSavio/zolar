//! Zcash block explorer (zecblock, testnet or mainnet host), used only by the Payee to check that a transaction is new.
//! The Payer never asks it anything: a lookup would tell the explorer which payment a Claim belongs to
//! (privacy limit, see the README).

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

pub struct Explorer {
    base: String,
    agent: ureq::Agent,
}

impl Explorer {
    pub fn new(base: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            agent,
        }
    }

    fn get(&self, path: &str) -> Result<(u16, Value)> {
        let url = format!("{}{}", self.base, path);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .with_context(|| format!("GET {url}"))?;
        let status = response.status().as_u16();
        let body: Value = response
            .body_mut()
            .read_json()
            .with_context(|| format!("GET {url}: answer is not JSON (HTTP {status})"))?;
        Ok((status, body))
    }

    /// Whether the explorer has seen the transaction at all (mined or waiting in a mempool).
    pub fn knows(&self, display_txid: &str) -> Result<bool> {
        let (status, _) = self.get(&format!("/api/tx/{display_txid}"))?;
        Ok(status != 404)
    }

    /// The transaction IDs of a block in block order, display form. Only the mainnet rehearsal example
    /// uses it, to pick a payment that is already mined; the Payer never does.
    pub fn block_txids(&self, height: u32) -> Result<Vec<String>> {
        let (status, body) = self.get(&format!("/api/block/{height}"))?;
        anyhow::ensure!(status == 200, "block {height}: HTTP {status}");
        body["transactions"]
            .as_array()
            .context("no transactions")?
            .iter()
            .map(|t| {
                t["txid"]
                    .as_str()
                    .map(str::to_string)
                    .context("transaction without txid")
            })
            .collect()
    }
}
