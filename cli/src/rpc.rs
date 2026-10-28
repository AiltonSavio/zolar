//! Account reads at a chosen commitment. The Relayer's cluster client reads at `confirmed`; a Payer
//! must see the Escrow at `finalized` before it starts a Zcash payment that cannot be undone.

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use solana_address::Address;

/// The data of `address` at the `finalized` commitment, or `None` if it does not exist there (yet).
pub fn finalized_account(url: &str, address: &Address) -> Result<Option<Vec<u8>>> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "getAccountInfo",
        "params": [address.to_string(), {"encoding": "base64", "commitment": "finalized"}]});
    let mut response = ureq::post(url)
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .send_json(body)
        .map_err(|e| anyhow!("getAccountInfo: {e}"))?;
    let value: Value = response.body_mut().read_json()?;
    if let Some(error) = value.get("error") {
        bail!("getAccountInfo: {error}");
    }
    let account = &value["result"]["value"];
    if account.is_null() {
        return Ok(None);
    }
    let data = account["data"][0]
        .as_str()
        .ok_or_else(|| anyhow!("account has no data"))?;
    Ok(Some(STANDARD.decode(data)?))
}

/// Like [`finalized_account`], asking again until the account appears or `wait` has passed.
pub fn finalized_account_within(url: &str, address: &Address, wait: Duration) -> Result<Vec<u8>> {
    let started = Instant::now();
    loop {
        if let Some(data) = finalized_account(url, address)? {
            return Ok(data);
        }
        if started.elapsed() >= wait {
            bail!("account {address} is not finalized on Solana (waited {wait:?})");
        }
        println!("waiting for {address} to be finalized on Solana");
        std::thread::sleep(Duration::from_secs(5));
    }
}
