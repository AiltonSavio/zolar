//! Forges a header that passes Equihash but misses its difficulty target.
//!
//! Real blocks always meet their target, so a "hash above target" fixture cannot be exported from
//! a chain. This takes a real header (arg 1: path to its hex), keeps its parent, Merkle root and
//! time, sets `nBits` to a target far tighter than the header's (arg 2: compact value in hex),
//! then runs the Equihash solver over nonces until it finds a valid solution whose SHA-256d hash is
//! above that target. Prints the forged header as hex. Equihash input covers `nBits`, so the real
//! solution cannot be reused.
//!
//! Usage: forge-header fixtures/testnet-4483600.hex 1d00ffff > fixtures/forged-testnet-hash-above-target.hex
use sha2::{Digest, Sha256};

fn target_above_hash(bits: u32, hash: &[u8; 32]) -> bool {
    let size = (bits >> 24) as usize;
    let mantissa = (bits & 0x007f_ffff).to_le_bytes();
    let mut target = [0u8; 32];
    for (i, b) in mantissa.iter().take(3).enumerate() {
        target[size - 3 + i] = *b;
    }
    // Both are little-endian numbers: compare from the most significant byte.
    for i in (0..32).rev() {
        if hash[i] != target[i] {
            return hash[i] > target[i];
        }
    }
    false
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("path to a header hex file");
    let bits = u32::from_str_radix(&args.next().expect("compact bits in hex"), 16).unwrap();
    let mut header = hex::decode(std::fs::read_to_string(path).unwrap().trim()).unwrap();
    assert_eq!(header.len(), 1487);
    header[104..108].copy_from_slice(&bits.to_le_bytes());

    let mut nonce = [0u8; 32];
    let mut counter = 0u32;
    loop {
        let input = header[..108].to_vec();
        let solutions = equihash::tromp::solve_200_9(&input, || {
            nonce[..4].copy_from_slice(&counter.to_le_bytes());
            counter += 1;
            Some(nonce)
        });
        // The solver ran one nonce; `counter - 1` produced these solutions.
        nonce[..4].copy_from_slice(&(counter - 1).to_le_bytes());
        for solution in solutions {
            let mut forged = header[..108].to_vec();
            forged.extend_from_slice(&nonce);
            forged.extend_from_slice(&[0xfd, 0x40, 0x05]);
            forged.extend_from_slice(&solution);
            assert_eq!(forged.len(), 1487);
            equihash::is_valid_solution(200, 9, &forged[..108], &forged[108..140], &forged[143..])
                .expect("solver output must verify");
            let hash: [u8; 32] = Sha256::digest(Sha256::digest(&forged)).into();
            if target_above_hash(bits, &hash) {
                println!("{}", hex::encode(forged));
                return;
            }
        }
    }
}
