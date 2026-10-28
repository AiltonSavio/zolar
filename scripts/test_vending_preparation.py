"""Run with python3 -I scripts/test_vending_preparation.py; no installs or SSH."""
import importlib.util
import json
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]


class PreparationTests(unittest.TestCase):
    def test_settlement_gate_rejects_execution_without_contacting_rpc(self):
        with tempfile.TemporaryDirectory() as directory:
            result_path = pathlib.Path(directory) / "result.json"
            for result, message in [
                ({"mode": "execute", "proof": None}, "Groth16 Payment Proof is required"),
                ({"mode": "groth16", "proof": "zz" * 356}, "malformed SP1 proof"),
            ]:
                result_path.write_text(json.dumps(result))
                run = subprocess.run([
                    "python3", "-I", str(ROOT / "scripts/check-vending-execution.py"),
                    "--result", str(result_path), "--machine", "unused", "--reservation", "unused",
                    "--snapshot-dir", str(pathlib.Path(directory) / "snapshot"),
                    "--require-proof", "--rpc", "http://127.0.0.1:1",
                ], capture_output=True, text=True)
                self.assertNotEqual(run.returncode, 0)
                self.assertIn(message, run.stderr)
                self.assertFalse((pathlib.Path(directory) / "snapshot").exists())

    def test_plan_cannot_install_build_prove_or_connect(self):
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/setup-vending-prover.sh"), "--plan"],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("v6.10.0", result.stdout)
        self.assertIn("28,000,000", result.stdout)
        self.assertIn("--prove", result.stdout)
        self.assertIn("no Claim", result.stdout)

    def test_bundle_selection_excludes_private_and_ignored_files(self):
        spec = importlib.util.spec_from_file_location("prepare", ROOT / "scripts/prepare-vending-prover.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for path in ["AGENTS.md", ".env.local", "wallet.json", "shop.seed", "research/notes/a.md", "x/../wallet.json"]:
            with self.subTest(path=path):
                self.assertFalse(module.allowed(path))
        for path in ["Cargo.toml", "crates/vending-prover/src/engine.rs", "scripts/setup-vending-prover.sh"]:
            self.assertTrue(module.allowed(path))

    def test_bundle_rejects_wrong_guest_before_writing_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            out = pathlib.Path(directory) / "bundle.tgz"
            guest = pathlib.Path(directory) / "guest"
            guest.write_bytes(b"wrong elf")
            result = subprocess.run(
                ["python3", "-I", str(ROOT / "scripts/prepare-vending-prover.py"),
                 "--elf", str(guest), "--out", str(out)], capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("guest ELF checksum", result.stderr)
            self.assertFalse(out.exists())

    def test_bundle_records_working_edits_and_never_packs_untracked_wallet_data(self):
        spec = importlib.util.spec_from_file_location("prepare", ROOT / "scripts/prepare-vending-prover.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "scripts").mkdir()
            guest = root / "public-guest"
            guest.write_bytes(b"fixture elf")
            import hashlib
            checksum = hashlib.sha256(guest.read_bytes()).hexdigest()
            (root / "scripts/vending-paid-demo.json").write_text(json.dumps({"guest_sha256": checksum}))
            (root / "Cargo.toml").write_text("working source edit")
            (root / "wallet.json").write_text("must never be read or transferred")
            (root / "AGENTS.md").write_text("internal")
            (root / "untracked.rs").write_text("unrelated session work")
            out = root / "bundle.tgz"
            with patch.object(module, "ROOT", root), patch.object(sys, "argv", ["prepare", "--elf", str(guest), "--out", str(out)]), patch.object(module.subprocess, "check_output", side_effect=[b"Cargo.toml\0wallet.json\0AGENTS.md\0", "abc123\n"]):
                module.main()
            with tarfile.open(out) as archive:
                self.assertEqual(archive.extractfile("Cargo.toml").read(), b"working source edit")
                for name in ["wallet.json", "AGENTS.md", "untracked.rs"]:
                    self.assertNotIn(name, archive.getnames())
                manifest = json.load(archive.extractfile("prover-bundle.json"))
                for name, expected in manifest["sha256"].items():
                    self.assertEqual(hashlib.sha256(archive.extractfile(name).read()).hexdigest(), expected)


if __name__ == "__main__":
    unittest.main()
