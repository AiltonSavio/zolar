
use relay_tests::*;

#[test]
fn checkpoint_initialises_a_relay_at_the_checkpoint_block() {
    let mut h = Harness::new();
    let checkpoint = testnet_checkpoint(10, 0);
    let relay_addr = h.init_relay(&checkpoint);

    let relay = h.relay(&relay_addr);
    assert_eq!(relay.state, 1, "sealed relay is active");
    assert_eq!(relay.halted, 0);
    assert_eq!(relay.tip_height, 4_483_599);
    assert_eq!(relay.tip_work, 0);
    assert_eq!(relay.tip_hash, checkpoint.hash);
    assert_eq!(relay.checkpoint.finality_depth, 10);
    assert_eq!(relay.checkpoint.times[112], checkpoint.times[112]);
}
