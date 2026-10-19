"""Run: python3 -I scripts/test_check_txid_absent.py"""
import base64
import importlib.util
import os
import unittest

spec = importlib.util.spec_from_file_location("c", os.path.join(os.path.dirname(__file__), "check_txid_absent.py"))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)

TXID = "fee9e219e5e1c48f5c619b6e411a2aafe675f5612e1d8544eba593731c20c75f"
DISPLAY = bytes.fromhex(TXID)
FILLER = bytes(range(200))


def search(raw=FILLER, js=None, parsed=None):
    return c.search_transaction(c.Needles(TXID), raw, js or {"a": "b"}, parsed or {"c": "d"})


class Search(unittest.TestCase):
    def test_a_clean_transaction_has_no_hits(self):
        self.assertEqual(search(), [])

    def test_raw_bytes_in_either_order(self):
        self.assertTrue(search(FILLER + DISPLAY))
        self.assertTrue(search(FILLER + DISPLAY[::-1] + FILLER))

    def test_a_truncated_copy_is_found(self):
        self.assertTrue(search(FILLER + DISPLAY[5:15]))

    def test_hex_in_logs_in_either_case_and_order(self):
        for text in (TXID, TXID.upper(), DISPLAY[::-1].hex()):
            self.assertTrue(search(js={"logMessages": [f"Program log: {text}"]}), text)

    def test_base58_and_base64_text(self):
        self.assertTrue(search(js={"m": c.b58encode(DISPLAY)}))
        self.assertTrue(search(js={"m": base64.b64encode(DISPLAY).decode()}))

    def test_instruction_data_hidden_in_a_base58_string(self):
        data = c.b58encode(bytes([7]) + FILLER + DISPLAY[::-1])
        self.assertTrue(search(parsed={"instructions": [{"data": data}]}))

    def test_base58_roundtrip_and_leading_zeros(self):
        for data in (b"\0\0abc", DISPLAY, bytes(32)):
            self.assertEqual(c.b58decode(c.b58encode(data)), data)

    def test_unrelated_data_is_not_a_hit(self):
        other = bytes.fromhex("11" * 32)
        self.assertEqual(search(FILLER + other, js={"m": other.hex()}), [])


if __name__ == "__main__":
    unittest.main()
