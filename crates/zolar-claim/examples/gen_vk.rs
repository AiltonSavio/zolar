//! Writes the Escrow's verifying key constants from a snarkjs `verification_key.json`:
//!
//!   cargo run -p zolar-claim --example gen_vk -- <verification_key.json> programs/escrow/src
//!
//! Re-run it after every ceremony; CI regenerates from the committed JSON and fails on a difference.

use groth16_solana::vk::circom::generate_vk_file;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(vk), Some(dir)) = (args.next(), args.next()) else {
        eprintln!("usage: gen_vk <verification_key.json> <output dir>");
        std::process::exit(2);
    };
    generate_vk_file(&vk, &dir, "verifying_key_generated.rs").expect("vk generation failed");
}
