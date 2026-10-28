//! Zcash block explorer (zecblock, testnet or mainnet host), used only by the mainnet rehearsal example to
//! pick a payment that is already mined. Neither the Payer nor the Payee asks it anything: a lookup
//! would tell the explorer which payment an Escrow or a Claim belongs to.

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

    /// The transaction IDs of a block in block order, display form.
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
