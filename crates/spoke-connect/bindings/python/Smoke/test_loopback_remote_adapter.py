"""RemoteAdapterFFI loopback smoke for the spoke-connect Python binding.

Dials `RemoteAdapterFFI` through a Python `Transport` implementation over the
in-memory loopback pair and the reference smoke host (parity with Swift
`loopback_smoke.swift`, Kotlin `RemoteAdapterLoopbackTest.kt`, and
`crates/spoke-connect/src/ffi.rs` remote_adapter_ffi_tests). The tool-pair
test drives both FFI faces (D15/D16) over a loopback pair with no smoke host:
both ends are FFI objects.

Requires bindings regenerated from a smoke cdylib (`ffi-smoke-host`) and that
cdylib staged beside `spoke_connect/__init__.py`. Production bindings skip this
module via `unittest.skipUnless`.
"""

from __future__ import annotations

import json
import time
import unittest
from collections.abc import Callable
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


def _tool_manifest_json(host_id: str) -> str:
    """Tool-carrying manifest — every tool capability also sits in
    `capabilities[]` so the negotiated set includes the `tools.*` ops
    (D13 dispatch gate). Mirror of the Rust `tool_manifest` test helper."""
    tool_descriptors = [
        ("tools.math.add", "Add two integers"),
        ("tools.echo.echo", "Echo the arguments"),
        ("tools.echo.boom", "Explodes"),
    ]
    manifest = {
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": ["spoke-baseline"] + [tool_id for tool_id, _ in tool_descriptors],
        "namespaces": ["math", "echo", "toy_world"],
        "extensions": {},
        "tools": [
            {
                "schema_version": 1,
                "capability_id": tool_id,
                "op": tool_id,
                "description": description,
                "input": {"type": "object"},
                "output": {"type": "object"},
            }
            for tool_id, description in tool_descriptors
        ],
    }
    return json.dumps(manifest)


def _wait_for_state(
    tc: unittest.TestCase, what: str, state: Callable[[], str], expected: str
) -> None:
    """Bounded poll for the handshake to settle (D16 constructor semantics)."""
    deadline = time.monotonic() + 5.0
    last = state()
    while last != expected:
        if time.monotonic() >= deadline:
            tc.fail(f"{what}: timed out waiting for {expected!r} (last: {last!r})")
        time.sleep(0.01)
        last = state()


class _SumToolHandler:
    """Foreign-callback tool handler: sums `a` + `b` (Rust `add_handler`
    parity) and records the invocation count."""

    def __init__(self) -> None:
        self._calls = 0

    def handle(self, arguments_json: str) -> str:
        self._calls += 1
        arguments = json.loads(arguments_json)
        return json.dumps({"sum": arguments.get("a", 0) + arguments.get("b", 0)})

    def calls(self) -> int:
        return self._calls


class _ThrowingToolHandler:
    """Foreign-callback tool handler that always raises the given
    application reject (D16 passthrough row)."""

    def __init__(self, reject: spoke_connect.FfiError.Rejected) -> None:
        self._reject = reject

    def handle(self, arguments_json: str) -> str:
        raise self._reject


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

    def test_tool_loopback_ffi_pair(self) -> None:
        """Tool faces over the loopback pair (D15/D16): both ends are FFI
        objects — the responder serves a foreign `ToolHandler`, the dialer
        serves reverse invokes through `RemoteAdapterFfi.register_tool_handler`,
        unregistered tools deny with `op_unsupported`, and a handler-thrown
        application reject passes through verbatim (parity with
        `crates/spoke-connect/src/ffi.rs` connect_responder_ffi_tests)."""
        seed_client = _decode_hex(_FIXTURE["seed_client_hex"])
        seed_host = _decode_hex(_FIXTURE["seed_host_hex"])
        pubkey_host = _decode_hex(_FIXTURE["pubkey_host_hex"])
        pubkey_client = _decode_hex(_FIXTURE["pubkey_client_hex"])
        peer_id_host = _FIXTURE["peer_id_host"]
        peer_id_client = _FIXTURE["peer_id_client"]

        pair = spoke_connect.loopback_transport_pair()
        # The accept-side constructor returns immediately in `Handshaking`
        # (D16): the dialer hello is the sync point, so the smoke polls
        # `state()` (bounded) to `Established` before invoking; a handshake
        # failure surfaces as `Closed`, never a thrown constructor error.
        responder = spoke_connect.connect_responder_ffi(
            LoopbackCallbackTransport(pair.server()),
            seed_host,
            _tool_manifest_json("test-responder"),
            [peer_id_client],
            {peer_id_client: pubkey_client},
            None,
        )
        dialer = spoke_connect.connect_remote_adapter_ffi(
            LoopbackCallbackTransport(pair.client()),
            seed_client,
            _tool_manifest_json("test-client"),
            pubkey_host,
            [peer_id_host],
            None,
        )

        try:
            self.assertEqual("Established", dialer.state())
            _wait_for_state(self, "tool responder handshake", responder.state, "Established")

            # 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
            responder_sum = _SumToolHandler()
            responder.register_tool_handler("tools.math.add", responder_sum)
            sum_json = dialer.invoke_tool("tools.math.add", '{"a": 1, "b": 2}')
            self.assertEqual(3, json.loads(sum_json)["sum"])
            self.assertEqual(1, responder_sum.calls(), "responder handler invocation count")

            # 2. Responder FFI invoke_tool -> dialer-side handler registered
            #    via RemoteAdapterFfi.register_tool_handler.
            dialer_sum = _SumToolHandler()
            dialer.register_tool_handler("tools.math.add", dialer_sum)
            reverse_sum_json = responder.invoke_tool("tools.math.add", '{"a": 21, "b": 21}')
            self.assertEqual(42, json.loads(reverse_sum_json)["sum"])
            self.assertEqual(1, dialer_sum.calls(), "dialer handler invocation count")

            # 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
            with self.assertRaises(spoke_connect.FfiError.Rejected) as denied:
                dialer.invoke_tool("tools.echo.boom", "{}")
            self.assertEqual("CAPABILITY_PORT_MISSING", denied.exception.code)
            self.assertEqual("op_unsupported", denied.exception.wire_code)

            # 4. Handler-thrown application reject passes through verbatim
            #    (kind / wire_code re-hung onto details by the bridge).
            dialer.register_tool_handler(
                "tools.echo.boom",
                _ThrowingToolHandler(
                    spoke_connect.FfiError.Rejected(
                        "REVISION_CONFLICT", "foreign handler rejected", None, "op_unsupported"
                    )
                ),
            )
            with self.assertRaises(spoke_connect.FfiError.Rejected) as passed:
                responder.invoke_tool("tools.echo.boom", "{}")
            self.assertEqual("REVISION_CONFLICT", passed.exception.code)
            self.assertEqual("foreign handler rejected", passed.exception.message)
            self.assertEqual("op_unsupported", passed.exception.wire_code)
        finally:
            dialer.close()
            responder.close()
            self.assertEqual("Closed", dialer.state())
            self.assertEqual("Closed", responder.state())


if __name__ == "__main__":
    unittest.main()
