#![no_main]
sp1_zkvm::entrypoint!(main);
pub fn main() {
    let witness: vending_guest::Witness = sp1_zkvm::io::read();
    let values = vending_guest::verify(&witness).expect("invalid vending payment");
    sp1_zkvm::io::commit_slice(&values.encode());
}
