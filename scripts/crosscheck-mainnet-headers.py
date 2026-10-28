#!/usr/bin/env python3
"""Cross-check an exported mainnet header file against the Blockchair block explorer API.

Usage: scripts/crosscheck-mainnet-headers.py <fixture.txt> [every-nth]

For the first and last header and every <every-nth> (default 10) header in between, fetches
height, hash, time and bits from https://api.blockchair.com/zcash/dashboards/blocks/<h,...> (10 heights
per request) and compares them with the values recomputed from the raw header (hash = SHA-256d,
reversed). Exits non-zero on any mismatch. Chain linkage of every header is checked too.
"""
import calendar, hashlib, json, struct, sys, time, urllib.request


def sha256d(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def main():
    path, nth = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 10
    rows = [(int(h), bytes.fromhex(r)) for h, r in (l.split() for l in open(path))]
    for (_, a), (_, b) in zip(rows, rows[1:]):
        assert b[4:36] == sha256d(a), "headers do not link"
    picks = sorted({0, len(rows) - 1, *range(0, len(rows), nth)})
    checked = 0
    for i in range(0, len(picks), 10):
        chunk = picks[i : i + 10]
        url = "https://api.blockchair.com/zcash/dashboards/blocks/" + ",".join(str(rows[j][0]) for j in chunk)
        data = json.load(urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "zolar-fixture-check"}), timeout=30))["data"]
        for j in chunk:
            height, raw = rows[j]
            got = data[str(height)]["block"]
            t, bits = struct.unpack("<II", raw[100:108])
            hash_hex = sha256d(raw)[::-1].hex()
            explorer_time = calendar.timegm(time.strptime(got["time"], "%Y-%m-%d %H:%M:%S"))
            assert got["hash"] == hash_hex, f"{height}: hash {got['hash']} != {hash_hex}"
            assert explorer_time == t, f"{height}: time {explorer_time} != {t}"
            assert int(got["bits"]) == bits, f"{height}: bits {got['bits']} != {bits}"
            checked += 1
        time.sleep(2)
    print(f"{checked} of {len(rows)} headers match Blockchair (hash, time, bits); all {len(rows)} link by SHA-256d")


main()
