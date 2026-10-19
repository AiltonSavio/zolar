//! A Solana JSON-RPC cluster over HTTP.
use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use solana_address::Address;
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::chain::{Cluster, TxError};

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(1000);
/// Public RPC nodes answer 429 when asked too often; wait and ask again.
const RATE_LIMIT_RETRIES: u32 = 8;

pub struct HttpRpc {
    url: String,
    agent: ureq::Agent,
}

impl HttpRpc {
    pub fn new(url: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            url: url.into(),
            agent,
        }
    }

    /// Returns the `result` of a call, or the `error` object as `Err`.
    fn call(&self, method: &str, params: Value) -> Result<Result<Value, Value>, String> {
        for attempt in 1..=RATE_LIMIT_RETRIES {
            match self.call_once(method, &params)? {
                Err(error) if is_rate_limit(&error) => {
                    sleep(Duration::from_millis(1500 * attempt as u64))
                }
                answer => return Ok(answer),
            }
        }
        Err(format!("{method}: rate limited"))
    }

    fn call_once(&self, method: &str, params: &Value) -> Result<Result<Value, Value>, String> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let mut response = self
            .agent
            .post(&self.url)
            .send_json(body)
            .map_err(|e| format!("{method}: {e}"))?;
        let status = response.status();
        let value: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| format!("{method}: HTTP {status}: {e}"))?;
        match value.get("error") {
            Some(error) => Ok(Err(error.clone())),
            None => value
                .get("result")
                .cloned()
                .map(Ok)
                .ok_or_else(|| format!("{method}: HTTP {status}: no result")),
        }
    }

    /// Lamports held by `address`.
    pub fn balance(&self, address: &Address) -> Result<u64, String> {
        let result = self
            .call(
                "getBalance",
                json!([address.to_string(), {"commitment": "confirmed"}]),
            )?
            .map_err(|e| format!("getBalance: {e}"))?;
        result["value"]
            .as_u64()
            .ok_or_else(|| "getBalance: no value".into())
    }

    fn blockhash(&self) -> Result<Hash, String> {
        let result = self
            .call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))?
            .map_err(|e| format!("getLatestBlockhash: {e}"))?;
        let text = result["value"]["blockhash"]
            .as_str()
            .ok_or("getLatestBlockhash: no blockhash")?;
        let bytes: [u8; 32] = bs58::decode(text)
            .into_vec()
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "blockhash is not 32 bytes".to_string())?;
        Ok(Hash::new_from_array(bytes))
    }
}

pub fn is_rate_limit(error: &Value) -> bool {
    error.get("code").and_then(Value::as_i64) == Some(429)
}

/// `InstructionError: [index, {"Custom": code}]` in a transaction error, if that is what it is.
pub fn program_error(err: &Value) -> Option<(usize, u32)> {
    let pair = err.get("InstructionError")?.as_array()?;
    let index = pair.first()?.as_u64()? as usize;
    let code = pair.get(1)?.get("Custom")?.as_u64()? as u32;
    Some((index, code))
}

fn tx_error(err: &Value) -> TxError {
    match program_error(err) {
        Some((index, code)) => TxError::Program { index, code },
        None => TxError::Other(err.to_string()),
    }
}

/// Shortvec-prefixed signatures, then the message: the wire format of a legacy transaction.
fn wire(tx: &Transaction) -> Vec<u8> {
    let mut out = vec![tx.signatures.len() as u8];
    for signature in &tx.signatures {
        out.extend_from_slice(signature.as_ref());
    }
    out.extend_from_slice(&tx.message.serialize());
    out
}

impl Cluster for HttpRpc {
    fn send(&mut self, instructions: &[Instruction], payer: &Keypair) -> Result<String, TxError> {
        let blockhash = self.blockhash().map_err(TxError::Other)?;
        let tx = Transaction::new(
            &[payer],
            Message::new(instructions, Some(&payer.pubkey())),
            blockhash,
        );
        let sent = self
            .call(
                "sendTransaction",
                json!([STANDARD.encode(wire(&tx)), {
                    "encoding": "base64",
                    "preflightCommitment": "confirmed",
                    "maxRetries": 5,
                }]),
            )
            .map_err(TxError::Other)?;
        let signature = match sent {
            Ok(value) => value.as_str().unwrap_or_default().to_string(),
            Err(error) => {
                return Err(match error.get("data").and_then(|d| d.get("err")) {
                    Some(err) => tx_error(err),
                    None => TxError::Other(format!("sendTransaction: {error}")),
                })
            }
        };
        let started = Instant::now();
        while started.elapsed() < CONFIRM_TIMEOUT {
            sleep(POLL);
            let statuses = self
                .call("getSignatureStatuses", json!([[signature]]))
                .map_err(TxError::Other)?
                .map_err(|e| TxError::Other(format!("getSignatureStatuses: {e}")))?;
            let status = &statuses["value"][0];
            if status.is_null() {
                continue;
            }
            if !status["err"].is_null() {
                return Err(tx_error(&status["err"]));
            }
            if matches!(
                status["confirmationStatus"].as_str(),
                Some("confirmed" | "finalized")
            ) {
                return Ok(signature);
            }
        }
        Err(TxError::Other(format!("{signature} not confirmed in time")))
    }

    fn account(&mut self, address: &Address) -> Result<Option<Vec<u8>>, String> {
        let result = self
            .call(
                "getAccountInfo",
                json!([address.to_string(), {"encoding": "base64", "commitment": "confirmed"}]),
            )?
            .map_err(|e| format!("getAccountInfo: {e}"))?;
        let value = &result["value"];
        if value.is_null() {
            return Ok(None);
        }
        let data = value["data"][0].as_str().ok_or("account has no data")?;
        STANDARD
            .decode(data)
            .map(Some)
            .map_err(|e| format!("account data: {e}"))
    }
}
