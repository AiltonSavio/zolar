//! Local RPC boundary fixtures. No chain access or funded wallets; state encodes with the generated
//! client, so failure tests exercise the service's real HTTP, persistence and transaction signing.
use base64::Engine;
use serde_json::{Value, json};
use solana_address::Address;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub(crate) struct Rpc {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Rpc {
    pub fn new(mut handler: impl FnMut(&Value) -> Result<Value, Value> + Send + 'static) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let thread = thread::spawn(move || {
            while !end.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap()
                else {
                    continue;
                };
                let body: Value = serde_json::from_reader(request.as_reader()).unwrap();
                recorded.lock().unwrap().push(body.clone());
                let mut reply = json!({"jsonrpc":"2.0","id":body["id"]});
                match handler(&body) {
                    Ok(value) => reply["result"] = value,
                    Err(error) => reply["error"] = error,
                };
                request
                    .respond(
                        tiny_http::Response::from_string(reply.to_string()).with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        ),
                    )
                    .unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

pub(crate) fn account(owner: Address, data: &[u8]) -> Value {
    json!({"value":{"owner":owner.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"]}})
}

pub(crate) fn relay(tip: u32) -> Vec<u8> {
    use header_relay_interface::*;
    let mut data = vec![0; RELAY_LEN];
    data[R_DISC] = DISC_RELAY;
    data[R_VERSION] = VERSION;
    data[R_STATE] = STATE_ACTIVE;
    data[R_CHECKPOINT + CP_NETWORK] = NETWORK_TESTNET;
    data[R_CHECKPOINT + CP_FINALITY_DEPTH..R_CHECKPOINT + CP_FINALITY_DEPTH + 2]
        .copy_from_slice(&10u16.to_le_bytes());
    data[R_TIP_HEIGHT..R_TIP_HEIGHT + 4].copy_from_slice(&tip.to_le_bytes());
    data
}

pub(crate) fn token(owner: Address, mint: Address, amount: u64) -> Vec<u8> {
    let mut data = vec![0; 165];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    data
}

pub(crate) fn escrow(
    payee: Address,
    payout: Address,
    mint: Address,
    relay: Address,
    txid: &str,
    salt: [u8; 32],
    expiry: u32,
) -> (Address, Vec<u8>) {
    let commitment =
        zolar_claim::payment_commitment(&crate::internal_txid(txid).unwrap(), &salt).unwrap();
    let (address, bump) = escrow_client::accounts::Escrow::find_pda(&payee, commitment);
    let value = escrow_client::accounts::Escrow {
        discriminator: 1,
        version: 1,
        state: 0,
        bump,
        payee,
        payout,
        mint,
        vault: crate::solana::associated_token_account(&address, &mint),
        relay,
        commitment,
        amount: 1,
        bond_required: 0,
        bond_deposited: 0,
        created_at: 0,
        created_tip: expiry - 18,
        expiry_height: expiry,
        claim_mode: 1,
        refund_requested_at: 0,
        reserved: [0; 55],
    };
    (address, borsh::to_vec(&value).unwrap())
}

pub(crate) fn transaction(signature: &str, escrow: Address, tag: u8) -> Value {
    json!({"meta":{"err":null},"transaction":{"signatures":[signature],"message":{
        "accountKeys":["payer",escrow.to_string(),escrow_client::programs::ESCROW_ID.to_string()],
        "instructions":[{"programIdIndex":2,"accounts":if tag == 1 || tag == 3 || tag == 4 {vec![1,0]} else {vec![0,1]},
            "data":bs58::encode([tag]).into_string()}]
    }}})
}
