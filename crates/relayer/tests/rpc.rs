use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use solana_address::Address;
use zolar_relayer::rpc::{is_rate_limit, program_error, HttpRpc};

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
