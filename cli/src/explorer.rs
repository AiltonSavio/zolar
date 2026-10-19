//! Zcash block explorer (zecblock, the testnet or mainnet host) used for the data the Relay does not hold: whether a
//! transaction was mined and in which block, and the transaction IDs of a block.
//!
//! The explorer is not trusted: [`crate::blocks::verified_block_txids`] recomputes the block's Merkle
//! root from what it returns and compares it with the header received over Zcash P2P, and the Escrow
//! checks the result again on-chain against the Final Block Tree.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

pub struct Explorer {
    base: String,
    agent: ureq::Agent,
}

/// A mined transaction.
#[derive(Debug, PartialEq, Eq)]
pub struct Mined {
    pub block_height: u32,
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

    /// `None` while the explorer does not know the transaction (not yet seen, or never mined).
    pub fn mined(&self, display_txid: &str) -> Result<Option<Mined>> {
        let (status, body) = self.get(&format!("/api/tx/{display_txid}"))?;
        if status == 404 {
            return Ok(None);
        }
        if body["status"].as_str() != Some("confirmed") || body["isCanonical"] != Value::Bool(true)
        {
            return Ok(None);
        }
        let height = number(&body["blockHeight"]).ok_or_else(|| anyhow!("no blockHeight"))?;
        Ok(Some(Mined {
            block_height: height as u32,
        }))
    }

    /// Whether the explorer has seen the transaction at all (mined or waiting in a mempool).
    pub fn knows(&self, display_txid: &str) -> Result<bool> {
        let (status, _) = self.get(&format!("/api/tx/{display_txid}"))?;
        Ok(status != 404)
    }

    /// The transaction IDs of a block in block order, display form.
    pub fn block_txids(&self, height: u32) -> Result<Vec<String>> {
        let (status, body) = self.get(&format!("/api/block/{height}"))?;
        anyhow::ensure!(status == 200, "block {height}: HTTP {status}");
        body["transactions"]
            .as_array()
            .ok_or_else(|| anyhow!("block {height}: no transactions"))?
            .iter()
            .map(|t| {
                t["txid"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("transaction without txid"))
            })
            .collect()
    }

    /// The tip height of the explorer's chain.
    pub fn tip(&self) -> Result<u32> {
        let (_, body) = self.get("/v1/blocks?limit=1")?;
        number(&body["data"][0]["height"])
            .map(|h| h as u32)
            .ok_or_else(|| anyhow!("no tip height"))
    }
}

/// A JSON number that the explorer may send as a string.
fn number(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_may_come_as_strings() {
        assert_eq!(number(&serde_json::json!("4515934")), Some(4_515_934));
        assert_eq!(number(&serde_json::json!(7)), Some(7));
        assert_eq!(number(&serde_json::json!(null)), None);
    }
}
