use vending_prover::engine::check_proving_memory;
#[test]
fn proving_requires_real_total_and_available_memory() {
    for text in [
        "",
        "MemTotal: 16000000 kB\nMemAvailable: 15000000 kB",
        "MemTotal: 64000000 kB\nMemAvailable: 1000000 kB",
    ] {
        assert!(check_proving_memory(text).is_err());
    }
    assert!(check_proving_memory("MemTotal: 64000000 kB\nMemAvailable: 40000000 kB").is_ok());
}
