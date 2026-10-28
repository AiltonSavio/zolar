#!/usr/bin/env python3
"""Catch the syscall import that devnet rejected during the vending deployment."""
import pathlib
import sys

artifact = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "programs/vending/target/deploy/vending.so")
data = artifact.read_bytes()
if not data.startswith(b"\x7fELF"):
    raise SystemExit("FAIL: build the vending SBF ELF first")
if b"sol_blake3\0" in data:
    raise SystemExit("FAIL: sol_blake3 is unavailable on devnet; use portable blake3 in the SP1 verifier")
print(f"PASS: {artifact}: no unavailable sol_blake3 import")
