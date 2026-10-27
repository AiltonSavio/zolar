use crate::{ProofEngine, ProofOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use vending_guest::Witness;

pub struct Sp1Engine {
    pub elf: PathBuf,
    pub prove: bool,
}

/// Check before creating a CPU prover. Execution always uses the light client instead.
pub fn check_proving_memory(meminfo: &str) -> Result<()> {
    fn kib(text: &str, key: &str) -> u64 {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
    if kib(meminfo, "MemTotal:") < 32_000_000 || kib(meminfo, "MemAvailable:") < 28_000_000 {
        bail!("Groth16 requires a 32 GB+ machine and 28 GB available; use --execute here");
    }
    Ok(())
}
#[async_trait]
impl ProofEngine for Sp1Engine {
    async fn run(&self, witness: Witness) -> Result<ProofOutput> {
        if self.prove {
            check_proving_memory(&std::fs::read_to_string("/proc/meminfo")?)?;
            if !cfg!(feature = "native-prove") {
                bail!("rebuild with --features native-prove for Docker-free Groth16");
            }
        }
        #[cfg(feature = "sp1")]
        {
            run_sp1(&self.elf, witness, self.prove).await
        }
        #[cfg(not(feature = "sp1"))]
        {
            let _ = witness;
            bail!("rebuild with --features sp1 for SP1 execution");
        }
    }
}

#[cfg(feature = "sp1")]
async fn run_sp1(path: &std::path::Path, witness: Witness, prove: bool) -> Result<ProofOutput> {
    use sp1_sdk::{Elf, HashableKey, ProveRequest, Prover, ProverClient, ProvingKey, SP1Stdin};
    let elf = Elf::from(std::fs::read(path)?);
    let expected = vending_guest::verify(&witness)
        .map_err(|e| anyhow::anyhow!("native statement: {e:?}"))?
        .encode();
    let mut stdin = SP1Stdin::new();
    stdin.write(&witness);
    let client = ProverClient::builder().light().build().await;
    let (pv, report) = client.execute(elf.clone(), stdin.clone()).await?;
    anyhow::ensure!(
        report.exit_code == 0,
        "SP1 guest exited with code {}",
        report.exit_code
    );
    anyhow::ensure!(
        pv.as_slice() == expected,
        "native/SP1 public values mismatch"
    );
    let cycles = report.total_instruction_count() + report.total_syscall_count();
    let pk = client.setup(elf.clone()).await?;
    let vkey_hash = Some(pk.verifying_key().bytes32());
    if !prove {
        return Ok(ProofOutput {
            mode: "execute".into(),
            proof: None,
            public_values: hex::encode(pv.as_slice()),
            cycles,
            vkey_hash,
        });
    }
    // Only reachable after the explicit memory and native backend gates above.
    let cpu = ProverClient::builder().cpu().build().await;
    let pk = cpu.setup(elf).await?;
    let proof = cpu.prove(&pk, stdin).groth16().await?;
    cpu.verify(&proof, pk.verifying_key(), None)?;
    anyhow::ensure!(
        proof.public_values.as_slice() == expected,
        "proof public values mismatch"
    );
    Ok(ProofOutput {
        mode: "groth16".into(),
        proof: Some(hex::encode(proof.bytes())),
        public_values: hex::encode(proof.public_values.as_slice()),
        cycles,
        vkey_hash,
    })
}
