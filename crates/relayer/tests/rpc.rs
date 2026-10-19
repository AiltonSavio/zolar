use serde_json::json;
use zolar_relayer::rpc::{is_rate_limit, program_error};

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
