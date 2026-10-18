#!/usr/bin/env python3
"""Generate crates/zcash-header/fixtures/difficulty-vectors.json with an independent Python
model of Zcash's difficulty rule (Python big integers, the same algorithm as Zebra's
`zebra-state/src/service/check/difficulty.rs`). Synthetic contexts cover what the live
chain never shows: minimum-difficulty blocks, the 256-bit overflow of the mean, both clamps
and mainnet. Fixed seed, so the output is reproducible.

Usage: scripts/gen-difficulty-vectors.py > crates/zcash-header/fixtures/difficulty-vectors.json
"""
import json, random

LIMIT = {"mainnet": (1 << 243) - 1, "testnet": (1 << 251) - 1}


def to_target(c):
    exp, m = c >> 24, c & 0x7FFFFF
    return m << (8 * (exp - 3)) if exp >= 3 else m >> (8 * (3 - exp))


def to_compact(t):
    size = (t.bit_length() + 7) // 8
    m = t << (8 * (3 - size)) if size <= 3 else t >> (8 * (size - 3))
    if m & 0x800000:
        m >>= 8
        size += 1
    return m | (size << 24)


def median(l):
    return sorted(l)[len(l) // 2]


def expected(net, nu7, height, time, times, bits):
    nu = height >= nu7
    w, s = (102, 25) if nu else (17, 75)
    limit = to_target(to_compact(LIMIT[net]))
    if net == "testnet" and height >= 299188 and time - times[0] > 450:
        return to_compact(limit)
    mean = sum(to_target(b) for b in bits[:w]) // w
    span = w * s
    actual = median(times[:11]) - median(times[w : w + 11])
    d = actual - span
    d = abs(d) // 4 * (1 if d >= 0 else -1)
    bounded = max(span * 84 // 100, min(span * 132 // 100, span + d))
    return to_compact(min(limit, mean // span * bounded))


def case(name, net, nu7, height, time, times, bits):
    return dict(name=name, network=net, nu7_height=nu7, height=height, time=time, times=times,
                bits=bits, expected_bits=expected(net, nu7, height, time, times, bits))


def context(rng, n, step, start_bits, jitter):
    # newest first; times strictly decrease by about `step` seconds
    times, t = [], 1_800_000_000
    for _ in range(n):
        times.append(t)
        t -= max(1, step + rng.randint(-jitter, jitter))
    return times, list(start_bits)


def main():
    rng = random.Random(20261019)
    cases = []
    nu7 = 4_465_026
    for h, label in ((nu7 + 500, "nu7"), (nu7 - 500, "pre-nu7")):
        step = 25 if h >= nu7 else 75
        bits = [0x1F0162F6 + rng.randint(0, 0xFFF) for _ in range(113)]
        times, _ = context(rng, 113, step, bits, 10)
        cases.append(case(f"{label}: on schedule", "testnet", nu7, h, times[0] + step, times, bits))
        for gap in (450, 451):
            cases.append(case(f"{label}: gap {gap}", "testnet", nu7, h, times[0] + gap, times, bits))
        slow, _ = context(rng, 113, step * 3, bits, 5)
        cases.append(case(f"{label}: slow chain hits the +32% clamp", "testnet", nu7, h, slow[0] + step, slow, bits))
        fast, _ = context(rng, 113, 1, bits, 0)
        cases.append(case(f"{label}: fast chain hits the -16% clamp", "testnet", nu7, h, fast[0] + 1, fast, bits))
        # a minimum-difficulty block inside the window stays in the mean
        mixed = list(bits)
        mixed[0] = 0x2007FFFF
        cases.append(case(f"{label}: minimum-difficulty parent in the mean", "testnet", nu7, h, times[0] + step, times, mixed))
        # every block at the limit: 102 x (2^251 - 1) overflows 256 bits
        limit_bits = [0x2007FFFF] * 113
        cases.append(case(f"{label}: all blocks at the testnet limit", "testnet", nu7, h, times[0] + step, times, limit_bits))
        cases.append(case(f"{label}: slow, all at the limit (result capped)", "testnet", nu7, h, slow[0] + step, slow, limit_bits))
    # not scheduled: always the 17-block window
    bits = [0x1B7FBC0E + rng.randint(0, 0xFFF) for _ in range(113)]
    times, _ = context(rng, 113, 75, bits, 20)
    cases.append(case("mainnet: no minimum difficulty however late", "mainnet", 2**32 - 1, 3_522_000, times[0] + 5000, times, bits))
    cases.append(case("mainnet: on schedule", "mainnet", 2**32 - 1, 3_522_000, times[0] + 75, times, bits))
    cases.append(case("testnet below the minimum-difficulty height", "testnet", 2**32 - 1, 299_187, times[0] + 5000, times, bits))
    # odd remainders: mean of targets that do not divide evenly
    bits = [0x1D00FFFF - rng.randint(0, 0x3FF) * 3 for _ in range(113)]
    times, _ = context(rng, 113, 75, bits, 40)
    cases.append(case("remainders: 17-block mean with odd targets", "testnet", 2**32 - 1, 4_000_000, times[0] + 75, times, bits))
    cases.append(case("remainders: 102-block mean with odd targets", "testnet", 0, 4_000_000, times[0] + 25, times, bits))
    print(json.dumps(cases, indent=1))


main()
