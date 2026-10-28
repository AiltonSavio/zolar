#!/usr/bin/env python3
"""Coordinate source defaults before regenerating clients and building a devnet deployment."""
import argparse
import pathlib
import re

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("program_id", help="public address from solana-keygen pubkey")
args = parser.parse_args()
alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
number = 0
try:
    for char in args.program_id:
        number = number * 58 + alphabet.index(char)
    raw = b"\0" * (len(args.program_id) - len(args.program_id.lstrip("1")))
    raw += number.to_bytes((number.bit_length() + 7) // 8, "big")
except ValueError:
    parser.error("invalid base58 program address")
if len(raw) != 32:
    parser.error("program address must decode to 32 bytes")

root = pathlib.Path(__file__).resolve().parent.parent
changes = [
    ("clients/codama/vending.mjs", r"(export const VENDING_PROGRAM_ID\s*=\s*)'[^']+'", rf"\g<1>'{args.program_id}'"),
    ("crates/vending-prover/src/backend.rs", r'(pub const VENDING_PROGRAM_ID: &str = )"[^"]+"', rf'\g<1>"{args.program_id}"'),
    ("shop/src/config.ts", r"(program: env.VITE_VENDING_PROGRAM \|\| )'[^']*'", rf"\g<1>'{args.program_id}'"),
    ("programs/vending/src/lib.rs", r"pub const ID: pinocchio::Address = pinocchio::Address::new_from_array\(\[[\s\S]*?\]\);", f"pub const ID: pinocchio::Address = pinocchio::Address::new_from_array({list(raw)});"),
]
prepared = []
for name, pattern, replacement in changes:
    path = root / name
    updated, count = re.subn(pattern, replacement, path.read_text())
    if count != 1:
        raise SystemExit(f"FAIL: expected exactly one program default in {name}; no files changed")
    prepared.append((path, updated))
for path, updated in prepared:
    path.write_text(updated)
print(f"PASS: source defaults set to {args.program_id}; regenerate Codama clients, format Rust and rebuild")
