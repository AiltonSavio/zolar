#!/usr/bin/env python3
"""Compare SP1 public values with finalized devnet Machine, Reservation and Relay accounts.

This authenticates the public statement and recent root, not a Groth16 proof or Claim.
"""
import argparse
import base64
import hashlib
import json
import pathlib
import re
import struct
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--result", required=True, type=pathlib.Path)
parser.add_argument("--machine", required=True)
parser.add_argument("--reservation", required=True)
parser.add_argument("--rpc", default="https://api.devnet.solana.com")
parser.add_argument("--snapshot-dir", required=True, type=pathlib.Path)
parser.add_argument("--require-proof", action="store_true", help="Reject execute-only output before settlement")
args = parser.parse_args()
result = json.loads(args.result.read_text())
if args.require_proof:
    if result.get("mode") != "groth16":
        raise SystemExit("a Groth16 Payment Proof is required; execution is not settlement")
    proof = result.get("proof")
    if not isinstance(proof, str) or not re.fullmatch(r"(?:[0-9a-fA-F]{712}|[0-9a-fA-F]{776})", proof):
        raise SystemExit("missing or malformed SP1 proof")
args.snapshot_dir.mkdir(parents=True, exist_ok=True)


def rpc(method, params):
    request = urllib.request.Request(args.rpc, data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        result = json.load(response)
    if "error" in result:
        raise SystemExit(f"RPC error: {result['error']}")
    return result


def b58decode(value):
    number = 0
    for char in value:
        number = number * 58 + "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".index(char)
    return number.to_bytes(32, "big")


def b58encode(value):
    number = int.from_bytes(value, "big")
    text = ""
    while number:
        number, digit = divmod(number, 58)
        text = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"[digit] + text
    return "1" * (len(value) - len(value.lstrip(b"\0"))) + text


def account(address, name, owner, length):
    result = rpc("getAccountInfo", [address, {"encoding": "base64", "commitment": "finalized"}])
    (args.snapshot_dir / f"{name}.json").write_text(json.dumps(result, indent=2))
    value = result["result"]["value"]
    assert value and value["owner"] == owner and not value["executable"], f"wrong {name} account"
    data = base64.b64decode(value["data"][0])
    assert len(data) == length, f"wrong {name} layout"
    return data


assert rpc("getGenesisHash", [])["result"] == "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG", "Solana devnet only"
definition = (pathlib.Path(__file__).resolve().parent.parent / "clients/codama/vending.mjs").read_text()
program = re.search(r"export const VENDING_PROGRAM_ID\s*=\s*'([^']+)'", definition)[1]
machine = account(args.machine, "machine", program, 291)
reservation = account(args.reservation, "reservation", program, 128)
relay = account(b58encode(machine[100:132]), "relay", "6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D", 5914)
assert machine[:2] == bytes([1, 1]) and reservation[:2] == bytes([2, 1])
assert relay[:4] == bytes([1, 1, 1, 0]) and relay[95] == 1, "running Zcash-testnet Relay required"
values = bytes.fromhex(result["public_values"])
assert len(values) == 249 and values[0] == 1 and values[188] == 3 and not any(values[221:])
assert struct.unpack_from("<I", values, 1)[0] == 0x77190AD9
assert values[5:37] == b58decode(args.machine) == reservation[3:35]
assert values[37:69] == reservation[35:67], "Reservation nonce differs"
assert values[189:221] == reservation[67:99], "Reservation payout differs"
amount = struct.unpack_from("<Q", values, 69)[0]
assert amount >= struct.unpack_from("<Q", reservation, 99)[0] == struct.unpack_from("<Q", machine, 132)[0]
height = struct.unpack_from("<I", values, 141)[0]
assert height <= struct.unpack_from("<I", reservation, 107)[0]
assert height <= struct.unpack_from("<I", relay, 1050)[0]
assert values[145:156] == machine[148:159], "shop diversifier differs"
assert values[156:188] == machine[159:191] == hashlib.sha256(machine[227:291]).digest()
assert bytes.fromhex(result["vkey_hash"].removeprefix("0x")) == machine[191:223], "guest vkey differs"
assert values[109:141] in [relay[i:i + 32] for i in range(1818, 5914, 32)], "root aged out of live Relay"
assert result["mode"] in ["execute", "groth16"]
if result["mode"] == "execute":
    assert result["proof"] is None, "execution must not be represented as a proof"
assert reservation[119] in [0, 1], "invalid release state"
if reservation[119]:
    raise SystemExit("Release has been requested; reconcile its Claim window before submitting")
checks = {"machine": args.machine, "reservation": args.reservation, "nonce": b58encode(values[37:69]), "payout": b58encode(values[189:221]), "value_zat": amount, "height": height, "root": values[109:141].hex(), "relay_tip": struct.unpack_from("<I", relay, 36)[0], "relay_finalized": struct.unpack_from("<I", relay, 1050)[0], "root_known": True, "mode": result["mode"], "cycles": result["cycles"], "vkey_hash": result["vkey_hash"]}
(args.snapshot_dir / "checks.json").write_text(json.dumps(checks, indent=2) + "\n")
print("PASS: machine, nonce, payout, price, height window, SVK hash, diversifier, NU7 branch, guest vkey and live Relay root")
print(json.dumps(checks, indent=2))
