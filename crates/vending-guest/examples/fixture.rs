#[path = "../tests/common/mod.rs"]
mod common;
fn main() {
    let dir = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&dir).unwrap();
    for kind in ["valid", "value", "rseed", "rho", "memo"] {
        let witness = common::fixture(kind);
        std::fs::write(
            format!("{dir}/{kind}.json"),
            serde_json::to_vec(&witness).unwrap(),
        )
        .unwrap();
    }
}
