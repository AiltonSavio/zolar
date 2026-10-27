use vending_prover::wire::{frame, unframe, BlockId};
#[test]
fn grpc_web_requires_success_trailers_and_exact_frames() {
    let msg = frame(&BlockId {
        height: 4515934,
        hash: vec![],
    });
    let trailer = b"grpc-status: 0\r\n";
    let mut good = msg.clone();
    good.push(128);
    good.extend((trailer.len() as u32).to_be_bytes());
    good.extend(trailer);
    assert_eq!(unframe(&good).unwrap().len(), 1);
    assert!(unframe(&msg).is_err());
    assert!(unframe(&good[..good.len() - 1]).is_err());
    let mut bad = good.clone();
    let last = bad.iter().rposition(|b| *b == b'0').unwrap();
    bad[last] = b'7';
    assert!(unframe(&bad).is_err());
    let mut compressed = good;
    compressed[0] = 1;
    assert!(unframe(&compressed).is_err());
}
