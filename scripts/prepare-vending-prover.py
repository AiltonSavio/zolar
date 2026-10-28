#!/usr/bin/env python3
"""Package tracked working source and the pinned public ELF; never wallet data."""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import shlex
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
EXTRAS = [
    "scripts/setup-vending-prover.sh", "scripts/prepare-vending-prover.py",
    "scripts/test_vending_preparation.py", "scripts/vending-paid-demo.json",
    "sdk/scripts/check-vending-settlement.mjs", "sdk/scripts/check-vending-settlement.test.mjs",
    "sdk/scripts/submit-vending-claim.mjs", "sdk/scripts/submit-vending-claim.test.mjs",
]


def allowed(name):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts:
        return False
    if any(part in {"research", ".zolar-work", ".git", "node_modules", "target", "dist"} for part in path.parts):
        return False
    if path.name in {"AGENTS.md", "CLAUDE.md", "wallet.json"}:
        return False
    if path.name.startswith(".env") and not path.name.endswith(".example"):
        return False
    return not any(word in path.name.lower() for word in [".seed", "keypair", "identity", "secret"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--elf", type=Path, default=ROOT / "crates/vending-guest/elf/vending-guest")
    args = parser.parse_args()
    demo = json.loads((ROOT / "scripts/vending-paid-demo.json").read_text())
    elf = args.elf.read_bytes()
    if hashlib.sha256(elf).hexdigest() != demo["guest_sha256"]:
        parser.error("guest ELF checksum differs from the paid Machine's pinned artifact")
    names = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    files = {}
    for name in sorted(set(names + EXTRAS)):
        if not name or not allowed(name):
            continue
        path = ROOT / name
        if path.is_symlink():
            parser.error(f"bundle refuses symlink: {name}")
        if path.is_file():
            files[name] = path.read_bytes()
    files["crates/vending-guest/elf/vending-guest"] = elf
    manifest = {
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source": "tracked working files plus named preparation tools; includes uncommitted edits",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["prover-bundle.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation prevents replacing a bundle that may already be on a rented host.
    with args.out.open("xb") as stream, tarfile.open(fileobj=stream, mode="w:gz") as archive:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755 if name.endswith(".sh") else 0o644
            archive.addfile(info, io.BytesIO(data))
    print(f"PASS: {len(files)} public source/artifact files; no ignored files or wallet directories")
    print(f"SHA256 {hashlib.sha256(args.out.read_bytes()).hexdigest()} {args.out}")
    print("Transfer this archive to a fresh directory on the rented host, then run:")
    print("bash scripts/setup-vending-prover.sh --run")
    print(f"Local archive: {shlex.quote(str(args.out.resolve()))}")


if __name__ == "__main__":
    main()
