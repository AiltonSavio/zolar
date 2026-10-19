#!/usr/bin/env python3
"""Record what a live peer sends during one handshake and one getheaders request.

Usage: scripts/record-p2p-transcript.py [--mainnet] <locator-hash> <out-file> [peer-host ...]

Only the bytes the peer sends are kept (the relayer tests replay them as the read side of a
connection). Output for mainnet: crates/relayer/fixtures/p2p-transcript-mainnet.bin.
"""
import importlib.util, os, random, socket, sys

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("exp", os.path.join(here, "export-testnet-p2p.py"))
exp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exp)


class Tap:
    """A socket that keeps every byte the peer sends."""

    def __init__(self, s):
        self.s, self.log = s, bytearray()

    def settimeout(self, t):
        self.s.settimeout(t)

    def sendall(self, b):
        self.s.sendall(b)

    def recv(self, n):
        chunk = self.s.recv(n)
        self.log += chunk
        return chunk

    def close(self):
        self.s.close()


def main():
    if sys.argv[1] == "--mainnet":
        del sys.argv[1]
        exp.MAGIC, exp.PORT, exp.MIN_PEER_PROTOCOL, exp.SEEDERS = (
            exp.MAINNET_MAGIC, 8233, exp.MAINNET_MIN_PEER_PROTOCOL, exp.MAINNET_SEEDERS)
    locator = bytes.fromhex(sys.argv[1])[::-1]
    out = sys.argv[2]
    hosts = sys.argv[3:] or exp.SEEDERS
    candidates = []
    for h in hosts:
        try:
            candidates += [ai[4][0] for ai in socket.getaddrinfo(h, exp.PORT, socket.AF_INET, socket.SOCK_STREAM)]
        except socket.gaierror:
            candidates.append(h)
    random.shuffle(candidates)
    real = socket.create_connection
    for host in candidates:
        taps = []
        socket.create_connection = lambda *a, **k: taps.append(Tap(real(*a, **k))) or taps[-1]
        try:
            s = exp.connect(host)
            got = exp.getheaders(s, locator)
            if len(got) < 100:
                raise RuntimeError("short reply")
            open(out, "wb").write(bytes(taps[0].log))
            print(f"peer {host}: {len(got)} headers, {len(taps[0].log)} bytes", file=sys.stderr)
            return
        except (OSError, EOFError, RuntimeError, TimeoutError) as e:
            print(f"{host}: {e}", file=sys.stderr)
    sys.exit("no peer answered")


main()
