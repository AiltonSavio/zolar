#!/usr/bin/env python3
"""Check that a Zcash transaction ID appears nowhere in the Solana transactions of a swap.

    scripts/check-txid-absent.sh [--url RPC] [--address ADDR ...] <txid> [signature ...]

<txid> is the 64 hex digits an explorer prints (display order). The script looks for it in every
transaction named by a signature, plus every transaction that touches an --address (use the Escrow,
its vault and the Payer's and Payee's token accounts: closed accounts keep their history).

For each transaction it fetches the raw bytes (base64), the `json` form with logs and inner
instructions, and the `jsonParsed` form that explorers render, and searches them for the ID in both
byte orders, as raw bytes, hex (either case), base58 and base64. Every string field is also decoded as
base58 and base64 and searched, so instruction data and account data are covered. Any 8-byte piece of
the ID in either order counts as a hit in raw bytes, so a truncated copy is found too.

Exit status: 0 only if at least one transaction was searched and nothing was found; 1 if the ID was
found anywhere; 2 on usage or RPC errors (a transaction that cannot be fetched is never "clean").
"""
import argparse
import base64
import json
import os
import re
import sys
import time
import urllib.request

ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
DEFAULT_URL = "https://api.devnet.solana.com"
PIECE = 8
NUMBER_RUN = re.compile(r"(?:\b\d{1,3}\b\s*,\s*){7,}\b\d{1,3}\b")
TOKEN = re.compile(r"[A-Za-z0-9+/=_-]{8,}")


def b58encode(data: bytes) -> str:
    n = int.from_bytes(data, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = ALPHABET[r] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def b58decode(text: str):
    n = 0
    for ch in text:
        i = ALPHABET.find(ch)
        if i < 0:
            return None
        n = n * 58 + i
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return b"\0" * (len(text) - len(text.lstrip("1"))) + raw


def b64decode(text: str):
    try:
        return base64.b64decode(text + "=" * (-len(text) % 4), validate=True)
    except Exception:
        return None


class Needles:
    """Everything the transaction ID can look like."""

    def __init__(self, display_hex: str):
        display = bytes.fromhex(display_hex)
        if len(display) != 32:
            raise ValueError("a transaction ID is 32 bytes (64 hex digits)")
        self.orders = {"display": display, "internal": display[::-1]}
        self.pieces = {
            f"{name}[{i}:{i + PIECE}]": order[i : i + PIECE]
            for name, order in self.orders.items()
            for i in range(0, 32 - PIECE + 1)
        }
        self.texts = {}
        self.fragment_hex = {
            f"{name} hex piece [{i}:{i + PIECE}]": order[i : i + PIECE].hex()
            for name, order in self.orders.items()
            for i in range(0, 32 - PIECE + 1)
        }
        for name, order in self.orders.items():
            self.texts[f"{name} hex"] = order.hex()
            self.texts[f"{name} base58"] = b58encode(order)
            self.texts[f"{name} base64"] = base64.b64encode(order).decode().rstrip("=")

    def in_bytes(self, data: bytes):
        hits = []
        for name, order in self.orders.items():
            if order in data:
                hits.append(f"raw bytes ({name} order)")
        for name, piece in self.pieces.items():
            if piece in data:
                hits.append(f"raw {PIECE}-byte piece {name}")
        return hits

    def in_text(self, text: str):
        lower = text.lower()
        hits = []
        for name, needle in self.texts.items():
            haystack = lower if name.endswith("hex") else text
            if (needle.lower() if name.endswith("hex") else needle) in haystack:
                hits.append(f"text ({name})")
        for name, needle in self.fragment_hex.items():
            if needle in lower:
                hits.append(f"text ({name})")
        for run in NUMBER_RUN.finditer(text):
            data = bytes(int(n) for n in re.findall(r"\d+", run.group()) if int(n) < 256)
            hits += [f"{h} as a list of decimal numbers" for h in self.in_bytes(data)]
        return hits


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for k, v in value.items():
            yield str(k)
            yield from strings(v)
    elif isinstance(value, list):
        for v in value:
            yield from strings(v)


def search_json(needles: Needles, value):
    """Hits in a JSON value: its text as is, and every string decoded as base58 and as base64."""
    hits = needles.in_text(json.dumps(value))
    for s in strings(value):
        hits += needles.in_text(s)
        # The whole string, and every word of it (a log line is "Program data: <base64>").
        for candidate in [s] + TOKEN.findall(s):
            if len(candidate) < 8:
                continue
            for decode in (b58decode, b64decode):
                data = decode(candidate)
                if data:
                    hits += [f"{h} in decoded string" for h in needles.in_bytes(data)]
                    # Text hidden in bytes, e.g. the hex digits of the ID as instruction data.
                    hits += [f"{h} in decoded string" for h in needles.in_text(data.decode("latin-1"))]
    return sorted(set(hits))


def search_transaction(needles: Needles, raw: bytes, json_form, parsed_form):
    hits = [f"{h} in the transaction" for h in needles.in_bytes(raw)]
    hits += [f"{h} in json form" for h in search_json(needles, json_form)]
    hits += [f"{h} in jsonParsed form" for h in search_json(needles, parsed_form)]
    return sorted(set(hits))


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    for attempt in range(8):
        try:
            req = urllib.request.Request(url, body, {"Content-Type": "application/json"})
            with urllib.request.urlopen(req, timeout=60) as r:
                answer = json.load(r)
            if "error" in answer:
                raise RuntimeError(answer["error"])
            return answer["result"]
        except Exception as e:  # rate limits and timeouts: wait and retry
            if attempt == 7:
                raise RuntimeError(f"{method} failed: {e}")
            time.sleep(2 + attempt * 2)


def signatures_of(url, address):
    out, before = [], None
    while True:
        opts = {"limit": 1000}
        if before:
            opts["before"] = before
        page = rpc(url, "getSignaturesForAddress", [address, opts])
        out += [p["signature"] for p in page]
        if len(page) < 1000:
            return out
        before = page[-1]["signature"]


def fetch(url, signature):
    cfg = {"commitment": "confirmed", "maxSupportedTransactionVersion": 0}
    raw = rpc(url, "getTransaction", [signature, {**cfg, "encoding": "base64"}])
    if raw is None:
        raise RuntimeError(f"{signature}: the RPC node does not have this transaction")
    as_json = rpc(url, "getTransaction", [signature, {**cfg, "encoding": "json"}])
    parsed = rpc(url, "getTransaction", [signature, {**cfg, "encoding": "jsonParsed"}])
    for name, form in (("json", as_json), ("jsonParsed", parsed)):
        if not form or form.get("meta") is None:
            raise RuntimeError(f"{signature}: the RPC node gave no {name} form with logs; cannot call it clean")
    return base64.b64decode(raw["transaction"][0]), as_json, parsed


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--url", default=os.environ.get("ZOLAR_RPC_URL", DEFAULT_URL))
    ap.add_argument("--address", action="append", default=[], help="also check every transaction touching this account")
    ap.add_argument("txid")
    ap.add_argument("signatures", nargs="*")
    args = ap.parse_args(argv)
    try:
        needles = Needles(args.txid.strip())
        signatures = list(dict.fromkeys(args.signatures))
        for address in args.address:
            for s in signatures_of(args.url, address):
                if s not in signatures:
                    signatures.append(s)
        if not signatures:
            print("no transactions to check", file=sys.stderr)
            return 2
        found = False
        for signature in signatures:
            raw, as_json, parsed = fetch(args.url, signature)
            hits = search_transaction(needles, raw, as_json, parsed)
            size = f"{len(raw)} raw bytes, {len(json.dumps(as_json))} json chars, {len(json.dumps(parsed))} parsed chars"
            if hits:
                found = True
                print(f"FOUND  {signature}  ({size})")
                pieces = [h for h in hits if "-byte piece" in h]
                for h in hits:
                    if "-byte piece" not in h:
                        print(f"         {h}")
                if pieces:
                    print(f"         {len(pieces)} matches of {PIECE}-byte pieces of the ID")
            else:
                print(f"clean  {signature}  ({size})")
        print(
            f"{len(signatures)} transactions searched for {args.txid}: "
            + ("THE TRANSACTION ID APPEARS ON SOLANA"
            if found
            else "no form of the transaction ID found in them")
        )
        return 1 if found else 0
    except (ValueError, RuntimeError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
