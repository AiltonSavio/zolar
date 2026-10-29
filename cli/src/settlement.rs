//! Positive, finalized Escrow history for restart recovery. Account absence is never proof of a
//! Claim, and neither a failed transaction nor another program's instruction establishes settlement.
use anyhow::{Context, Result, bail};
use escrow_client::programs::ESCROW_ID;
use serde_json::{Value, json};
use solana_address::Address;
use zolar_relayer::rpc::{HttpRpc, PreparedTransaction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Created,
    Claimed,
    Refunded,
}

#[derive(Default, Debug)]
pub(crate) struct History {
    pub created: Option<String>,
    pub settled: Option<(Event, String)>,
}

fn key(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value.get("pubkey")?.as_str())
}

/// Handles legacy and v0 keys. Only top-level instructions inherit transaction success: a CPI error
/// can be caught by its caller, so an inner instruction is not settlement evidence. Claims and Refund use
/// account index 0; creation uses index 1.
pub(crate) fn event(tx: &Value, escrow: &Address) -> Option<Event> {
    if tx.get("meta")?.get("err")? != &Value::Null {
        return None;
    }
    let message = tx.get("transaction")?.get("message")?;
    let mut keys: Vec<&str> = message
        .get("accountKeys")?
        .as_array()?
        .iter()
        .map(key)
        .collect::<Option<_>>()?;
    for kind in ["writable", "readonly"] {
        if let Some(extra) = tx["meta"]["loadedAddresses"][kind].as_array() {
            keys.extend(extra.iter().map(key).collect::<Option<Vec<_>>>()?);
        }
    }
    let instructions = message.get("instructions")?.as_array()?;
    let wanted = escrow.to_string();
    let program = ESCROW_ID.to_string();
    for instruction in instructions {
        let id = instruction["programId"].as_str().or_else(|| {
            keys.get(instruction["programIdIndex"].as_u64()? as usize)
                .copied()
        });
        if id != Some(program.as_str()) {
            continue;
        }
        let data = instruction["data"]
            .as_str()
            .and_then(|s| bs58::decode(s).into_vec().ok());
        let (event, position) = match data.as_deref().and_then(|d| d.first()) {
            Some(0) => (Event::Created, 1),
            Some(1 | 4) => (Event::Claimed, 0),
            Some(3) => (Event::Refunded, 0),
            _ => continue,
        };
        let Some(account) = instruction["accounts"].get(position) else {
            continue;
        };
        let address = key(account).or_else(|| keys.get(account.as_u64()? as usize).copied());
        if address == Some(wanted.as_str()) {
            return Some(event);
        }
    }
    None
}

pub(crate) fn transaction(rpc: &HttpRpc, signature: &str) -> Result<Option<Value>> {
    let tx = rpc
        .request(
            "getTransaction",
            json!([signature, {
                "encoding": "json", "commitment": "finalized", "maxSupportedTransactionVersion": 0,
            }]),
        )
        .map_err(anyhow::Error::msg)?;
    if tx.is_null() {
        return Ok(None);
    }
    if tx["transaction"]["signatures"][0].as_str() != Some(signature) {
        bail!("RPC returned a different transaction");
    }
    Ok(Some(tx))
}

pub(crate) fn history(rpc: &HttpRpc, escrow: &Address) -> Result<History> {
    let mut before: Option<String> = None;
    let mut found = History::default();
    for _ in 0..10 {
        let mut config = json!({"commitment": "finalized", "limit": 100});
        if let Some(before) = &before {
            config["before"] = json!(before);
        }
        let page = rpc
            .request(
                "getSignaturesForAddress",
                json!([escrow.to_string(), config]),
            )
            .map_err(anyhow::Error::msg)?;
        let rows = page.as_array().context("RPC history is not an array")?;
        for row in rows {
            let signature = row["signature"]
                .as_str()
                .context("history has no signature")?;
            before = Some(signature.to_string());
            if !row
                .get("err")
                .context("history has no transaction outcome")?
                .is_null()
            {
                continue;
            }
            let tx = transaction(rpc, signature)?
                .context("finalized transaction history is unavailable")?;
            match event(&tx, escrow) {
                Some(Event::Created) => {
                    found.created = Some(signature.to_string());
                    return Ok(found);
                }
                Some(event) if found.settled.is_none() => {
                    found.settled = Some((event, signature.to_string()));
                }
                _ => {}
            }
        }
        if rows.len() < 100 {
            if found.settled.is_some() {
                bail!("Escrow creation history is unavailable");
            }
            return Ok(found);
        }
    }
    bail!("Escrow history needs more than 1000 transactions; recovery remains unresolved")
}

/// A new blockhash is allowed only when the old one cannot land, the RPC still covers that period,
/// and the searched signature is absent or finalized with an error. Callers also check the account
/// and complete address history before invoking this. Incomplete evidence means no replacement.
pub(crate) fn can_replace(rpc: &HttpRpc, tx: &PreparedTransaction) -> Result<bool> {
    let height = rpc
        .request("getBlockHeight", json!([{"commitment": "finalized"}]))
        .map_err(anyhow::Error::msg)?
        .as_u64()
        .context("no finalized block height")?;
    if height <= tx.last_valid_block_height {
        return Ok(false);
    }
    let first = rpc
        .request("getFirstAvailableBlock", json!([]))
        .map_err(anyhow::Error::msg)?
        .as_u64()
        .context("no history coverage")?;
    if first > tx.context_slot {
        bail!("RPC history no longer covers the saved transaction");
    }
    let statuses = rpc
        .request(
            "getSignatureStatuses",
            json!([[tx.signature], {"searchTransactionHistory": true}]),
        )
        .map_err(anyhow::Error::msg)?;
    let status = statuses["value"]
        .as_array()
        .and_then(|v| v.first())
        .context("no signature status")?;
    Ok(status.is_null()
        || (status["confirmationStatus"] == "finalized"
            && status.get("err").is_some_and(|e| !e.is_null())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(tag: u8) -> Value {
        let escrow = Address::new_from_array([7; 32]);
        json!({"meta":{"err":null},"transaction":{"message":{
            "accountKeys":["payer",escrow.to_string(),ESCROW_ID.to_string()],
            "instructions":[{"programIdIndex":2,"accounts":if tag == 1 || tag == 3 || tag == 4 {vec![1,0]} else {vec![0,1]},"data":bs58::encode([tag]).into_string()}]
        }}})
    }

    #[test]
    fn refund_metadata_matches_the_generated_client_and_never_settles_the_relay_account() {
        use escrow_client::instructions::Refund;
        let address = Address::new_from_array([7; 32]);
        let relay = Address::new_from_array([8; 32]);
        let ix = Refund {
            escrow: address,
            relay,
            vault: Address::new_from_array([9; 32]),
            payee_token: Address::new_from_array([10; 32]),
            payee: Address::new_from_array([11; 32]),
            token_program: crate::solana::TOKEN_PROGRAM,
        }
        .instruction();
        let mut keys: Vec<String> = ix.accounts.iter().map(|a| a.pubkey.to_string()).collect();
        let program_index = keys.len();
        keys.push(ix.program_id.to_string());
        let tx = json!({"meta":{"err":null},"transaction":{"message":{"accountKeys":keys,"instructions":[{
            "programIdIndex":program_index,"accounts":(0..ix.accounts.len()).collect::<Vec<_>>(),"data":bs58::encode(ix.data).into_string()}]}}});
        assert_eq!(event(&tx, &address), Some(Event::Refunded));
        assert_eq!(event(&tx, &relay), None);
    }

    #[test]
    fn only_a_successful_bound_instruction_can_establish_settlement() {
        let address = Address::new_from_array([7; 32]);
        assert_eq!(event(&tx(4), &address), Some(Event::Claimed));
        assert_eq!(event(&tx(1), &address), Some(Event::Claimed));
        assert_eq!(event(&tx(3), &address), Some(Event::Refunded));
        assert_eq!(event(&tx(0), &address), Some(Event::Created));
        assert_eq!(event(&tx(5), &address), None);
        assert_eq!(event(&tx(4), &Address::new_from_array([8; 32])), None);
        let mut failed = tx(4);
        failed["meta"]["err"] = json!({"InstructionError":[0,"InvalidArgument"]});
        assert_eq!(event(&failed, &address), None);
        let mut wrong = tx(4);
        wrong["transaction"]["message"]["instructions"][0]["programIdIndex"] = json!(0);
        assert_eq!(event(&wrong, &address), None);
        assert_eq!(event(&json!({}), &address), None);
    }

    #[test]
    fn loaded_addresses_are_resolved() {
        let address = Address::new_from_array([7; 32]);
        let mut loaded = tx(4);
        loaded["transaction"]["message"]["accountKeys"] = json!(["payer"]);
        loaded["meta"]["loadedAddresses"] =
            json!({"writable":[address.to_string()],"readonly":[ESCROW_ID.to_string()]});
        assert_eq!(event(&loaded, &address), Some(Event::Claimed));
    }

    #[test]
    fn a_cpi_whose_error_was_caught_does_not_establish_settlement() {
        let address = Address::new_from_array([7; 32]);
        let mut caught = tx(4);
        let instruction = caught["transaction"]["message"]["instructions"][0].take();
        caught["transaction"]["message"]["instructions"] = json!([]);
        caught["meta"]["innerInstructions"] = json!([{"index":0,"instructions":[instruction]}]);
        // meta.err is null for the wrapper, even though its CPI may have failed.
        assert_eq!(event(&caught, &address), None);
    }

    #[test]
    fn replacing_a_creation_requires_expiry_status_and_complete_history_coverage() {
        use crate::test_support::Rpc;
        for (height, first, status, expected) in [
            (99, 0, Value::Null, Some(false)),
            (200, 11, Value::Null, None),
            (200, 0, Value::Null, Some(true)),
            (
                200,
                0,
                json!({"confirmationStatus":"finalized","err":null}),
                Some(false),
            ),
            (
                200,
                0,
                json!({"confirmationStatus":"confirmed","err":{"InstructionError":[0,"InvalidArgument"]}}),
                Some(false),
            ),
            (
                200,
                0,
                json!({"confirmationStatus":"finalized","err":{"InstructionError":[0,"InvalidArgument"]}}),
                Some(true),
            ),
        ] {
            let rpc = Rpc::new(move |request| {
                Ok(match request["method"].as_str().unwrap() {
                    "getBlockHeight" => json!(height),
                    "getFirstAvailableBlock" => json!(first),
                    "getSignatureStatuses" => json!({"value":[status]}),
                    other => panic!("unexpected {other}"),
                })
            });
            let saved = PreparedTransaction {
                signature: "saved".into(),
                wire: "wire".into(),
                last_valid_block_height: 100,
                context_slot: 10,
            };
            let actual = can_replace(&HttpRpc::new(&rpc.url), &saved);
            match expected {
                Some(value) => assert_eq!(actual.unwrap(), value),
                None => assert!(actual.is_err()),
            };
        }
    }
}
