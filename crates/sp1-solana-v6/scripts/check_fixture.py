#!/usr/bin/env python3
"""Check the pinned upstream bincode fixture and reproduce its SDK wire encoding.

No SDK build, network access, execution, or proving is needed.
"""
import hashlib
from pathlib import Path
import struct


def main():
    root = Path(__file__).resolve().parent.parent
    fixtures = root / "tests" / "fixtures"
    data = (fixtures / "upstream_sp1_proof.bin").read_bytes()
    assert hashlib.sha256(data).hexdigest() == (
        "9fb91cc514d6667ba0cd83d96be51d507867371c0bbbcd7119151f375eeb0a45"
    )
    offset = 0

    def read(size):
        nonlocal offset
        value = data[offset : offset + size]
        assert len(value) == size
        offset += size
        return value

    def blob():
        return read(struct.unpack("<Q", read(8))[0])

    # SP1Proof::Groth16, five decimal strings, encoded/raw proof, key hash,
    # SP1PublicValues { Buffer { data, skipped ptr } }, version, absent TEE.
    assert read(4) == struct.pack("<I", 3)
    inputs = [int(blob()) for _ in range(5)]
    encoded = bytes.fromhex(blob().decode("ascii"))
    blob()  # raw_proof is not part of SP1ProofWithPublicValues::bytes().
    key_hash = read(32)
    values = blob()
    assert blob() == b"v6.1.0"
    assert read(1) == b"\0"
    assert offset == len(data)
    vk = (root / "src" / "groth16_vk.bin").read_bytes()
    assert len(vk) == 492
    assert key_hash == hashlib.sha256(vk).digest()
    assert key_hash.hex() == (
        "4388a21c687fdd5f218d7e3d13190cac4c5355818d3605fd5fb811df468ee696"
    )
    assert len(encoded) == 352
    assert [int.from_bytes(encoded[i : i + 32], "big") for i in (0, 32, 64)] == inputs[2:]
    assert inputs[2] == 0
    assert inputs[3].to_bytes(32, "big").hex() == (
        "002f850ee998974d6cc00e50cd0814b098c05bfade466d28573240d057f25352"
    )
    assert inputs[1] == (
        int.from_bytes(hashlib.sha256(values).digest(), "big") & ((1 << 253) - 1)
    )
    assert key_hash[:4] + encoded == (fixtures / "proof.bin").read_bytes()
    assert values == (fixtures / "public_values.bin").read_bytes()
    assert inputs[0].to_bytes(32, "big") == (fixtures / "vkey_hash.bin").read_bytes()
    print("PASS: upstream fixture digest, v6.10 key, all five inputs, SDK encoding")


if __name__ == "__main__":
    main()
