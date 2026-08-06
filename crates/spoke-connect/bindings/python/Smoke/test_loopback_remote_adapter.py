"""RemoteAdapterFFI loopback smoke for the spoke-connect Python binding.

Dials `RemoteAdapterFFI` through a Python `Transport` implementation over the
in-memory loopback pair and the reference smoke host (parity with Swift
`loopback_smoke.swift`, Kotlin `RemoteAdapterLoopbackTest.kt`, and
`crates/spoke-connect/src/ffi.rs` remote_adapter_ffi_tests).

Requires bindings regenerated from a smoke cdylib (`ffi-smoke-host`) and that
cdylib staged beside `spoke_connect/__init__.py`. Production bindings skip this
module via `unittest.skipUnless`.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

import spoke_connect

FIXTURE_PATH = Path(__file__).resolve().parent / "fixtures" / "loopback-smoke.json"

with open(FIXTURE_PATH, encoding="utf-8") as _fixture_file:
    _FIXTURE = json.load(_fixture_file)


def _decode_hex(hex_str: str) -> bytes:
    if len(hex_str) % 2 != 0:
        raise ValueError("hex must have even length")
    return bytes.fromhex(hex_str)


def _knowledge_entry_json(entry_id: str, canonical_name: str) -> str:
    return (
        '{"schema_version":1,"entry_id":"'
        + entry_id
        + '","entry_type":"character","canonical_name":"'
        + canonical_name
        + '","status":"provisional","body":{"summary":"Upserted over the loopback: '
        + entry_id
        + '"},"extensions":{}}'
    )


class LoopbackCallbackTransport:
    """Foreign-callback transport delegating to the client end of a loopback pair."""

    def __init__(self, inner: spoke_connect.LoopbackTransport) -> None:
        self._inner = inner

    def send(self, envelope: bytes) -> None:
        self._inner.send(envelope)

    def recv(self) -> bytes:
        return self._inner.recv()

    def close(self) -> None:
        self._inner.close()


@unittest.skipUnless(
    hasattr(spoke_connect, "start_loopback_smoke_host"),
    "requires ffi-smoke-host cdylib and smoke-generated bindings",
)
class RemoteAdapterLoopbackTests(unittest.TestCase):
    def test_remote_adapter_put_get_round_trip(self) -> None:
        seed_client = _decode_hex(_FIXTURE["seed_client_hex"])
        pubkey_host = _decode_hex(_FIXTURE["pubkey_host_hex"])
        peer_id_host = _FIXTURE["peer_id_host"]
        client_manifest_json = _FIXTURE["client_manifest_json"]
        session_id = _FIXTURE["session_id"]
        entry_id = _FIXTURE["entry_id"]
        entry_canonical_name = _FIXTURE["entry_canonical_name"]

        pair = spoke_connect.loopback_transport_pair()
        host = spoke_connect.start_loopback_smoke_host(pair.server())

        transport = LoopbackCallbackTransport(pair.client())
        adapter = spoke_connect.connect_remote_adapter_ffi(
            transport,
            seed_client,
            client_manifest_json,
            pubkey_host,
            [peer_id_host],
            None,
        )

        try:
            self.assertEqual("Established", adapter.state())
            self.assertEqual(session_id, adapter.session_id())
            self.assertEqual(session_id, host.session_id())
            self.assertEqual(peer_id_host, adapter.remote_peer_id())

            remote_manifest = adapter.remote_manifest()
            self.assertIsNotNone(remote_manifest)
            manifest_json = json.loads(remote_manifest)
            self.assertEqual("test-host", manifest_json["host_id"])

            entry_json = _knowledge_entry_json(entry_id, entry_canonical_name)
            put_json = adapter.put_knowledge_entry(entry_json, None)
            self.assertTrue(put_json)
            put_object = json.loads(put_json)
            self.assertEqual(entry_id, put_object["entry_id"])

            get_json = adapter.get_knowledge_entry(entry_id)
            self.assertTrue(get_json)
            get_object = json.loads(get_json)
            self.assertEqual(entry_id, get_object["entry_id"])
            self.assertEqual(entry_canonical_name, get_object["canonical_name"])
        finally:
            adapter.close()
            self.assertEqual("Closed", adapter.state())
            host.close()


if __name__ == "__main__":
    unittest.main()
