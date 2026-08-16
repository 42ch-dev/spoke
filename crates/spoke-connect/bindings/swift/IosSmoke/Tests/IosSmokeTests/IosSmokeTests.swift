// IosSmokeTests.swift — iOS simulator smoke for the spoke-connect Swift
// bindings (uniffi), exercising the committed `spoke_connectFFI.xcframework`
// simulator slice through the root `SpokeConnect` SPM product.
//
// Asserts the same golden triad as the macOS smoke: golden peer id, golden
// base64url hello signature, and protocol version 1. The golden fixture is a
// byte-identical copy of `crates/spoke-connect/tests/fixtures/golden-hello.json`
// (registered in `tooling/connect/golden-vector-sync.mjs`).
//
// Run (from the IosSmoke package directory):
//
//   xcodebuild test -scheme IosSmoke-Package \
//     -destination 'platform=iOS Simulator,name=iPhone 17' \
//     -derivedDataPath <tmp>
//
// Named simulator devices vary per Xcode install; `-destination
// 'generic/platform=iOS Simulator'` is the portable fallback.

import XCTest
import SpokeConnect

final class IosSmokeTests: XCTestCase {
    private struct Golden {
        let seed: Data
        let pubkey: Data
        let peerId: String
        let signature: String
        let manifestJson: String
        let nonce: String
    }

    /// Decode a lowercase hex string into raw bytes (`nil` on malformed input).
    private func hexData(_ hex: String) -> Data? {
        guard hex.count % 2 == 0 else { return nil }
        var out = Data()
        var index = hex.startIndex
        while index < hex.endIndex {
            let end = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<end], radix: 16) else { return nil }
            out.append(byte)
            index = end
        }
        return out
    }

    private func loadGolden() throws -> Golden {
        let url = try XCTUnwrap(
            Bundle.module.url(forResource: "golden-hello", withExtension: "json", subdirectory: "fixtures"),
            "missing golden fixture resource"
        )
        let data = try Data(contentsOf: url)
        let json = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: data) as? [String: Any],
            "fixture is not a JSON object"
        )
        return Golden(
            seed: try XCTUnwrap(hexData(XCTUnwrap(json["seed_hex"] as? String))),
            pubkey: try XCTUnwrap(hexData(XCTUnwrap(json["pubkey_hex"] as? String))),
            peerId: try XCTUnwrap(json["peer_id"] as? String),
            signature: try XCTUnwrap(json["signature_b64u"] as? String),
            manifestJson: try XCTUnwrap(json["manifest_json"] as? String),
            nonce: try XCTUnwrap(json["nonce"] as? String)
        )
    }

    func testGoldenPeerIdParity() throws {
        let golden = try loadGolden()
        let derived = try derivePeerIdFromEd25519Pubkey(pubkey: golden.pubkey)
        XCTAssertEqual(derived, golden.peerId)
    }

    func testGoldenHelloSignatureParity() throws {
        let golden = try loadGolden()
        let helloJson = try signHelloEd25519(
            secret: golden.seed, nonce: golden.nonce, hostJson: golden.manifestJson
        )
        let hello = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(helloJson.utf8)) as? [String: Any]
        )
        XCTAssertEqual(hello["peer_id"] as? String, golden.peerId)
        XCTAssertEqual(hello["signature"] as? String, golden.signature)
    }

    func testProtocolVersion() {
        XCTAssertEqual(protocolVersion(), 1)
    }

    // ── Tool faces over the loopback pair (D15/D16) ────────────────────────
    //
    // Both ends are FFI objects over the in-repo loopback transport pair: the
    // responder serves a foreign `ToolHandler`, the dialer serves reverse
    // invokes through `RemoteAdapterFFI.registerToolHandler`, unregistered
    // tools deny with `op_unsupported`, and a handler-thrown application
    // reject passes through verbatim. No smoke host is required — every face
    // is on the committed production xcframework.

    private struct LoopbackFixture {
        let seedClient: Data
        let seedHost: Data
        let pubkeyHost: Data
        let pubkeyClient: Data
        let peerIdHost: String
        let peerIdClient: String
    }

    private func loadLoopback() throws -> LoopbackFixture {
        let url = try XCTUnwrap(
            Bundle.module.url(forResource: "loopback-smoke", withExtension: "json", subdirectory: "fixtures"),
            "missing loopback fixture resource"
        )
        let data = try Data(contentsOf: url)
        let json = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: data) as? [String: Any],
            "fixture is not a JSON object"
        )
        return LoopbackFixture(
            seedClient: try XCTUnwrap(hexData(XCTUnwrap(json["seed_client_hex"] as? String))),
            seedHost: try XCTUnwrap(hexData(XCTUnwrap(json["seed_host_hex"] as? String))),
            pubkeyHost: try XCTUnwrap(hexData(XCTUnwrap(json["pubkey_host_hex"] as? String))),
            pubkeyClient: try XCTUnwrap(hexData(XCTUnwrap(json["pubkey_client_hex"] as? String))),
            peerIdHost: try XCTUnwrap(json["peer_id_host"] as? String),
            peerIdClient: try XCTUnwrap(json["peer_id_client"] as? String)
        )
    }

    private final class LoopbackCallbackTransport: Transport {
        private let inner: LoopbackTransport

        init(inner: LoopbackTransport) {
            self.inner = inner
        }

        func send(envelope: Data) throws {
            try inner.send(envelope: envelope)
        }

        func recv() throws -> Data {
            try inner.recv()
        }

        func close() throws {
            try inner.close()
        }
    }

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

    private final class ThrowingToolHandler: ToolHandler {
        private let error: Error

        init(error: Error) {
            self.error = error
        }

        func handle(argumentsJson: String) throws -> String {
            throw error
        }
    }

    private func toolManifestJson(hostId: String) -> String {
        let manifest: [String: Any] = [
            "schema_version": 1,
            "host_id": hostId,
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline", "tools.math.add", "tools.echo.echo", "tools.echo.boom"],
            "namespaces": ["math", "echo", "toy_world"],
            "extensions": [:],
            "tools": [
                [
                    "schema_version": 1,
                    "capability_id": "tools.math.add",
                    "op": "tools.math.add",
                    "description": "Add two integers",
                    "input": ["type": "object"],
                    "output": ["type": "object"],
                ],
                [
                    "schema_version": 1,
                    "capability_id": "tools.echo.echo",
                    "op": "tools.echo.echo",
                    "description": "Echo the arguments",
                    "input": ["type": "object"],
                    "output": ["type": "object"],
                ],
                [
                    "schema_version": 1,
                    "capability_id": "tools.echo.boom",
                    "op": "tools.echo.boom",
                    "description": "Explodes",
                    "input": ["type": "object"],
                    "output": ["type": "object"],
                ],
            ],
        ]
        let data = try! JSONSerialization.data(withJSONObject: manifest)
        return String(data: data, encoding: .utf8)!
    }

    private func waitForState(_ what: String, _ state: () -> String, _ expected: String) throws {
        let deadline = Date().addingTimeInterval(5)
        var last = state()
        while last != expected {
            if Date() > deadline {
                throw NSError(domain: "ios_smoke", code: 4, userInfo: [
                    NSLocalizedDescriptionKey: "\(what): timed out waiting for \(expected) (last: \(last))",
                ])
            }
            Thread.sleep(forTimeInterval: 0.01)
            last = state()
        }
    }

    private func parseSum(_ resultJson: String) -> Int64? {
        guard
            let data = resultJson.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return (object["sum"] as? NSNumber)?.int64Value
    }

    func testToolLoopbackFfiPair() throws {
        let fixture = try loadLoopback()

        let pair = loopbackTransportPair()
        // The accept-side constructor returns immediately in `Handshaking`
        // (D16): the dialer hello is the sync point, so the test polls
        // `state()` (bounded) to `Established` before invoking; a handshake
        // failure surfaces as `Closed`, never a thrown constructor error.
        let responder = try connectResponderFfi(
            transport: LoopbackCallbackTransport(inner: pair.server()),
            seed: fixture.seedHost,
            manifestJson: toolManifestJson(hostId: "test-responder"),
            allowlist: [fixture.peerIdClient],
            peerKeys: [fixture.peerIdClient: fixture.pubkeyClient],
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
            XCTAssertEqual(dialer.state(), "Closed", "tool dialer state after close")
            XCTAssertEqual(responder.state(), "Closed", "tool responder state after close")
        }

        XCTAssertEqual(dialer.state(), "Established")
        try waitForState("tool responder handshake", { responder.state() }, "Established")

        // 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
        let responderSum = SumToolHandler()
        try responder.registerToolHandler(capabilityId: "tools.math.add", handler: responderSum)
        let sumJson = try dialer.invokeTool(capabilityId: "tools.math.add", argumentsJson: "{\"a\": 1, \"b\": 2}")
        XCTAssertEqual(parseSum(sumJson), 3, "dialer invoke_tool answered by responder foreign ToolHandler")
        XCTAssertEqual(responderSum.calls, 1, "responder handler invocation count")

        // 2. Responder FFI invoke_tool -> dialer-side handler registered via
        //    RemoteAdapterFFI.registerToolHandler.
        let dialerSum = SumToolHandler()
        try dialer.registerToolHandler(capabilityId: "tools.math.add", handler: dialerSum)
        let reverseSumJson = try responder.invokeTool(capabilityId: "tools.math.add", argumentsJson: "{\"a\": 21, \"b\": 21}")
        XCTAssertEqual(parseSum(reverseSumJson), 42, "responder invoke_tool answered by dialer-side handler")
        XCTAssertEqual(dialerSum.calls, 1, "dialer handler invocation count")

        // 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
        do {
            _ = try dialer.invokeTool(capabilityId: "tools.echo.boom", argumentsJson: "{}")
            XCTFail("unregistered tool must be denied")
        } catch let error as FfiError {
            if case let .Rejected(code, _, _, wireCode) = error {
                XCTAssertEqual(code, "CAPABILITY_PORT_MISSING", "unregistered tool deny code")
                XCTAssertEqual(wireCode, "op_unsupported", "unregistered tool deny wire_code")
            } else {
                XCTFail("unregistered tool deny surfaced as \(error), expected FfiError.Rejected")
            }
        } catch {
            XCTFail("unregistered tool deny surfaced as \(error), expected FfiError")
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
            XCTFail("handler-thrown reject must pass through")
        } catch let error as FfiError {
            if case let .Rejected(code, message, _, wireCode) = error {
                XCTAssertEqual(code, "REVISION_CONFLICT", "reject passthrough code")
                XCTAssertEqual(message, "foreign handler rejected", "reject passthrough message")
                XCTAssertEqual(wireCode, "op_unsupported", "reject passthrough wire_code")
            } else {
                XCTFail("reject passthrough surfaced as \(error), expected FfiError.Rejected")
            }
        } catch {
            XCTFail("reject passthrough surfaced as \(error), expected FfiError")
        }
    }
}
