//! The Private Claim proof, made on the Payer's machine.
//!
//! The witness holds the transaction ID, the salt and the position of the payment, so it never leaves
//! this process except through two local child processes: `circuits/scripts/prove.sh` (snarkjs
//! witness generation in node, then the local Groth16 prover) reading a file in a private directory
//! that is deleted afterwards. No network access happens here and no prover service exists.

use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use zolar_claim::private_claim::{CircuitInput, ProofBytes, public_from_snarkjs};

pub struct Prover {
    /// `circuits/scripts/prove.sh` of this repository.
    pub script: PathBuf,
    /// Where the large artifacts live (`build/d<depth>`: wasm, R1CS, proving key, verification key).
    /// Never in git; `circuits/README.md` says how to get them.
    pub data_dir: PathBuf,
    /// Levels of the transaction tree in the circuit (8 in production).
    pub depth: usize,
    /// Hard memory cap for each proving step, e.g. `2500M` (the prover peaks at 1.8 GiB).
    pub mem_max: String,
}

#[derive(Debug)]
pub struct Proof {
    pub bytes: ProofBytes,
    pub took: Duration,
}

impl Prover {
    /// Proves `input` and returns the proof bytes, after checking that the proof's public inputs are
    /// exactly the ones the Escrow will build (root, commitment, window).
    pub fn prove(&self, input: &CircuitInput, work: &Path) -> Result<Proof> {
        ensure!(
            self.script.is_file(),
            "prove script {} not found (--prove-script / ZOLAR_PROVE_SCRIPT)",
            self.script.display()
        );
        ensure!(
            self.data_dir
                .join(format!("build/d{}", self.depth))
                .is_dir(),
            "no circuit artifacts in {}/build/d{}; see circuits/README.md for how to get them \
             (--zolar-data / ZOLAR_DATA)",
            self.data_dir.display(),
            self.depth
        );
        std::fs::create_dir_all(work)?;
        // A run that was killed leaves its directory; a witness must not outlive it.
        for entry in std::fs::read_dir(work)?.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("claim-proof-")
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let dir = work.join(format!("claim-proof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let result = self.run(input, &dir);
        // The input names the transaction ID; do not leave it behind, whatever happened.
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn run(&self, input: &CircuitInput, dir: &Path) -> Result<Proof> {
        let input_path = dir.join("input.json");
        std::fs::write(&input_path, serde_json::to_vec(&input.json)?)?;
        std::fs::set_permissions(&input_path, std::fs::Permissions::from_mode(0o600))?;
        let out = dir.join("out");
        let started = Instant::now();
        let output = Command::new(&self.script)
            .arg(self.depth.to_string())
            .arg(&input_path)
            .arg(&out)
            .env("ZOLAR_DATA", &self.data_dir)
            .env("ZOLAR_MEM_MAX", &self.mem_max)
            .output()
            .with_context(|| format!("running {}", self.script.display()))?;
        if !output.status.success() {
            bail!(
                "proving failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
        let took = started.elapsed();
        let proof = std::fs::read_to_string(out.join("proof.json")).context("proof.json")?;
        let public = std::fs::read_to_string(out.join("public.json")).context("public.json")?;
        let got = public_from_snarkjs(&public)?;
        ensure!(
            got == input.public.to_bytes(),
            "the proof's public inputs are not the ones the Escrow will check; wrong circuit \
             artifacts?"
        );
        Ok(Proof {
            bytes: ProofBytes::from_snarkjs(&proof)?,
            took,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zolar_claim::private_claim::PublicInputs;

    const PROOF: &str = include_str!("../../circuits/fixtures/swap2.proof.json");
    const PUBLIC: &str = include_str!("../../circuits/fixtures/swap2.public.json");

    /// A stand-in for prove.sh: copies the recorded proof, and records what it was given.
    fn fake_script(dir: &Path, public: &str) -> PathBuf {
        std::fs::write(dir.join("proof.json"), PROOF).unwrap();
        std::fs::write(dir.join("public.json"), public).unwrap();
        let script = dir.join("prove.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nset -e\nmkdir -p \"$3\"\ncp '{d}/proof.json' \"$3/proof.json\"\ncp '{d}/public.json' \"$3/public.json\"\n\
                 echo \"$1 $ZOLAR_DATA $ZOLAR_MEM_MAX $(ls -ld \"$(dirname \"$2\")\" | cut -c1-10) $(wc -c < \"$2\")\" > '{d}/args.txt'\n",
                d = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn inputs_of(public: &str) -> CircuitInput {
        let p = public_from_snarkjs(public).unwrap();
        let num = |b: &[u8; 32]| u32::from_be_bytes(b[28..].try_into().unwrap());
        CircuitInput {
            json: json!({"txid": "secret"}),
            public: PublicInputs {
                root: p[0],
                commitment: p[1],
                lower: num(&p[2]),
                expiry: num(&p[3]),
            },
        }
    }

    fn setup(public: &str) -> (tempdir::Dir, Prover) {
        let dir = tempdir::Dir::new();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join("build/d8")).unwrap();
        let script = fake_script(dir.path(), public);
        let prover = Prover {
            script,
            data_dir: data,
            depth: 8,
            mem_max: "2500M".into(),
        };
        (dir, prover)
    }

    #[test]
    fn a_proof_is_returned_and_the_witness_file_is_private_and_removed() {
        let (dir, prover) = setup(PUBLIC);
        let work = dir.path().join("work");
        let proof = prover.prove(&inputs_of(PUBLIC), &work).unwrap();
        assert_eq!(proof.bytes, ProofBytes::from_snarkjs(PROOF).unwrap());
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.starts_with("8 "), "{args}");
        assert!(args.contains("2500M"), "{args}");
        assert!(args.contains("drwx------"), "private directory: {args}");
        assert_eq!(
            std::fs::read_dir(&work).unwrap().count(),
            0,
            "nothing of the witness is left in the work directory"
        );
    }

    #[test]
    fn the_witness_of_a_killed_earlier_run_is_removed() {
        let (dir, prover) = setup(PUBLIC);
        let work = dir.path().join("work");
        let stale = work.join("claim-proof-99999");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("input.json"), "secret").unwrap();
        prover.prove(&inputs_of(PUBLIC), &work).unwrap();
        assert!(!stale.exists());
    }

    #[test]
    fn a_proof_for_other_public_inputs_is_not_sent() {
        let (dir, prover) = setup(PUBLIC);
        let mut wrong = inputs_of(PUBLIC);
        wrong.public.expiry += 1;
        let err = prover.prove(&wrong, &dir.path().join("work")).unwrap_err();
        assert!(err.to_string().contains("public inputs"), "{err}");
    }

    #[test]
    fn missing_artifacts_say_where_to_get_them() {
        let (dir, mut prover) = setup(PUBLIC);
        prover.data_dir = dir.path().join("nowhere");
        let err = prover
            .prove(&inputs_of(PUBLIC), &dir.path().join("work"))
            .unwrap_err();
        assert!(err.to_string().contains("circuits/README.md"), "{err}");
    }

    mod tempdir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use rand::RngCore;
                let p = std::env::temp_dir().join(format!(
                    "zolar-prover-test-{:x}",
                    rand::thread_rng().next_u64()
                ));
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
