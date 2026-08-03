"""Golden-parity smoke for the spoke-connect Python binding.

Reproduces the Rust golden vectors through the generated uniffi surface,
loading the shared cross-language golden vector from the SSOT
(`crates/spoke-connect/tests/fixtures/golden-hello.json`). Run from the
repository root:

    PYTHONPATH=crates/spoke-connect/bindings/python python3 -m unittest discover -s crates/spoke-connect/bindings/python/Smoke -v
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

import spoke_connect

# Shared golden vector SSOT (single cross-language source of truth). The
# fixture carries the seed / nonce / manifest inputs AND the pinned output
# bytes (pubkey, peer id, JCS hex, signature) — asserted below, never
# recomputed and written back.
FIXTURE_PATH = (
    Path(__file__).resolve().parents[3]
    / "tests"
    / "fixtures"
    / "golden-hello.json"
)

with open(FIXTURE_PATH, encoding="utf-8") as _fixture_file:
    _GOLDEN = json.load(_fixture_file)

GOLDEN_SEED = bytes.fromhex(_GOLDEN["seed_hex"])
GOLDEN_PUBKEY = bytes.fromhex(_GOLDEN["pubkey_hex"])
GOLDEN_PEER_ID = _GOLDEN["peer_id"]
GOLDEN_SIGNATURE = _GOLDEN["signature_b64u"]
GOLDEN_MANIFEST_JSON = _GOLDEN["manifest_json"]
GOLDEN_NONCE = _GOLDEN["nonce"]


class GoldenParityTests(unittest.TestCase):
    def test_derive_peer_id(self) -> None:
        peer = spoke_connect.derive_peer_id_from_ed25519_pubkey(GOLDEN_PUBKEY)
        self.assertEqual(peer, GOLDEN_PEER_ID)

    def test_sign_hello_signature(self) -> None:
        hello_json = spoke_connect.sign_hello_ed25519(
            GOLDEN_SEED, GOLDEN_NONCE, GOLDEN_MANIFEST_JSON
        )
        hello = json.loads(hello_json)
        self.assertEqual(hello["peer_id"], GOLDEN_PEER_ID)
        self.assertEqual(hello["signature"], GOLDEN_SIGNATURE)

    def test_verify_hello(self) -> None:
        hello_json = spoke_connect.sign_hello_ed25519(
            GOLDEN_SEED, GOLDEN_NONCE, GOLDEN_MANIFEST_JSON
        )
        spoke_connect.verify_hello_ed25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, hello_json)

    def test_tampered_hello_rejected(self) -> None:
        hello_json = spoke_connect.sign_hello_ed25519(
            GOLDEN_SEED, GOLDEN_NONCE, GOLDEN_MANIFEST_JSON
        )
        tampered = hello_json.replace("data-store", "checker")
        with self.assertRaises(spoke_connect.CoreError.InvalidHelloSignature):
            spoke_connect.verify_hello_ed25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, tampered)

    def test_protocol_version(self) -> None:
        self.assertEqual(spoke_connect.protocol_version(), 1)


if __name__ == "__main__":
    unittest.main()
