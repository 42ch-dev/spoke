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

The KE remote section (`KeRemoteFfiPairTests`) mirrors the Rust
`ke_remote_ffi_tests` battery: the `extract` service face round-trips a
provisional batch, its three unavailability rows are distinct (unnegotiated
flag / absent ports probe / callback application refusal), and the existing
Scope query carries the `ke-ownership` gate (OQ-FFI-1) with the served
fields intact on allow and zero callback calls on deny. The host-local
loader value never crosses the wire: a loader-only canary is checked absent
against the recorded envelopes, and no loader callback is exported.
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


# ── KE remote (F1/F2/F3): the `extract` service face + the `ke-ownership`
# gate on the existing Scope query. Parity with the Rust
# `ke_remote_ffi_tests` battery (`crates/spoke-connect/src/ffi.rs`).
#
# The generated `PortsHandler` catalogue has no loader method — source
# loading is the serving host's own business — so the loader value is a
# private handler field and the canary below exists nowhere else: not in the
# request, not in the response. It is checked absent against the recorded
# envelopes.

LOADER_CANARY = "ke-ffi-loader-canary-python"


def _ke_manifest_json(host_id: str, capabilities: list[str]) -> str:
    """Both peers' hello manifests: the baseline capability plus whatever the
    scenario negotiates — a capability must appear in *both* hellos to be
    negotiated, so the omitted side is how the deny scenarios are set up.
    Mirror of the Rust `ke_manifest_json` test helper."""
    return json.dumps(
        {
            "schema_version": 1,
            "host_id": host_id,
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline", *capabilities],
            "namespaces": ["toy_world"],
            "extensions": {},
        }
    )


def _extract_request_json(run_id: str) -> str:
    """An `extract` request for the scenario's run id: the payload is the
    `ExtractRequest` itself and `sources` carry references only — source
    loading is the serving host's business."""
    return json.dumps(
        {
            "run_id": run_id,
            "sources": [
                {"schema_version": 1, "source_id": "manuscript/ch1", "extensions": {}},
                {"schema_version": 1, "source_id": "manuscript/ch2", "extensions": {}},
            ],
        }
    )


def _viewpoint_scope_json() -> str:
    """A Scope carrying a reader viewpoint plus opaque extension values the
    callback must receive unchanged."""
    return json.dumps(
        {
            "scope_id": "toy-scope-001",
            "viewpoint": "kb_tw_mira",
            "entry_types": ["note"],
            "extensions": {"product": {"viewpoint": "decoy", "owner": "someone"}},
        }
    )


def _ke_entry(entry_id: str, canonical_name: str, visibility: str) -> dict:
    """One served `KnowledgeEntry`; `visibility` rides the opaque extensions
    and is the host's own filtering input (not a wire contract)."""
    return {
        "schema_version": 1,
        "entry_id": entry_id,
        "entry_type": "knowledge",
        "canonical_name": canonical_name,
        "status": "active",
        "body": {"summary": "served through the foreign ports callback"},
        "extensions": {"visibility": {"scope": visibility}},
    }


def _entry_visible(entry: dict, viewpoint: object) -> bool:
    """The host's real viewpoint filter: shared entries plus the reader's own
    private entries; foreign-private entries stay behind."""
    visibility = entry["extensions"]["visibility"]["scope"]
    return visibility == "shared" or visibility == f"private:{viewpoint}"


class _RecordingCallbackTransport(LoopbackCallbackTransport):
    """Dialer-side transport that records every envelope in both directions,
    so the smoke can assert what did and did not reach the wire."""

    def __init__(self, inner: spoke_connect.LoopbackTransport) -> None:
        super().__init__(inner)
        self.frames: list[bytes] = []

    def send(self, envelope: bytes) -> None:
        self.frames.append(envelope)
        super().send(envelope)

    def recv(self) -> bytes:
        envelope = super().recv()
        self.frames.append(envelope)
        return envelope


class _KeRemotePortsHandler:
    """Foreign-callback ports handler for the KE remote matrix: a private
    host-local loader value, the `extract` service face, and a
    viewpoint-filtered knowledge store. Mirror of the Rust
    `KeRemotePortsHandler` double.

    `decline_extract` answers `extract` with an application refusal — "this
    binding does not serve extraction" — which is an ordinary deny with
    `wire_code = None`, never the absent-provider probe deny. Every call and
    every received request/scope JSON is recorded."""

    def __init__(self, decline_extract: bool = False) -> None:
        self.decline_extract = decline_extract
        self.extract_requests: list[object] = []
        self.scope_calls: list[object] = []
        self._loaded_input = {"canary": LOADER_CANARY}
        self._entries = [
            _ke_entry("kb_ffi_ke_foreign", "FFI KE Foreign", "private:kb_tw_other"),
            _ke_entry("kb_ffi_ke_own", "FFI KE Own", "private:kb_tw_mira"),
            _ke_entry("kb_ffi_ke_shared", "FFI KE Shared", "shared"),
        ]

    def extract(self, extract_request_json: str) -> str:
        request = json.loads(extract_request_json)
        self.extract_requests.append(request)
        if self.decline_extract:
            raise spoke_connect.FfiError.Rejected(
                "CAPABILITY_PORT_MISSING",
                "this binding does not serve extraction",
                None,
                None,
            )
        # The loader runs inside the host service: its value feeds the
        # extractor here and is never a transport argument or callback value.
        if not self._loaded_input.get("canary"):
            raise RuntimeError("host loader value is missing")
        run_id = request["run_id"]
        candidates = [
            {
                "schema_version": 1,
                "entry_id": f"ke-ffi-{run_id}-{index}",
                "entry_type": "note",
                "canonical_name": f"Extracted note {index}",
                "status": "provisional",
                "body": {"summary": f"provisional candidate {index}"},
                "extensions": {},
            }
            for index, _source in enumerate(request["sources"])
        ]
        return json.dumps({"candidates": candidates, "run": {"run_id": run_id}})

    def list_knowledge_entries(self, scope_json: str) -> str:
        scope = json.loads(scope_json)
        self.scope_calls.append(scope)
        viewpoint = scope.get("viewpoint")
        visible = [entry for entry in self._entries if _entry_visible(entry, viewpoint)]
        return json.dumps(sorted(visible, key=lambda entry: entry["entry_id"]))

    # Catalogue ops this double does not serve: an ordinary application
    # reject (`wire_code = None`), never containment.
    def _unserved(self, op: str) -> None:
        raise spoke_connect.FfiError.Rejected(
            "INVALID_INPUT", f"{op} is not served by this test handler", None, None
        )

    def get_knowledge_entry(self, entry_id: str) -> str:
        self._unserved("get_knowledge_entry")
        raise AssertionError("unreachable")

    def put_knowledge_entry(self, entry_json: str, expected_base_revision: object) -> str:
        self._unserved("put_knowledge_entry")
        raise AssertionError("unreachable")

    def get_relation(self, relation_id: str) -> str:
        self._unserved("get_relation")
        raise AssertionError("unreachable")

    def put_relation(self, relation_json: str, expected_base_revision: object) -> str:
        self._unserved("put_relation")
        raise AssertionError("unreachable")

    def list_timeline_events(self, scope_json: str) -> str:
        self._unserved("list_timeline_events")
        raise AssertionError("unreachable")

    def put_findings(self, findings_json: str) -> str:
        self._unserved("put_findings")
        raise AssertionError("unreachable")

    def list_rules(self, rule_refs: list[str]) -> str:
        self._unserved("list_rules")
        raise AssertionError("unreachable")

    def list_peer_host_capability_manifests(self) -> str:
        self._unserved("list_peer_host_capability_manifests")
        raise AssertionError("unreachable")

    def project(self, project_request_json: str) -> str:
        self._unserved("project")
        raise AssertionError("unreachable")

    def compute(self, compute_request_json: str) -> str:
        self._unserved("compute")
        raise AssertionError("unreachable")

    def list_fork_timeline_events(self, scope_json: str) -> str:
        self._unserved("list_fork_timeline_events")
        raise AssertionError("unreachable")


class KeRemoteFfiPairTests(unittest.TestCase):
    """The frozen KE remote matrix over both FFI faces: extract round-trip,
    its three unavailability outcomes, and the OQ-FFI-1 ownership witness on
    the existing Scope query."""

    def _dial_ke_pair(
        self,
        client_capabilities: list[str],
        responder_capabilities: list[str],
        ports: object,
    ) -> tuple[object, object, _RecordingCallbackTransport]:
        """Loopback pair through both FFI faces with the scenario's caps and
        optional foreign `PortsHandler`. Mirror of the Rust `dial_ke_remote`
        test helper."""
        pair = spoke_connect.loopback_transport_pair()
        responder = spoke_connect.connect_responder_ffi(
            LoopbackCallbackTransport(pair.server()),
            _decode_hex(_FIXTURE["seed_host_hex"]),
            _ke_manifest_json("test-responder", responder_capabilities),
            [_FIXTURE["peer_id_client"]],
            {_FIXTURE["peer_id_client"]: _decode_hex(_FIXTURE["pubkey_client_hex"])},
            ports,
            None,
        )
        dialer_transport = _RecordingCallbackTransport(pair.client())
        dialer = spoke_connect.connect_remote_adapter_ffi(
            dialer_transport,
            _decode_hex(_FIXTURE["seed_client_hex"]),
            _ke_manifest_json("test-client", client_capabilities),
            _decode_hex(_FIXTURE["pubkey_host_hex"]),
            [_FIXTURE["peer_id_host"]],
            None,
        )
        self.assertEqual("Established", dialer.state())
        _wait_for_state(
            self, "ke-remote responder handshake", responder.state, "Established"
        )
        return responder, dialer, dialer_transport

    def _assert_unavailable(
        self, invoke: object, wire_code: object, what: str
    ) -> None:
        """Assert the unavailable-capability row shared by the unnegotiated
        and absent-provider denials (the callback-refusal row differs in
        `wire_code` and is asserted at its own case)."""
        with self.assertRaises(spoke_connect.FfiError.Rejected) as denied:
            invoke()
        self.assertEqual("CAPABILITY_PORT_MISSING", denied.exception.code, f"{what}: code")
        self.assertIsNone(denied.exception.kind, f"{what}: kind")
        self.assertEqual(wire_code, denied.exception.wire_code, f"{what}: wire_code")

    def test_ke_remote_extract_round_trips_the_provisional_batch(self) -> None:
        handler = _KeRemotePortsHandler()
        responder, dialer, _transport = self._dial_ke_pair(
            ["ke-extraction"], ["ke-extraction"], handler
        )
        try:
            response = json.loads(dialer.extract(_extract_request_json("run-ffi-ke-1")))

            # Observable, deterministic response behaviour: one provisional
            # candidate per declared source reference, carrying the run id.
            self.assertEqual("run-ffi-ke-1", response["run"]["run_id"])
            candidates = response["candidates"]
            self.assertEqual(2, len(candidates), "one candidate per declared source")
            self.assertEqual(
                ["ke-ffi-run-ffi-ke-1-0", "ke-ffi-run-ffi-ke-1-1"],
                [candidate["entry_id"] for candidate in candidates],
            )
            for candidate in candidates:
                self.assertEqual("provisional", candidate["status"])
                self.assertEqual("note", candidate["entry_type"])

            # The callback received the `ExtractRequest` itself: no wrapper
            # and no loader value — only references travelled.
            self.assertEqual(1, len(handler.extract_requests))
            request = handler.extract_requests[0]
            self.assertEqual("run-ffi-ke-1", request["run_id"])
            self.assertEqual(2, len(request["sources"]))
            self.assertEqual("manuscript/ch1", request["sources"][0]["source_id"])
            for wrapper in ("request", "arguments", "input", "loaded_input"):
                self.assertNotIn(wrapper, request, f"{wrapper} is not part of the payload")
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_extract_rejects_malformed_request_json_with_zero_wire_traffic(self) -> None:
        handler = _KeRemotePortsHandler()
        responder, dialer, transport = self._dial_ke_pair(
            ["ke-extraction"], ["ke-extraction"], handler
        )
        try:
            frames_before = len(transport.frames)
            with self.assertRaises(spoke_connect.FfiError.Rejected) as malformed:
                dialer.extract("{ not json")
            self.assertEqual("INVALID_INPUT", malformed.exception.code)
            self.assertIsNone(malformed.exception.kind)
            self.assertIsNone(malformed.exception.wire_code)
            self.assertEqual([], handler.extract_requests, "the callback is never reached")
            self.assertEqual(
                frames_before,
                len(transport.frames),
                "the malformed request never reaches the wire",
            )
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_extract_denies_when_the_responding_hello_omits_ke_extraction(self) -> None:
        # Otherwise identical manifests with one flag omitted: the dialing
        # hello declares `ke-extraction`, the responding hello does not, so
        # the negotiated intersection lacks it and the gate denies before any
        # host work.
        handler = _KeRemotePortsHandler()
        responder, dialer, _transport = self._dial_ke_pair(
            ["ke-extraction"], [], handler
        )
        try:
            self._assert_unavailable(
                lambda: dialer.extract(_extract_request_json("run-ffi-unnegotiated")),
                "op_unsupported",
                "unnegotiated (responder omits the flag)",
            )
            self.assertEqual([], handler.extract_requests, "the gate denies before the callback")
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_extract_denies_when_the_dialing_hello_omits_ke_extraction(self) -> None:
        # The mirror direction of the negotiation proof: the responding hello
        # declares the flag, the dialing one does not.
        handler = _KeRemotePortsHandler()
        responder, dialer, _transport = self._dial_ke_pair(
            [], ["ke-extraction"], handler
        )
        try:
            self._assert_unavailable(
                lambda: dialer.extract(_extract_request_json("run-ffi-unnegotiated")),
                "op_unsupported",
                "unnegotiated (dialer omits the flag)",
            )
            self.assertEqual([], handler.extract_requests, "the gate denies before the callback")
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_extract_probe_denies_when_ports_are_absent(self) -> None:
        # `ke-extraction` is negotiated in both hellos but the responder has
        # no ports face at all: the absent-provider probe deny — a different
        # row from the unnegotiated gate and from a callback's own refusal.
        responder, dialer, _transport = self._dial_ke_pair(
            ["ke-extraction"], ["ke-extraction"], None
        )
        try:
            self._assert_unavailable(
                lambda: dialer.extract(_extract_request_json("run-ffi-absent-ports")),
                "op_unsupported",
                "absent-ports probe deny",
            )
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_extract_passes_a_callback_application_refusal_through(self) -> None:
        handler = _KeRemotePortsHandler(decline_extract=True)
        responder, dialer, _transport = self._dial_ke_pair(
            ["ke-extraction"], ["ke-extraction"], handler
        )
        try:
            with self.assertRaises(spoke_connect.FfiError.Rejected) as refused:
                dialer.extract(_extract_request_json("run-ffi-refusal"))
            self.assertEqual("CAPABILITY_PORT_MISSING", refused.exception.code)
            self.assertIsNone(refused.exception.kind)
            self.assertIsNone(
                refused.exception.wire_code,
                "a callback refusal is not a missing-method probe deny",
            )
            self.assertIn("does not serve extraction", refused.exception.message)
            self.assertEqual(
                1,
                len(handler.extract_requests),
                "a refusal is the callback's own answer, not a probe deny",
            )
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_scope_viewpoint_serves_with_ownership_negotiated(self) -> None:
        # OQ-FFI-1: the existing Scope method carries the ownership gate — no
        # ownership-specific method or callback is added.
        handler = _KeRemotePortsHandler()
        responder, dialer, _transport = self._dial_ke_pair(
            ["ke-ownership"], ["ke-ownership"], handler
        )
        try:
            served = json.loads(dialer.list_knowledge_entries(_viewpoint_scope_json()))
            self.assertEqual(
                ["kb_ffi_ke_own", "kb_ffi_ke_shared"],
                [entry["entry_id"] for entry in served],
                "the host's viewpoint filter keeps shared + own-private entries",
            )
            self.assertEqual("FFI KE Own", served[0]["canonical_name"])

            # The callback received the declared Scope unchanged: the
            # viewpoint and the opaque extension values are not stripped to
            # make the request succeed, and no extraction call was involved.
            self.assertEqual(1, len(handler.scope_calls), "the callback is reached exactly once")
            scope = handler.scope_calls[0]
            self.assertEqual("kb_tw_mira", scope["viewpoint"])
            self.assertEqual(["note"], scope["entry_types"])
            self.assertEqual("decoy", scope["extensions"]["product"]["viewpoint"])
            self.assertEqual("someone", scope["extensions"]["product"]["owner"])
            self.assertEqual([], handler.extract_requests)
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_scope_viewpoint_denies_when_the_dialing_hello_omits_ke_ownership(self) -> None:
        handler = _KeRemotePortsHandler()
        responder, dialer, _transport = self._dial_ke_pair(
            [], ["ke-ownership"], handler
        )
        try:
            self._assert_unavailable(
                lambda: dialer.list_knowledge_entries(_viewpoint_scope_json()),
                "op_unsupported",
                "ownership deny (dialer omits the flag)",
            )
            self.assertEqual(
                [], handler.scope_calls, "the gate refuses before the callback is reached"
            )
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_scope_viewpoint_denies_when_the_responding_hello_omits_ke_ownership(self) -> None:
        handler = _KeRemotePortsHandler()
        responder, dialer, _transport = self._dial_ke_pair(
            ["ke-ownership"], [], handler
        )
        try:
            self._assert_unavailable(
                lambda: dialer.list_knowledge_entries(_viewpoint_scope_json()),
                "op_unsupported",
                "ownership deny (responder omits the flag)",
            )
            self.assertEqual(
                [], handler.scope_calls, "the gate refuses before the callback is reached"
            )
        finally:
            dialer.close()
            responder.close()

    def test_ke_remote_loader_canary_never_reaches_the_wire(self) -> None:
        handler = _KeRemotePortsHandler()
        responder, dialer, transport = self._dial_ke_pair(
            ["ke-extraction"], ["ke-extraction"], handler
        )
        try:
            response = dialer.extract(_extract_request_json("run-ffi-canary"))
            self.assertIn("run-ffi-canary", response, "the round-trip really happened")
            self.assertNotIn(
                LOADER_CANARY, response, "the loader value is not serialized into the response"
            )
            self.assertTrue(transport.frames, "the dialer recorded the session envelopes")
            # Positive control: the recording really observes the plaintext
            # wire (the invoke request's run id is visible), so the canary
            # absence below is a real observation, not a vacuous one.
            recorded = b"".join(transport.frames)
            self.assertIn(b"run-ffi-canary", recorded, "the recorded envelopes carry the request")
            offsets = [
                index
                for index, frame in enumerate(transport.frames)
                if LOADER_CANARY.encode() in frame
            ]
            self.assertEqual(
                [],
                offsets,
                f"the loader canary must never appear in a recorded envelope (frames {offsets})",
            )
        finally:
            dialer.close()
            responder.close()


if __name__ == "__main__":
    unittest.main()
