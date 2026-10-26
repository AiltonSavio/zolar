//! Integration stub: rejects every proof. Replace with the SP1 v6 verifier crate.
#![no_std]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_every_proof_shape() {
        for proof in [&[][..], &[0; 256][..], &[1; 388][..]] {
            assert_eq!(
                super::verify_proof(proof, &[0; 249], &[0; 32]),
                Err(super::Error)
            );
        }
    }
}

pub fn verify_proof(_: &[u8], _: &[u8], _: &[u8; 32]) -> Result<(), Error> {
    Err(Error)
}
