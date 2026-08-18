// ports_loopback_smoke.swift — optional-port dialer ops + responder ports
// serving over the loopback pair (D16), run in the DEFAULT macOS smoke
// against the committed production binding (no smoke host needed): the
// responder serves baseline + optional `port.*` families through a foreign
// `PortsHandler` (user lock), the dialer drives `project` / `compute` /
// `listForkTimelineEvents`, and the error rows — capability-gate deny,
// absent-ports fail-closed deny, and foreign-fault containment with
// serve-loop survival — mirror the Rust `connect_responder_ffi_tests`
// battery (parity with crates/spoke-connect/src/ffi.rs). The router is
// untouched: optional ops ride the per-peer `RemoteAdapterFFI`.

import Foundation

/// Ports-carrying manifest — baseline + optional families, so the negotiated
/// set includes l2-computable / l5-fork. Mirror of the Rust
/// `ports_manifest_json` test helper.
private func portsManifestJson(hostId: String) -> String {
    """
    {"schema_version":1,"host_id":"\(hostId)","roles":["data-store","l2-computable"],
    "capabilities":["spoke-baseline","l2-computable","l5-fork"],
    "namespaces":["toy_world"],"extensions":{}}
    """
}

/// Tool-carrying manifest (baseline + tools only — no optional families) for
/// the capability-deny session.
private func baselineOnlyManifestJson(hostId: String) -> String {
    """
    {"schema_version":1,"host_id":"\(hostId)","roles":["data-store"],
    "capabilities":["spoke-baseline","tools.math.add"],
    "namespaces":["math","toy_world"],"extensions":{},
    "tools":[{"schema_version":1,"capability_id":"tools.math.add","op":"tools.math.add",
    "description":"Add two integers","input":{"type":"object"},"output":{"type":"object"}}]}
    """
}

private func knowledgeEntryJson(entryId: String, canonicalName: String) -> String {
    """
    {"schema_version":1,"entry_id":"\(entryId)","entry_type":"knowledge",
    "canonical_name":"\(canonicalName)","status":"active",
    "body":{"summary":"served through the foreign ports callback"},"extensions":{}}
    """
}

private let projectRequestJson =
    "{\"session_id\":\"sess_ffi_ports\",\"entry_id\":\"kb_ffi_ports_proj\",\"state\":{\"tide_level\":2.1,\"cargo_tons\":40}}"
private let computeRequestJson =
    "{\"session_id\":\"sess_ffi_ports\",\"entry_id\":\"kb_ffi_ports_cmp\",\"computable\":{\"tide_level\":2.5,\"cargo_tons\":37},\"settle\":true}"
private let forkScopeJson =
    "{\"scope_id\":\"pkt_tw_scope\",\"fork_id\":\"fork_tw_ffi_events\"}"

/// Loopback fixture (byte-identical copy of the crate SSOT fixture; same
/// fields as the tool-smoke loader).
private struct PortsLoopbackSmokeFixture {
    let seedClient: Data
    let seedHost: Data
    let pubkeyHost: Data
    let pubkeyClient: Data
    let peerIdHost: String
    let peerIdClient: String

    static func load() throws -> PortsLoopbackSmokeFixture {
        let fixtureURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("fixtures/loopback-smoke.json")
        let data = try Data(contentsOf: fixtureURL)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "ports_loopback_smoke", code: 7, userInfo: [
                NSLocalizedDescriptionKey: "fixture is not a JSON object",
            ])
        }
        guard
            let seedClientHex = json["seed_client_hex"] as? String,
            let seedClient = hexData(seedClientHex),
            let seedHostHex = json["seed_host_hex"] as? String,
            let seedHost = hexData(seedHostHex),
            let pubkeyHostHex = json["pubkey_host_hex"] as? String,
            let pubkeyHost = hexData(pubkeyHostHex),
            let pubkeyClientHex = json["pubkey_client_hex"] as? String,
            let pubkeyClient = hexData(pubkeyClientHex),
            let peerIdHost = json["peer_id_host"] as? String,
            let peerIdClient = json["peer_id_client"] as? String
        else {
            throw NSError(domain: "ports_loopback_smoke", code: 8, userInfo: [
                NSLocalizedDescriptionKey: "fixture missing required fields at \(fixtureURL.path)",
            ])
        }
        return PortsLoopbackSmokeFixture(
            seedClient: seedClient,
            seedHost: seedHost,
            pubkeyHost: pubkeyHost,
            pubkeyClient: pubkeyClient,
            peerIdHost: peerIdHost,
            peerIdClient: peerIdClient
        )
    }
}

func runPortsLoopbackSmoke(_ r: Reporter) throws {
    let fixture = try PortsLoopbackSmokeFixture.load()

    // 1. Serving round-trips through a foreign PortsHandler (baseline +
    //    optional), the error rows, and post-containment survival.
    let pairA = loopbackTransportPair()
    let portsHandler = SmokePortsHandler()
    let responder = try connectResponderFfi(
        transport: LoopbackCallbackTransport(inner: pairA.server()),
        seed: fixture.seedHost,
        manifestJson: portsManifestJson(hostId: "test-responder"),
        allowlist: [fixture.peerIdClient],
        peerKeys: [fixture.peerIdClient: fixture.pubkeyClient],
        ports: portsHandler,
        invokeTimeoutMs: nil
    )
    let dialer = try connectRemoteAdapterFfi(
        transport: LoopbackCallbackTransport(inner: pairA.client()),
        localSeed: fixture.seedClient,
        localManifestJson: portsManifestJson(hostId: "test-client"),
        remotePubkey: fixture.pubkeyHost,
        allowlist: [fixture.peerIdHost],
        invokeTimeoutMs: nil
    )

    defer {
        dialer.close()
        responder.close()
        r.check("ports dialer state is Closed after close", dialer.state() == "Closed")
        r.check("ports responder state is Closed after close", responder.state() == "Closed")
    }

    r.check("ports dialer state is Established", dialer.state() == "Established")
    try waitForStatePorts("ports responder handshake", { responder.state() }, "Established")

    // 1a. Baseline round-trip through the foreign ports handler: put stores
    //     the entry JSON in the handler, get serves it back. The wire
    //     carries the canonicalized entry JSON (typed round-trip), so
    //     compare semantically, not byte-wise.
    let entryJson = knowledgeEntryJson(entryId: "kb_ffi_ports_put", canonicalName: "FFI Ports Put")
    let putJson = try dialer.putKnowledgeEntry(entryJson: entryJson, expectedBaseRevision: nil)
    r.check("put through the foreign ports handler", jsonStringField(putJson, "entry_id") == "kb_ffi_ports_put")
    let getJson = try dialer.getKnowledgeEntry(entryId: "kb_ffi_ports_put")
    r.check("get through the foreign ports handler", jsonStringField(getJson, "canonical_name") == "FFI Ports Put")

    // 1b. Application-reject passthrough: an unknown entry rejects with the
    //     handler's locked code + re-hung kind (ordinary deny, NOT
    //     containment).
    do {
        _ = try dialer.getKnowledgeEntry(entryId: "kb_ffi_ports_missing")
        r.check("unknown entry rejects", false)
    } catch let error as FfiError {
        if case let .Rejected(code, _, kind, wireCode) = error {
            r.check("unknown entry reject code is KNOWLEDGE_ENTRY_NOT_FOUND", code == "KNOWLEDGE_ENTRY_NOT_FOUND")
            r.check("unknown entry reject kind is store_miss (re-hung)", kind == "store_miss")
            r.check("unknown entry reject wire_code is nil", wireCode == nil)
        } else {
            r.check("unknown entry reject surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("unknown entry reject surfaces as FfiError (got \(error))", false)
    }

    // 1c. Optional dialer ops round-trip through the callback (l2-computable
    //     / l5-fork negotiated by both manifests).
    let projectJson = try dialer.project(projectRequestJson: projectRequestJson)
    r.check("project session_id round-trips", jsonStringField(projectJson, "session_id") == "sess_ffi_ports")
    r.check("project entry_id round-trips", jsonStringField(projectJson, "entry_id") == "kb_ffi_ports_proj")
    r.check("project computable tide_level", jsonDoubleField(projectJson, "computable.tide_level") == 2.4)
    r.check("project computable cargo_tons", jsonDoubleField(projectJson, "computable.cargo_tons") == 38)

    let computeJson = try dialer.compute(computeRequestJson: computeRequestJson)
    r.check("compute echoes the request computable", jsonDoubleField(computeJson, "computable.tide_level") == 2.5)
    r.check("compute settle state tide_level", jsonDoubleField(computeJson, "state.tide_level") == 2.5)

    let eventsJson = try dialer.listForkTimelineEvents(scopeJson: forkScopeJson)
    r.check("fork round-trip carries the fork event", jsonStringField(eventsJson, "0.timeline_event_id") == "evt_tw_ffi_storm")
    r.check("fork event fork_id round-trips", jsonStringField(eventsJson, "0.fork_id") == "fork_tw_ffi_events")

    // 1d. Malformed JSON is rejected locally (INVALID_INPUT, zero wire
    //     traffic) — the dialer pre-validation row per op.
    do {
        _ = try dialer.project(projectRequestJson: "{ not json")
        r.check("malformed project json rejects", false)
    } catch let error as FfiError {
        if case let .Rejected(code, _, _, wireCode) = error {
            r.check("malformed project json code is INVALID_INPUT", code == "INVALID_INPUT")
            r.check("malformed project json wire_code is nil", wireCode == nil)
        } else {
            r.check("malformed project json surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("malformed project json surfaces as FfiError (got \(error))", false)
    }

    // 1e. Foreign-fault containment: the handler faults on kb_ffi_ports_boom
    //     -> INTERNAL_ERROR with no details; the session survives and the
    //     serve loop answers the next healthy put.
    do {
        _ = try dialer.getKnowledgeEntry(entryId: "kb_ffi_ports_boom")
        r.check("foreign-fault handler is contained", false)
    } catch let error as FfiError {
        if case let .Rejected(code, _, kind, wireCode) = error {
            r.check("foreign-fault containment code is INTERNAL_ERROR", code == "INTERNAL_ERROR")
            r.check("foreign-fault containment kind is nil", kind == nil)
            r.check("foreign-fault containment wire_code is nil (details None)", wireCode == nil)
        } else {
            r.check("foreign-fault containment surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("foreign-fault containment surfaces as FfiError (got \(error))", false)
    }

    let healthyJson = try dialer.putKnowledgeEntry(
        entryJson: knowledgeEntryJson(entryId: "kb_ffi_ports_after", canonicalName: "After Containment"),
        expectedBaseRevision: nil
    )
    r.check("serve loop survives foreign-fault containment", jsonStringField(healthyJson, "entry_id") == "kb_ffi_ports_after")

    // 2. Absent-ports constructor is still valid (default deny): optional
    //    families negotiated, no PortsHandler — every optional op denies
    //    fail-closed with the preserved op_unsupported wire code.
    let pairB = loopbackTransportPair()
    let absentResponder = try connectResponderFfi(
        transport: LoopbackCallbackTransport(inner: pairB.server()),
        seed: fixture.seedHost,
        manifestJson: portsManifestJson(hostId: "test-responder"),
        allowlist: [fixture.peerIdClient],
        peerKeys: [fixture.peerIdClient: fixture.pubkeyClient],
        ports: nil,
        invokeTimeoutMs: nil
    )
    let absentDialer = try connectRemoteAdapterFfi(
        transport: LoopbackCallbackTransport(inner: pairB.client()),
        localSeed: fixture.seedClient,
        localManifestJson: portsManifestJson(hostId: "test-client"),
        remotePubkey: fixture.pubkeyHost,
        allowlist: [fixture.peerIdHost],
        invokeTimeoutMs: nil
    )

    defer {
        absentDialer.close()
        absentResponder.close()
        r.check("absent-ports dialer state is Closed after close", absentDialer.state() == "Closed")
        r.check("absent-ports responder state is Closed after close", absentResponder.state() == "Closed")
    }

    r.check("absent-ports dialer state is Established", absentDialer.state() == "Established")
    try waitForStatePorts("absent-ports responder handshake", { absentResponder.state() }, "Established")
    try assertOptionalOpsDenied(r, absentDialer, "absent-ports deny")

    // 3. Capability-gate deny: default manifests advertise spoke-baseline
    //    only, so the negotiated set lacks l2-computable / l5-fork and every
    //    optional op is denied at the responder's dispatch gate with the
    //    preserved op_unsupported wire code.
    let pairC = loopbackTransportPair()
    let gateResponder = try connectResponderFfi(
        transport: LoopbackCallbackTransport(inner: pairC.server()),
        seed: fixture.seedHost,
        manifestJson: baselineOnlyManifestJson(hostId: "test-responder"),
        allowlist: [fixture.peerIdClient],
        peerKeys: [fixture.peerIdClient: fixture.pubkeyClient],
        ports: nil,
        invokeTimeoutMs: nil
    )
    let gateDialer = try connectRemoteAdapterFfi(
        transport: LoopbackCallbackTransport(inner: pairC.client()),
        localSeed: fixture.seedClient,
        localManifestJson: baselineOnlyManifestJson(hostId: "test-client"),
        remotePubkey: fixture.pubkeyHost,
        allowlist: [fixture.peerIdHost],
        invokeTimeoutMs: nil
    )

    defer {
        gateDialer.close()
        gateResponder.close()
        r.check("capability-deny dialer state is Closed after close", gateDialer.state() == "Closed")
        r.check("capability-deny responder state is Closed after close", gateResponder.state() == "Closed")
    }

    r.check("capability-deny dialer state is Established", gateDialer.state() == "Established")
    try waitForStatePorts("capability-deny responder handshake", { gateResponder.state() }, "Established")
    try assertOptionalOpsDenied(r, gateDialer, "capability deny")
}

/// Assert all three optional ops deny with CAPABILITY_PORT_MISSING +
/// op_unsupported on the given dialer.
private func assertOptionalOpsDenied(_ r: Reporter, _ dialer: RemoteAdapterFfi, _ what: String) throws {
    let cases: [(String, () throws -> String)] = [
        ("project", { try dialer.project(projectRequestJson: projectRequestJson) }),
        ("compute", { try dialer.compute(computeRequestJson: computeRequestJson) }),
        ("listForkTimelineEvents", { try dialer.listForkTimelineEvents(scopeJson: forkScopeJson) }),
    ]
    for (name, invoke) in cases {
        do {
            _ = try invoke()
            r.check("\(what): \(name) denies", false)
        } catch let error as FfiError {
            if case let .Rejected(code, _, _, wireCode) = error {
                r.check("\(what): \(name) deny code is CAPABILITY_PORT_MISSING", code == "CAPABILITY_PORT_MISSING")
                r.check("\(what): \(name) deny wire_code is op_unsupported", wireCode == "op_unsupported")
            } else {
                r.check("\(what): \(name) deny surfaces as FfiError.Rejected (got \(error))", false)
            }
        } catch {
            r.check("\(what): \(name) deny surfaces as FfiError (got \(error))", false)
        }
    }
}

/// Bounded poll for the handshake to settle (D16 constructor semantics).
private func waitForStatePorts(_ what: String, _ state: () -> String, _ expected: String) throws {
    let deadline = Date().addingTimeInterval(5)
    var last = state()
    while last != expected {
        if Date() > deadline {
            throw NSError(domain: "ports_loopback_smoke", code: 4, userInfo: [
                NSLocalizedDescriptionKey: "\(what): timed out waiting for \(expected) (last: \(last))",
            ])
        }
        Thread.sleep(forTimeInterval: 0.01)
        last = state()
    }
}

/// Dotted-path JSON field lookup returning a string (nil-safe: returns nil on
/// malformed input or a missing field so the caller's check fails visibly).
private func jsonStringField(_ raw: String, _ path: String) -> String? {
    guard let data = raw.data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data),
          let value = jsonValueAt(object, path) else {
        return nil
    }
    if let string = value as? String {
        return string
    }
    return "\(value)"
}

/// Dotted-path JSON field lookup returning a Double (nil-safe).
private func jsonDoubleField(_ raw: String, _ path: String) -> Double? {
    guard let data = raw.data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data),
          let value = jsonValueAt(object, path) else {
        return nil
    }
    return (value as? NSNumber)?.doubleValue
}

private func jsonValueAt(_ root: Any, _ path: String) -> Any? {
    var current: Any = root
    for part in path.split(separator: ".") {
        if let array = current as? [Any], let index = Int(part), array.indices.contains(index) {
            current = array[index]
        } else if let object = current as? [String: Any], let value = object[String(part)] {
            current = value
        } else {
            return nil
        }
    }
    return current
}

/// Foreign-callback ports handler: in-memory knowledge store plus canned
/// optional-family answers; unknown entries reject with an application
/// `Rejected` (ordinary deny — not containment); `kb_ffi_ports_boom` faults
/// (the containment row). Mirror of the Rust `TestPortsHandler`.
private final class SmokePortsHandler: PortsHandler {
    private let lock = NSLock()
    private var entries: [String: [String: Any]] = [:]

    func getKnowledgeEntry(entryId: String) throws -> String {
        lock.lock()
        defer { lock.unlock() }
        if entryId == "kb_ffi_ports_boom" {
            throw NSError(domain: "ports_loopback_smoke", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "foreign ports handler fault (containment row)",
            ])
        }
        guard let entry = entries[entryId] else {
            throw FfiError.Rejected(
                code: "KNOWLEDGE_ENTRY_NOT_FOUND",
                message: "entry \(entryId) not found",
                kind: "store_miss",
                wireCode: nil
            )
        }
        let data = try JSONSerialization.data(withJSONObject: entry)
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    func putKnowledgeEntry(entryJson: String, expectedBaseRevision: UInt64?) throws -> String {
        lock.lock()
        defer { lock.unlock() }
        guard let data = entryJson.data(using: .utf8),
              let entry = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let entryId = entry["entry_id"] as? String else {
            throw NSError(domain: "ports_loopback_smoke", code: 2, userInfo: [
                NSLocalizedDescriptionKey: "handler received malformed entry JSON",
            ])
        }
        entries[entryId] = entry
        return entryJson
    }

    func getRelation(relationId: String) throws -> String {
        throw FfiError.Rejected(
            code: "INVALID_INPUT",
            message: "relation serving not exercised by this test handler",
            kind: nil,
            wireCode: nil
        )
    }

    func putRelation(relationJson: String, expectedBaseRevision: UInt64?) throws -> String {
        throw FfiError.Rejected(
            code: "INVALID_INPUT",
            message: "relation serving not exercised by this test handler",
            kind: nil,
            wireCode: nil
        )
    }

    func listKnowledgeEntries(scopeJson: String) throws -> String {
        lock.lock()
        defer { lock.unlock() }
        let data = try JSONSerialization.data(withJSONObject: Array(entries.values))
        return String(data: data, encoding: .utf8) ?? "[]"
    }

    func listTimelineEvents(scopeJson: String) throws -> String {
        "[]"
    }

    func putFindings(findingsJson: String) throws -> String {
        "[]"
    }

    func listRules(ruleRefs: [String]) throws -> String {
        "[]"
    }

    func listPeerHostCapabilityManifests() throws -> String {
        "[]"
    }

    func project(projectRequestJson: String) throws -> String {
        guard let data = projectRequestJson.data(using: .utf8),
              let request = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "ports_loopback_smoke", code: 3, userInfo: [
                NSLocalizedDescriptionKey: "handler received malformed project JSON",
            ])
        }
        let response: [String: Any] = [
            "session_id": request["session_id"] ?? "",
            "entry_id": request["entry_id"] ?? "",
            "computable": ["tide_level": 2.4, "cargo_tons": 38],
        ]
        let responseData = try JSONSerialization.data(withJSONObject: response)
        return String(data: responseData, encoding: .utf8) ?? "{}"
    }

    func compute(computeRequestJson: String) throws -> String {
        guard let data = computeRequestJson.data(using: .utf8),
              let request = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let computable = request["computable"] else {
            throw NSError(domain: "ports_loopback_smoke", code: 5, userInfo: [
                NSLocalizedDescriptionKey: "handler received malformed compute JSON",
            ])
        }
        let response: [String: Any] = [
            "session_id": request["session_id"] ?? "",
            "entry_id": request["entry_id"] ?? "",
            "computable": computable,
            "state": computable,
        ]
        let responseData = try JSONSerialization.data(withJSONObject: response)
        return String(data: responseData, encoding: .utf8) ?? "{}"
    }

    func listForkTimelineEvents(scopeJson: String) throws -> String {
        guard let data = scopeJson.data(using: .utf8),
              let scope = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "ports_loopback_smoke", code: 6, userInfo: [
                NSLocalizedDescriptionKey: "handler received malformed scope JSON",
            ])
        }
        guard scope["fork_id"] as? String == "fork_tw_ffi_events" else {
            return "[]"
        }
        let event: [String: Any] = [
            "schema_version": 1,
            "timeline_event_id": "evt_tw_ffi_storm",
            "canonical_name": "FFI Fork Storm",
            "fork_id": "fork_tw_ffi_events",
            "extensions": [:],
        ]
        let eventData = try JSONSerialization.data(withJSONObject: [event])
        return String(data: eventData, encoding: .utf8) ?? "[]"
    }
}
