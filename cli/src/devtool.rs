//! zcash-devtool as a subprocess: the Payer's wallet stays in the tool that already knows how to
//! prove, sign and send Ironwood transactions. Keys are passed by path and never read here.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

pub struct Devtool {
    pub binary: PathBuf,
    pub wallet_dir: PathBuf,
    /// The age identity file that decrypts the wallet seed.
    pub identity: PathBuf,
}

impl Devtool {
    fn run(&self, args: &[&std::ffi::OsStr]) -> Result<String> {
        let output = Command::new(&self.binary)
            .args(args)
            .output()
            .with_context(|| format!("running {}", self.binary.display()))?;
        if !output.status.success() {
            bail!(
                "zcash-devtool {:?} failed: {}",
                args.first().unwrap_or(&std::ffi::OsStr::new("")),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn pczt(&self, rest: &[&std::ffi::OsStr]) -> Result<String> {
        let wallet = self.wallet_dir.as_os_str();
        let mut args: Vec<&std::ffi::OsStr> = vec!["pczt".as_ref(), "-w".as_ref(), wallet];
        args.extend_from_slice(rest);
        self.run(&args)
    }

    pub fn sync(&self) -> Result<()> {
        self.run(&[
            "wallet".as_ref(),
            "-w".as_ref(),
            self.wallet_dir.as_os_str(),
            "sync".as_ref(),
        ])?;
        Ok(())
    }

    /// Builds an unsigned PCZT paying `value` zatoshis to `address`, valid until block `expiry_height`.
    /// Needs the zcash-devtool built with `--expiry-height` (the patch kept next to the CLI).
    pub fn create(
        &self,
        address: &str,
        value: u64,
        memo: &str,
        expiry_height: u32,
        out: &Path,
    ) -> Result<()> {
        let value = value.to_string();
        let expiry = expiry_height.to_string();
        self.pczt(&[
            "create".as_ref(),
            "--address".as_ref(),
            address.as_ref(),
            "--value".as_ref(),
            value.as_ref(),
            "--memo".as_ref(),
            memo.as_ref(),
            "--target-note-count".as_ref(),
            "1".as_ref(),
            "--expiry-height".as_ref(),
            expiry.as_ref(),
            "--output".as_ref(),
            out.as_os_str(),
        ])?;
        Ok(())
    }

    pub fn prove(&self, input: &Path, out: &Path) -> Result<()> {
        self.pczt(&[
            "prove".as_ref(),
            input.as_os_str(),
            "--output".as_ref(),
            out.as_os_str(),
        ])?;
        Ok(())
    }

    pub fn sign(&self, input: &Path, out: &Path) -> Result<()> {
        self.pczt(&[
            "sign".as_ref(),
            "-i".as_ref(),
            self.identity.as_os_str(),
            input.as_os_str(),
            "--output".as_ref(),
            out.as_os_str(),
        ])?;
        Ok(())
    }

    pub fn combine(&self, proven: &Path, signed: &Path, out: &Path) -> Result<()> {
        let output = Command::new(&self.binary)
            .args(["pczt", "combine", "-i"])
            .arg(proven)
            .arg("-i")
            .arg(signed)
            .arg("-o")
            .arg(out)
            .output()
            .context("running zcash-devtool combine")?;
        if !output.status.success() {
            bail!(
                "zcash-devtool combine failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    pub fn send(&self, combined: &Path) -> Result<()> {
        self.pczt(&["send".as_ref(), combined.as_os_str()])?;
        Ok(())
    }
}
