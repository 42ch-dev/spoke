"""Optional-port dialer ops + responder ports serving smoke for the
spoke-connect Python binding.

Drives both new FFI faces over the loopback pair (D16) with no smoke host —
every face is on the committed production binding (`ffi,remote-adapter`), so
it runs in the default `unittest discover` suite: the responder serves
baseline + optional `port.*` families through a foreign `PortsHandler` (user
lock), the dialer drives `project` / `compute` / `listForkTimelineEvents`, and
the error rows — capability-gate deny, absent-ports fail-closed deny, and
foreign-fault containment with serve-loop survival — mirror the Rust
`connect_responder_ffi_tests` battery (parity with
`crates/spoke-connect/src/ffi.rs`). The router is untouched: optional ops
ride the per-peer `RemoteAdapterFFI`.
"""

from __future__ import annotations

import json
import time
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


def _ports_manifest_json(host_id: str) -> str:
    """Ports-carrying manifest — baseline + optional families, so the
    negotiated set includes l2-computable / l5-fork. Mirror of the Rust
    `ports_manifest_json` test helper."""
    return json.dumps(
        {
            "schema_version": 1,
            "host_id": host_id,
            "roles": ["data-store", "l2-computable"],
            "capabilities": ["spoke-baseline", "l2-computable", "l5-fork"],
            "namespaces": ["toy_world"],
            "extensions": {},
        }
    )


def _tool_manifest_json(host_id: str) -> str:
    """Tool-carrying manifest (baseline + tools only — no optional families)
    for the capability-deny session."""
    return json.dumps(
        {
            "schema_version": 1,
            "host_id": host_id,
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline", "tools.math.add"],
            "namespaces": ["math", "toy_world"],
            "extensions": {},
            "tools": [
                {
                    "schema_version": 1,
                    "capability_id": "tools.math.add",
                    "op": "tools.math.add",
                    "description": "Add two integers",
                    "input": {"type": "object"},
                    "output": {"type": "object"},
                }
            ],
        }
    )


def _knowledge_entry_json(entry_id: str, canonical_name: str) -> str:
    return json.dumps(
        {
            "schema_version": 1,
            "entry_id": entry_id,
            "entry_type": "knowledge",
            "canonical_name": canonical_name,
            "status": "active",
            "body": {"summary": "served through the foreign ports callback"},
            "extensions": {},
        }
    )


def _wait_for_state(
    tc: unittest.TestCase, what: str, state: object, expected: str
) -> None:
    """Bounded poll for the handshake to settle (D16 constructor semantics)."""
    deadline = time.monotonic() + 5.0
    last = state()
    while last != expected:
        if time.monotonic() >= deadline:
            tc.fail(f"{what}: timed out waiting for {expected!r} (last: {last!r})")
        time.sleep(0.01)
        last = state()


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


class _SmokePortsHandler:
    """Foreign-callback ports handler: in-memory knowledge store plus canned
    optional-family answers; unknown entries reject with an application
    `Rejected` (ordinary deny — not containment); `kb_ffi_ports_boom` faults
    (the containment row). Mirror of the Rust `TestPortsHandler`."""

    def __init__(self) -> None:
        self._entries: dict[str, object] = {}

    def get_knowledge_entry(self, entry_id: str) -> str:
        if entry_id == "kb_ffi_ports_boom":
            raise RuntimeError("foreign ports handler fault (containment row)")
        entry = self._entries.get(entry_id)
        if entry is None:
            raise spoke_connect.FfiError.Rejected(
                "KNOWLEDGE_ENTRY_NOT_FOUND",
                f"entry {entry_id} not found",
                "store_miss",
                None,
            )
        return json.dumps(entry)

    def put_knowledge_entry(self, entry_json: str, expected_base_revision: object) -> str:
        entry = json.loads(entry_json)
        self._entries[entry["entry_id"]] = entry
        return entry_json

    def get_relation(self, relation_id: str) -> str:
        raise spoke_connect.FfiError.Rejected(
            "INVALID_INPUT", "relation serving not exercised by this test handler", None, None
        )

    def put_relation(self, relation_json: str, expected_base_revision: object) -> str:
        raise spoke_connect.FfiError.Rejected(
            "INVALID_INPUT", "relation serving not exercised by this test handler", None, None
        )

    def list_knowledge_entries(self, scope_json: str) -> str:
        return json.dumps(list(self._entries.values()))

    def list_timeline_events(self, scope_json: str) -> str:
        return "[]"

    def put_findings(self, findings_json: str) -> str:
        return "[]"

    def list_rules(self, rule_refs: list[str]) -> str:
        return "[]"

    def list_peer_host_capability_manifests(self) -> str:
        return "[]"

    def project(self, project_request_json: str) -> str:
        request = json.loads(project_request_json)
        return json.dumps(
            {
                "session_id": request["session_id"],
                "entry_id": request["entry_id"],
                "computable": {"tide_level": 2.4, "cargo_tons": 38},
            }
        )

    def compute(self, compute_request_json: str) -> str:
        request = json.loads(compute_request_json)
        return json.dumps(
            {
                "session_id": request["session_id"],
                "entry_id": request["entry_id"],
                "computable": request["computable"],
                "state": request["computable"],
            }
        )

    def list_fork_timeline_events(self, scope_json: str) -> str:
        scope = json.loads(scope_json)
        if scope["fork_id"] != "fork_tw_ffi_events":
            return "[]"
        return json.dumps(
            [
                {
                    "schema_version": 1,
                    "timeline_event_id": "evt_tw_ffi_storm",
                    "canonical_name": "FFI Fork Storm",
                    "fork_id": "fork_tw_ffi_events",
                    "extensions": {},
                }
            ]
        )


class PortsLoopbackFfiPairTests(unittest.TestCase):
    def _dial_ports_pair(
        self, ports: spoke_connect.PortsHandler | None
    ) -> tuple[object, object]:
        """Loopback pair through both FFI faces with an optional foreign
        `PortsHandler`; both manifests declare the optional families. Mirror
        of the Rust `dial_responder_ffi_with_ports` test helper."""
        seed_client = _decode_hex(_FIXTURE["seed_client_hex"])
        seed_host = _decode_hex(_FIXTURE["seed_host_hex"])
        pubkey_host = _decode_hex(_FIXTURE["pubkey_host_hex"])
        pubkey_client = _decode_hex(_FIXTURE["pubkey_client_hex"])
        peer_id_host = _FIXTURE["peer_id_host"]
        peer_id_client = _FIXTURE["peer_id_client"]

        pair = spoke_connect.loopback_transport_pair()
        # The accept-side constructor returns immediately in `Handshaking`
        # (D16): the dialer hello is the sync point, so the smoke polls
        # `state()` (bounded) to `Established` before invoking.
        responder = spoke_connect.connect_responder_ffi(
            LoopbackCallbackTransport(pair.server()),
            seed_host,
            _ports_manifest_json("test-responder"),
            [peer_id_client],
            {peer_id_client: pubkey_client},
            ports,
            None,
        )
        dialer = spoke_connect.connect_remote_adapter_ffi(
            LoopbackCallbackTransport(pair.client()),
            seed_client,
            _ports_manifest_json("test-client"),
            pubkey_host,
            [peer_id_host],
            None,
        )
        return responder, dialer

    def _assert_optional_ops_denied(self, dialer: object, what: str) -> None:
        cases = [
            (
                "project",
                lambda: dialer.project(
                    '{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}'
                ),
            ),
            (
                "compute",
                lambda: dialer.compute(
                    '{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}'
                ),
            ),
            (
                "listForkTimelineEvents",
                lambda: dialer.list_fork_timeline_events(
                    '{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}'
                ),
            ),
        ]
        for name, invoke in cases:
            with self.assertRaises(spoke_connect.FfiError.Rejected) as denied:
                invoke()
            self.assertEqual(
                "CAPABILITY_PORT_MISSING",
                denied.exception.code,
                f"{what}: {name} deny code",
            )
            self.assertEqual(
                "op_unsupported",
                denied.exception.wire_code,
                f"{what}: {name} deny wire_code",
            )

    def test_ports_loopback_serves_baseline_and_optional_families(self) -> None:
        """Round-trips through a foreign `PortsHandler` (baseline + optional),
        the application-reject passthrough, malformed-JSON pre-validation,
        and foreign-fault containment with session survival."""
        handler = _SmokePortsHandler()
        responder, dialer = self._dial_ports_pair(handler)

        try:
            self.assertEqual("Established", dialer.state())
            _wait_for_state(self, "ports responder handshake", responder.state, "Established")

            # 1. Baseline round-trip through the foreign ports handler: put
            #    stores the entry JSON in the handler, get serves it back.
            #    The wire carries the canonicalized entry JSON (typed
            #    round-trip), so compare semantically, not byte-wise.
            entry_json = _knowledge_entry_json("kb_ffi_ports_put", "FFI Ports Put")
            put_json = dialer.put_knowledge_entry(entry_json, None)
            self.assertEqual("kb_ffi_ports_put", json.loads(put_json)["entry_id"])
            get_json = dialer.get_knowledge_entry("kb_ffi_ports_put")
            self.assertEqual("FFI Ports Put", json.loads(get_json)["canonical_name"])

            # 2. Application-reject passthrough: an unknown entry rejects
            #    with the handler's locked code + re-hung kind (ordinary
            #    deny, NOT containment).
            with self.assertRaises(spoke_connect.FfiError.Rejected) as missing:
                dialer.get_knowledge_entry("kb_ffi_ports_missing")
            self.assertEqual("KNOWLEDGE_ENTRY_NOT_FOUND", missing.exception.code)
            self.assertEqual("store_miss", missing.exception.kind, "kind re-hung")
            self.assertIsNone(missing.exception.wire_code)

            # 3. Optional dialer ops round-trip through the callback
            #    (l2-computable / l5-fork negotiated by both manifests).
            project_json = dialer.project(
                '{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}'
            )
            project = json.loads(project_json)
            self.assertEqual("sess_ffi_ports", project["session_id"])
            self.assertEqual("kb_ffi_ports_proj", project["entry_id"])
            self.assertEqual({"tide_level": 2.4, "cargo_tons": 38}, project["computable"])

            compute_json = dialer.compute(
                '{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}'
            )
            compute = json.loads(compute_json)
            expected_computable = {"tide_level": 2.5, "cargo_tons": 37}
            self.assertEqual(expected_computable, compute["computable"])
            self.assertEqual(expected_computable, compute["state"])

            events_json = dialer.list_fork_timeline_events(
                '{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}'
            )
            events = json.loads(events_json)
            self.assertEqual(1, len(events))
            self.assertEqual("evt_tw_ffi_storm", events[0]["timeline_event_id"])
            self.assertEqual("fork_tw_ffi_events", events[0]["fork_id"])

            # 4. Malformed JSON is rejected locally (INVALID_INPUT, zero wire
            #    traffic) — the dialer pre-validation row per op.
            with self.assertRaises(spoke_connect.FfiError.Rejected) as bad_project:
                dialer.project("{ not json")
            self.assertEqual("INVALID_INPUT", bad_project.exception.code)
            self.assertIsNone(bad_project.exception.wire_code)

            # 5. Foreign-fault containment: the handler faults on
            #    kb_ffi_ports_boom -> INTERNAL_ERROR with no details; the
            #    session survives and the serve loop answers the next
            #    healthy put.
            with self.assertRaises(spoke_connect.FfiError.Rejected) as contained:
                dialer.get_knowledge_entry("kb_ffi_ports_boom")
            self.assertEqual("INTERNAL_ERROR", contained.exception.code)
            self.assertIsNone(contained.exception.kind)
            self.assertIsNone(contained.exception.wire_code, "containment wire_code is None (details None)")

            healthy_json = dialer.put_knowledge_entry(
                _knowledge_entry_json("kb_ffi_ports_after", "After Containment"), None
            )
            self.assertEqual(
                "kb_ffi_ports_after",
                json.loads(healthy_json)["entry_id"],
                "serve loop survives foreign-fault containment",
            )
        finally:
            dialer.close()
            responder.close()
            self.assertEqual("Closed", dialer.state())
            self.assertEqual("Closed", responder.state())

    def test_ports_loopback_absent_ports_constructor_denies_fail_closed(self) -> None:
        """Absent-`ports` constructor is still valid (default deny): the
        responder is built without a `PortsHandler` while both manifests
        negotiate the optional families — the capability gate passes, the
        serving probe finds no ports face, and every optional op denies
        fail-closed with the preserved `op_unsupported` wire code."""
        responder, dialer = self._dial_ports_pair(None)

        try:
            self.assertEqual("Established", dialer.state())
            _wait_for_state(self, "absent-ports responder handshake", responder.state, "Established")

            self._assert_optional_ops_denied(dialer, "absent-ports deny")
        finally:
            dialer.close()
            responder.close()
            self.assertEqual("Closed", dialer.state())
            self.assertEqual("Closed", responder.state())

    def test_ports_loopback_capability_gate_denies_optional_ops(self) -> None:
        """Capability-gate deny: default manifests advertise `spoke-baseline`
        only, so the negotiated set lacks l2-computable / l5-fork and every
        optional op is denied at the responder's dispatch gate with the
        preserved `op_unsupported` wire code."""
        seed_client = _decode_hex(_FIXTURE["seed_client_hex"])
        seed_host = _decode_hex(_FIXTURE["seed_host_hex"])
        pubkey_host = _decode_hex(_FIXTURE["pubkey_host_hex"])
        pubkey_client = _decode_hex(_FIXTURE["pubkey_client_hex"])
        peer_id_host = _FIXTURE["peer_id_host"]
        peer_id_client = _FIXTURE["peer_id_client"]

        pair = spoke_connect.loopback_transport_pair()
        responder = spoke_connect.connect_responder_ffi(
            LoopbackCallbackTransport(pair.server()),
            seed_host,
            _tool_manifest_json("test-responder"),
            [peer_id_client],
            {peer_id_client: pubkey_client},
            None,
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
            _wait_for_state(self, "capability-deny responder handshake", responder.state, "Established")

            self._assert_optional_ops_denied(dialer, "capability deny")
        finally:
            dialer.close()
            responder.close()
            self.assertEqual("Closed", dialer.state())
            self.assertEqual("Closed", responder.state())


if __name__ == "__main__":
    unittest.main()
