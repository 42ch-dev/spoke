"""Golden-parity smoke for the spoke-connect Python binding.

Reproduces the Rust golden vectors (crates/spoke-connect/src/ffi.rs tests)
through the generated uniffi surface. Run from the repository root:

    PYTHONPATH=crates/spoke-connect/bindings/python python3 -m unittest discover -s crates/spoke-connect/bindings/python/Smoke -v
"""

from __future__ import annotations

import json
import unittest

import spoke_connect

GOLDEN_SEED = bytes(range(1, 33))
GOLDEN_PUBKEY = bytes(
    [
        0x79,
        0xB5,
        0x56,
        0x2E,
        0x8F,
        0xE6,
        0x54,
        0xF9,
        0x40,
        0x78,
        0xB1,
        0x12,
        0xE8,
        0xA9,
        0x8B,
        0xA7,
        0x90,
        0x1F,
        0x85,
        0x3A,
        0xE6,
        0x95,
        0xBE,
        0xD7,
        0xE0,
        0xE3,
        0x91,
        0x0B,
        0xAD,
        0x04,
        0x96,
        0x64,
    ]
)
GOLDEN_PEER_ID = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"
GOLDEN_SIGNATURE = (
    "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg"
)
GOLDEN_MANIFEST_JSON = (
    '{"capabilities":["spoke-baseline"],"extensions":{},"host_id":"golden-host",'
    '"namespaces":[],"roles":["data-store"],"schema_version":1}'
)


def golden_nonce() -> str:
    return "-".join(["golden-nonce", "000000000001"])


class GoldenParityTests(unittest.TestCase):
    def test_derive_peer_id(self) -> None:
        peer = spoke_connect.derive_peer_id_from_ed25519_pubkey(GOLDEN_PUBKEY)
        self.assertEqual(peer, GOLDEN_PEER_ID)

    def test_sign_hello_signature(self) -> None:
        hello_json = spoke_connect.sign_hello_ed25519(
            GOLDEN_SEED, golden_nonce(), GOLDEN_MANIFEST_JSON
        )
        hello = json.loads(hello_json)
        self.assertEqual(hello["peer_id"], GOLDEN_PEER_ID)
        self.assertEqual(hello["signature"], GOLDEN_SIGNATURE)

    def test_verify_hello(self) -> None:
        hello_json = spoke_connect.sign_hello_ed25519(
            GOLDEN_SEED, golden_nonce(), GOLDEN_MANIFEST_JSON
        )
        spoke_connect.verify_hello_ed25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, hello_json)

    def test_tampered_hello_rejected(self) -> None:
        hello_json = spoke_connect.sign_hello_ed25519(
            GOLDEN_SEED, golden_nonce(), GOLDEN_MANIFEST_JSON
        )
        tampered = hello_json.replace("data-store", "checker")
        with self.assertRaises(spoke_connect.CoreError.InvalidHelloSignature):
            spoke_connect.verify_hello_ed25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, tampered)

    def test_protocol_version(self) -> None:
        self.assertEqual(spoke_connect.protocol_version(), 1)


if __name__ == "__main__":
    unittest.main()
