use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use solana_address::Address;
use zolar_relayer::rpc::{is_rate_limit, program_error, HttpRpc};

fn request_body(stream: &std::net::TcpStream) -> serde_json::Value {
    use std::io::BufRead;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn reply(stream: &mut std::net::TcpStream, result: serde_json::Value) {
    let body = json!({"jsonrpc":"2.0","id":1,"result":result}).to_string();
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
}

#[test]
fn a_lost_submission_response_can_retry_the_exact_saved_transaction() {
    use base64::Engine;
    use solana_keypair::Keypair;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let rpc = HttpRpc::new(format!("http://{}", listener.local_addr().unwrap()));
    let server = thread::spawn(move || {
        let mut wires = Vec::new();
        let mut signature = String::new();
        for step in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let body = request_body(&stream);
            match step {
                0 => {
                    assert_eq!(body["method"], "getLatestBlockhash");
                    reply(
                        &mut stream,
                        json!({"context":{"slot":42},"value":{"blockhash":bs58::encode([7;32]).into_string(),"lastValidBlockHeight":100}}),
                    );
                }
                1 | 2 => {
                    assert_eq!(body["method"], "sendTransaction");
                    let wire = body["params"][0].as_str().unwrap().to_string();
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&wire)
                        .unwrap();
                    signature = bs58::encode(&bytes[1..65]).into_string();
                    wires.push(wire);
                    if step == 2 {
                        reply(&mut stream, json!(signature));
                    }
                    // First send was received, but its HTTP response is lost.
                }
                _ => {
                    assert_eq!(body["method"], "getSignatureStatuses");
                    assert_eq!(body["params"][0][0], signature);
                    reply(
                        &mut stream,
                        json!({"value":[{"err":null,"confirmationStatus":"finalized"}]}),
                    );
                }
            }
        }
        assert_eq!(wires.len(), 2);
        assert_eq!(wires[0], wires[1]);
        signature
    });
    let saved = rpc.prepare(&[], &Keypair::new()).unwrap();
    assert_eq!(saved.context_slot, 42);
    assert_eq!(saved.last_valid_block_height, 100);
    let restored = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert!(rpc.submit_prepared(&saved).is_err());
    let acknowledged = rpc.submit_prepared(&restored).unwrap();
    assert_eq!(acknowledged, saved.signature);
    assert_eq!(server.join().unwrap(), acknowledged);
}

#[test]
fn a_program_refusal_is_told_apart_from_other_failures() {
    // As `sendTransaction` preflight and `getSignatureStatuses` report them.
    let refused = json!({"InstructionError": [1, {"Custom": 27}]});
    assert_eq!(program_error(&refused), Some((1, 27)));
    assert_eq!(
        program_error(&json!({"InstructionError": [0, "InvalidAccountData"]})),
        None
    );
    assert_eq!(program_error(&json!("BlockhashNotFound")), None);
}

#[test]
fn the_public_rpc_rate_limit_answer_is_recognised() {
    let answer = json!({"code": 429, "message": "Too many requests for a specific RPC call"});
    assert!(is_rate_limit(&answer));
    assert!(!is_rate_limit(&json!({"code": -32002})));
}

#[test]
fn a_plain_http_429_is_waited_out_and_asked_again() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let replies = [
            "HTTP/1.1 429 Too Many Requests\r\ncontent-length: 8\r\nconnection: close\r\n\r\nslow down",
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 62\r\nconnection: close\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":123456},\"x\":\"y\"}    ",
        ];
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });

    let lamports = HttpRpc::new(url).balance(&Address::new_from_array([1; 32]));

    server.join().unwrap();
    assert_eq!(lamports, Ok(123_456));
}
