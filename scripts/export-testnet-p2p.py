#!/usr/bin/env python3
"""Export consecutive raw Zcash testnet block headers from public P2P peers (getheaders).

Usage: scripts/export-testnet-p2p.py <locator-height> <locator-hash> <count> <out-file> [peer-host ...]

<locator-hash> is the block hash as explorers print it (big-endian hex) of a block on the chain you
want; the export starts at <locator-height>+1 and writes <count> headers, one `<height> <hex>` line
each (hex = 1,487 bytes: 140 header + `fd4005` + 1,344-byte Equihash solution). Every header is checked
to link to the previous one by SHA-256d, so a peer cannot hand over a different chain unnoticed.
One connection is reused for all requests; peers drop clients that reconnect repeatedly.
Default peers are the testnet DNS seeders; only peers that speak protocol >= 170180 (NU7) are useful.
"""
import hashlib, random, socket, struct, sys, time

MAGIC = bytes.fromhex("fa1af9bf")
PROTOCOL = 170180
HEADER_LEN = 1487
SEEDERS = ["testnet.seeder.zfnd.org", "seeder.testnet.zec.rocks", "dnsseed.testnet.z.cash"]


def sha256d(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def varint(n):
    return bytes([n]) if n < 253 else b"\xfd" + struct.pack("<H", n)


def message(cmd, payload):
    return MAGIC + cmd.encode().ljust(12, b"\0") + struct.pack("<I", len(payload)) + sha256d(payload)[:4] + payload


def netaddr():
    return struct.pack("<Q", 0) + b"\0" * 10 + b"\xff\xff" + bytes(4) + struct.pack(">H", 0)


def recv_exact(s, n):
    out = b""
    while len(out) < n:
        chunk = s.recv(n - len(out))
        if not chunk:
            raise EOFError
        out += chunk
    return out


def recv_message(s):
    head = recv_exact(s, 24)
    return head[4:16].rstrip(b"\0").decode(), recv_exact(s, struct.unpack("<I", head[16:20])[0])


def connect(host):
    s = socket.create_connection((host, 18233), timeout=15)
    s.settimeout(30)
    ua = b"/zolar-fixture-export:0.1/"
    s.sendall(message("version", struct.pack("<iQq", PROTOCOL, 0, int(time.time())) + netaddr() + netaddr()
                      + struct.pack("<Q", random.getrandbits(64)) + varint(len(ua)) + ua + struct.pack("<i", 0) + b"\0"))
    verack = peer_version = False
    while not (verack and peer_version):
        cmd, payload = recv_message(s)
        if cmd == "version":
            peer_version = struct.unpack("<i", payload[:4])[0]
            s.sendall(message("verack", b""))
        elif cmd == "verack":
            verack = True
    if peer_version < PROTOCOL:
        raise RuntimeError(f"{host} speaks {peer_version}, older than NU7 ({PROTOCOL})")
    return s


def getheaders(s, locator_internal):
    s.sendall(message("getheaders", struct.pack("<i", PROTOCOL) + varint(1) + locator_internal + bytes(32)))
    for _ in range(60):
        cmd, payload = recv_message(s)
        if cmd == "headers":
            n, off = (payload[0], 1) if payload[0] < 253 else (struct.unpack("<H", payload[1:3])[0], 3)
            return [payload[off + i * (HEADER_LEN + 1): off + i * (HEADER_LEN + 1) + HEADER_LEN] for i in range(n)]
        if cmd == "ping":
            s.sendall(message("pong", payload))
    raise TimeoutError("no headers reply")


def main():
    height, locator_hash, count, out = int(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4]
    peers = sys.argv[5:] or SEEDERS
    candidates = []
    for p in peers:
        try:
            candidates += [ai[4][0] for ai in socket.getaddrinfo(p, 18233, socket.AF_INET, socket.SOCK_STREAM)]
        except socket.gaierror:
            candidates.append(p)
    random.shuffle(candidates)
    last = bytes.fromhex(locator_hash)[::-1]
    rows = []
    for host in candidates:
        try:
            s = connect(host)
            while len(rows) < count:
                got = getheaders(s, last)
                if not got:
                    break
                for raw in got:
                    if raw[4:36] != last:
                        raise RuntimeError("headers do not link to the previous block")
                    last = sha256d(raw)
                    height += 1
                    rows.append((height, raw))
            if len(rows) >= count:
                print(f"peer {host}", file=sys.stderr)
                break
        except (OSError, EOFError, RuntimeError, TimeoutError) as e:
            print(f"{host}: {e}", file=sys.stderr)
    if len(rows) < count:
        sys.exit(f"only {len(rows)} headers exported")
    with open(out, "w") as f:
        for h, raw in rows[:count]:
            f.write(f"{h} {raw.hex()}\n")
    print(f"wrote {count} headers {rows[0][0]}..{rows[count-1][0]} to {out}")


main()
