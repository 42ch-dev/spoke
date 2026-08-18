// tool_loopback_smoke.swift — tool faces over the loopback pair (D15/D16),
// run in the DEFAULT macOS smoke against the committed production binding
// (no smoke host needed): both ends are FFI objects — the responder serves a
// foreign `ToolHandler`, the dialer serves reverse invokes through
// `RemoteAdapterFFI.registerToolHandler`, unregistered tools deny with
// `op_unsupported`, and a handler-thrown application reject passes through
// verbatim (parity with crates/spoke-connect/src/ffi.rs
// connect_responder_ffi_tests).

import Foundation

private struct ToolLoopbackSmokeFixture {
    let seedClient: Data
    let seedHost: Data
    let pubkeyHost: Data
    let pubkeyClient: Data
    let peerIdHost: String
    let peerIdClient: String

    static func load() throws -> ToolLoopbackSmokeFixture {
        let fixtureURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("fixtures/loopback-smoke.json")
        let data = try Data(contentsOf: fixtureURL)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "tool_loopback_smoke", code: 1, userInfo: [
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
            throw NSError(domain: "tool_loopback_smoke", code: 2, userInfo: [
                NSLocalizedDescriptionKey: "fixture missing required fields at \(fixtureURL.path)",
            ])
        }
        return ToolLoopbackSmokeFixture(
            seedClient: seedClient,
            seedHost: seedHost,
            pubkeyHost: pubkeyHost,
            pubkeyClient: pubkeyClient,
            peerIdHost: peerIdHost,
            peerIdClient: peerIdClient
        )
    }
}

func runToolLoopbackSmoke(_ r: Reporter) throws {
    let fixture = try ToolLoopbackSmokeFixture.load()

    let pair = loopbackTransportPair()
    // The accept-side constructor returns immediately in `Handshaking` (D16):
    // the dialer hello is the sync point, so the smoke polls `state()`
    // (bounded) to `Established` before invoking; a handshake failure
    // surfaces as `Closed`, never a thrown constructor error.
    let responder = try connectResponderFfi(
        transport: LoopbackCallbackTransport(inner: pair.server()),
        seed: fixture.seedHost,
        manifestJson: toolManifestJson(hostId: "test-responder"),
        allowlist: [fixture.peerIdClient],
        peerKeys: [fixture.peerIdClient: fixture.pubkeyClient],
        ports: nil,
        invokeTimeoutMs: nil
    )
    let dialer = try connectRemoteAdapterFfi(
        transport: LoopbackCallbackTransport(inner: pair.client()),
        localSeed: fixture.seedClient,
        localManifestJson: toolManifestJson(hostId: "test-client"),
        remotePubkey: fixture.pubkeyHost,
        allowlist: [fixture.peerIdHost],
        invokeTimeoutMs: nil
    )

    defer {
        dialer.close()
        responder.close()
        r.check("tool dialer state is Closed after close", dialer.state() == "Closed")
        r.check("tool responder state is Closed after close", responder.state() == "Closed")
    }

    r.check("tool dialer state is Established", dialer.state() == "Established")
    try waitForState("tool responder handshake", { responder.state() }, "Established")

    // 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
    let responderSum = SumToolHandler()
    try responder.registerToolHandler(capabilityId: "tools.math.add", handler: responderSum)
    let sumJson = try dialer.invokeTool(capabilityId: "tools.math.add", argumentsJson: "{\"a\": 1, \"b\": 2}")
    r.check("dialer invoke_tool answered by responder foreign ToolHandler", parseSum(sumJson) == 3)
    r.check("responder handler invocation count is 1", responderSum.calls == 1)

    // 2. Responder FFI invoke_tool -> dialer-side handler registered via
    //    RemoteAdapterFFI.registerToolHandler.
    let dialerSum = SumToolHandler()
    try dialer.registerToolHandler(capabilityId: "tools.math.add", handler: dialerSum)
    let reverseSumJson = try responder.invokeTool(capabilityId: "tools.math.add", argumentsJson: "{\"a\": 21, \"b\": 21}")
    r.check("responder invoke_tool answered by dialer-side handler", parseSum(reverseSumJson) == 42)
    r.check("dialer handler invocation count is 1", dialerSum.calls == 1)

    // 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
    do {
        _ = try dialer.invokeTool(capabilityId: "tools.echo.boom", argumentsJson: "{}")
        r.check("unregistered tool is denied", false)
    } catch let error as FfiError {
        if case let .Rejected(code, _, _, wireCode) = error {
            r.check("unregistered tool deny code is CAPABILITY_PORT_MISSING", code == "CAPABILITY_PORT_MISSING")
            r.check("unregistered tool deny wire_code is op_unsupported", wireCode == "op_unsupported")
        } else {
            r.check("unregistered tool deny surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("unregistered tool deny surfaces as FfiError (got \(error))", false)
    }

    // 4. Handler-thrown application reject passes through verbatim (kind /
    //    wire_code re-hung onto details by the bridge).
    try dialer.registerToolHandler(
        capabilityId: "tools.echo.boom",
        handler: ThrowingToolHandler(
            error: FfiError.Rejected(
                code: "REVISION_CONFLICT",
                message: "foreign handler rejected",
                kind: nil,
                wireCode: "op_unsupported"
            )
        )
    )
    do {
        _ = try responder.invokeTool(capabilityId: "tools.echo.boom", argumentsJson: "{}")
        r.check("handler-thrown reject passes through", false)
    } catch let error as FfiError {
        if case let .Rejected(code, message, _, wireCode) = error {
            r.check("reject passthrough code is REVISION_CONFLICT", code == "REVISION_CONFLICT")
            r.check("reject passthrough message is verbatim", message == "foreign handler rejected")
            r.check("reject passthrough wire_code is op_unsupported", wireCode == "op_unsupported")
        } else {
            r.check("reject passthrough surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("reject passthrough surfaces as FfiError (got \(error))", false)
    }

    // 5. Handler rejects with an unknown code string -> the host observes the
    //    INTERNAL_ERROR downgrade (message preserved, details re-hung): the
    //    foreign code cannot be represented by the typed SpokeRejectCode, so
    //    the bridge falls back.
    try dialer.registerToolHandler(
        capabilityId: "tools.echo.boom",
        handler: ThrowingToolHandler(
            error: FfiError.Rejected(
                code: "NOT_A_WIRE_CODE",
                message: "unknown code message",
                kind: nil,
                wireCode: "op_unsupported"
            )
        )
    )
    do {
        _ = try responder.invokeTool(capabilityId: "tools.echo.boom", argumentsJson: "{}")
        r.check("unknown-code handler is downgraded", false)
    } catch let error as FfiError {
        if case let .Rejected(code, message, _, wireCode) = error {
            r.check("unknown-code downgrade code is INTERNAL_ERROR", code == "INTERNAL_ERROR")
            r.check("unknown-code downgrade message is preserved", message == "unknown code message")
            r.check("unknown-code downgrade wire_code is op_unsupported", wireCode == "op_unsupported")
        } else {
            r.check("unknown-code downgrade surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("unknown-code downgrade surfaces as FfiError (got \(error))", false)
    }

    // 6. Handler throws a foreign (non-Rejected) fault -> contained to
    //    INTERNAL_ERROR with no details; the session survives and the serve
    //    loop still answers the next healthy reverse invoke.
    try dialer.registerToolHandler(
        capabilityId: "tools.echo.boom",
        handler: ThrowingToolHandler(
            error: NSError(
                domain: "spoke-connect-smoke",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "foreign fault"]
            )
        )
    )
    do {
        _ = try responder.invokeTool(capabilityId: "tools.echo.boom", argumentsJson: "{}")
        r.check("foreign-fault handler is contained", false)
    } catch let error as FfiError {
        if case let .Rejected(code, _, _, wireCode) = error {
            r.check("foreign-fault containment code is INTERNAL_ERROR", code == "INTERNAL_ERROR")
            r.check("foreign-fault containment wire_code is nil (details None)", wireCode == nil)
        } else {
            r.check("foreign-fault containment surfaces as FfiError.Rejected (got \(error))", false)
        }
    } catch {
        r.check("foreign-fault containment surfaces as FfiError (got \(error))", false)
    }

    let healthyJson = try responder.invokeTool(capabilityId: "tools.math.add", argumentsJson: "{\"a\": 21, \"b\": 21}")
    r.check("serve loop survives foreign-fault containment", parseSum(healthyJson) == 42)
    r.check("dialer handler invocation count after containment is 2", dialerSum.calls == 2)
}

/// Tool-carrying manifest — every tool capability also sits in
/// `capabilities[]` so the negotiated set includes the `tools.*` ops (D13
/// dispatch gate). Mirror of the Rust `tool_manifest` test helper.
private func toolManifestJson(hostId: String) -> String {
    """
    {"schema_version":1,"host_id":"\(hostId)","roles":["data-store"],
    "capabilities":["spoke-baseline","tools.math.add","tools.echo.echo","tools.echo.boom"],
    "namespaces":["math","echo","toy_world"],"extensions":{},
    "tools":[
    {"schema_version":1,"capability_id":"tools.math.add","op":"tools.math.add","description":"Add two integers","input":{"type":"object"},"output":{"type":"object"}},
    {"schema_version":1,"capability_id":"tools.echo.echo","op":"tools.echo.echo","description":"Echo the arguments","input":{"type":"object"},"output":{"type":"object"}},
    {"schema_version":1,"capability_id":"tools.echo.boom","op":"tools.echo.boom","description":"Explodes","input":{"type":"object"},"output":{"type":"object"}}
    ]}
    """
}

/// Bounded poll for the handshake to settle (D16 constructor semantics).
private func waitForState(_ what: String, _ state: () -> String, _ expected: String) throws {
    let deadline = Date().addingTimeInterval(5)
    var last = state()
    while last != expected {
        if Date() > deadline {
            throw NSError(domain: "tool_loopback_smoke", code: 4, userInfo: [
                NSLocalizedDescriptionKey: "\(what): timed out waiting for \(expected) (last: \(last))",
            ])
        }
        Thread.sleep(forTimeInterval: 0.01)
        last = state()
    }
}

/// Parse `{ "sum": N }` from a tool result JSON string (nil-safe: returns
/// nil on malformed input so the caller's check fails visibly).
private func parseSum(_ resultJson: String) -> Int64? {
    guard
        let data = resultJson.data(using: .utf8),
        let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else {
        return nil
    }
    return (object["sum"] as? NSNumber)?.int64Value
}

/// Foreign-callback tool handler: sums `a` + `b` (Rust `add_handler` parity)
/// and records the invocation count.
private final class SumToolHandler: ToolHandler {
    private let lock = NSLock()
    private var _calls = 0

    var calls: Int {
        lock.lock()
        defer { lock.unlock() }
        return _calls
    }

    func handle(argumentsJson: String) throws -> String {
        lock.lock()
        _calls += 1
        lock.unlock()
        let args = try? JSONSerialization.jsonObject(with: Data(argumentsJson.utf8)) as? [String: Any]
        let a = (args?["a"] as? NSNumber)?.int64Value ?? 0
        let b = (args?["b"] as? NSNumber)?.int64Value ?? 0
        return "{\"sum\": \(a + b)}"
    }
}

/// Foreign-callback tool handler that always throws the given application
/// reject (D16 passthrough row).
private final class ThrowingToolHandler: ToolHandler {
    private let error: Error

    init(error: Error) {
        self.error = error
    }

    func handle(argumentsJson: String) throws -> String {
        throw error
    }
}
