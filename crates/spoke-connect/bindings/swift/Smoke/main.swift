// main.swift — macOS smoke for the spoke-connect Swift bindings (uniffi).
//
// Exercises the exported sync-core facade from Swift and asserts golden-vector
// parity with the Rust core. The golden constants below are byte-identical to
// the fixtures in crates/spoke-connect/src/ffi.rs (tests module), so a pass
// here means the Swift surface produces the same peer id and hello signature
// as the Rust reference.
//
// Build & run (from the repository root):
//
//   # 1. Build the cdylib that carries the exported-surface metadata.
//   cargo build -p spoke-connect --features ffi
//
//   # 2. Regenerate the Swift bindings from the cdylib.
//   cargo run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
//     generate --library target/debug/libspoke_connect.dylib \
//     --language swift --out-dir crates/spoke-connect/bindings/swift/generated
//
//   # 3. Point the dylib install name at @rpath (cargo bakes in the absolute
//   #    deps-dir path, which would pin the smoke to one machine).
//   install_name_tool -id @rpath/libspoke_connect.dylib target/debug/libspoke_connect.dylib
//
//   # 4. Compile the smoke (Swift 5 language mode keeps top-level code
//   #    simple; `-fmodule-map-file` is required — the Clang importer does
//   #    not discover the uniffi module map from `-I` alone).
//   swiftc -Xcc -fmodule-map-file="$PWD/crates/spoke-connect/bindings/swift/generated/spoke_connectFFI.modulemap" \
//     -L target/debug -lspoke_connect \
//     -Xlinker -rpath -Xlinker "$PWD/target/debug" \
//     -swift-version 5 \
//     -o crates/spoke-connect/bindings/swift/Smoke/smoke \
//     crates/spoke-connect/bindings/swift/Smoke/main.swift \
//     crates/spoke-connect/bindings/swift/generated/spoke_connect.swift
//
//   # 5. Run it.
//   ./crates/spoke-connect/bindings/swift/Smoke/smoke
//
// See README.md in this directory for the same sequence.
//
// Note: a plain `cargo build` (default features) between steps replaces the
// ffi cdylib in the shared `target/debug`. If the smoke fails with
// `dyld: Symbol not found: _ffi_spoke_connect_rustbuffer_free`, re-run step 1
// (`cargo build -p spoke-connect --features ffi`) before recompiling.

import Foundation

// MARK: - Golden fixtures (shared SSOT copy)

/// Decode a lowercase hex string into raw bytes (`nil` on malformed input).
func hexData(_ hex: String) -> Data? {
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

// MARK: - Reporter

final class Reporter {
    private(set) var passed = 0

    func check(_ name: String, _ condition: Bool) {
        if condition {
            passed += 1
            print("PASS  \(name)")
        } else {
            print("FAIL  \(name)")
            exit(1)
        }
    }
}

// MARK: - Exercises

func run() throws {
    // Load the golden hello vector from the registered byte-identical copy
    // of the crate SSOT (`crates/spoke-connect/tests/fixtures/golden-hello.json`)
    // next to this smoke. `#filePath` resolves at compile time; the smoke is
    // compiled and run from the repository root (see the header comment).
    let fixtureURL = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent("fixtures/golden-hello.json")
    let goldenData = try Data(contentsOf: fixtureURL)
    guard
        let golden = (try? JSONSerialization.jsonObject(with: goldenData))
            as? [String: Any],
        let seedHex = golden["seed_hex"] as? String,
        let seedData = hexData(seedHex),
        let pubkeyHex = golden["pubkey_hex"] as? String,
        let pubkeyData = hexData(pubkeyHex),
        let goldenPeerId = golden["peer_id"] as? String,
        let goldenSignature = golden["signature_b64u"] as? String,
        let goldenManifestJson = golden["manifest_json"] as? String,
        let goldenNonce = golden["nonce"] as? String
    else {
        print("ERROR: cannot load golden fixture at \(fixtureURL.path)")
        exit(1)
    }
    // The fixture carries the seed / nonce / manifest inputs AND the pinned
    // output bytes (pubkey, peer id, JCS hex, signature) — asserted below,
    // never recomputed and written back.
    let goldenSeed = seedData
    let goldenPubkey = pubkeyData

    let r = Reporter()

    // 1. Golden peer id parity: derive from the golden public key.
    let derivedPeerId = try derivePeerIdFromEd25519Pubkey(pubkey: goldenPubkey)
    r.check("derivePeerIdFromEd25519Pubkey matches golden peer id", derivedPeerId == goldenPeerId)

    // 2. Sign the golden hello, then verify it round-trips.
    let helloJson = try signHelloEd25519(secret: goldenSeed, nonce: goldenNonce, hostJson: goldenManifestJson)
    r.check("signHelloEd25519 returns a JSON envelope", !helloJson.isEmpty)

    // 3. Full golden signature parity: the envelope must carry the exact
    //    golden peer id and base64url signature (same host/nonce as the golden
    //    manifest, so the signature is byte-reproducible).
    let hello = try JSONSerialization.jsonObject(with: Data(helloJson.utf8)) as? [String: Any]
    guard let helloObject = hello else {
        r.check("signed envelope parses as JSON", false)
        return
    }
    r.check("hello peer_id equals golden", helloObject["peer_id"] as? String == goldenPeerId)
    r.check("hello signature equals golden base64url", helloObject["signature"] as? String == goldenSignature)

    // 4. Sign → verify round trip through the FFI boundary.
    try verifyHelloEd25519(publicKey: goldenPubkey, expectedPeerId: goldenPeerId, helloJson: helloJson)
    r.check("verifyHelloEd25519 accepts the signed hello", true)

    // 5. Error path: a tampered hello fails verification with the mapped case.
    let tampered = helloJson.replacingOccurrences(of: "data-store", with: "checker")
    do {
        try verifyHelloEd25519(publicKey: goldenPubkey, expectedPeerId: goldenPeerId, helloJson: tampered)
        r.check("verifyHelloEd25519 rejects a tampered hello", false)
    } catch let error as CoreError {
        switch error {
        case .InvalidHelloSignature:
            r.check("verifyHelloEd25519 rejects a tampered hello with CoreError.InvalidHelloSignature", true)
        default:
            r.check("tampered hello maps to CoreError.InvalidHelloSignature (got \(error))", false)
        }
    } catch {
        r.check("tampered hello surfaces as CoreError (got \(error))", false)
    }

    // 6. Allowlist fails closed.
    let allowlist = ["peer-a", "peer-b"]
    r.check("isAllowlisted accepts an allowlisted peer", isAllowlisted(allowlist: allowlist, peerId: "peer-a"))
    r.check("isAllowlisted rejects an unknown peer", !isAllowlisted(allowlist: allowlist, peerId: "peer-c"))
    r.check("isAllowlisted fails closed on an empty allowlist", !isAllowlisted(allowlist: [], peerId: "peer-a"))

    // 7. Outbound sequence allocates 0, then 1.
    let outbound = OutboundSequence()
    r.check("outboundSequence first allocate is 0", (try outbound.allocate()) == 0)
    r.check("outboundSequence second allocate is 1", (try outbound.allocate()) == 1)

    // 8. Inbound sequence advances and rejects replays.
    let inbound = InboundSequence()
    r.check("inboundSequence advance(0) returns next expectation 1", (try inbound.advance(sequence: 0)) == 1)
    do {
        _ = try inbound.advance(sequence: 0)
        r.check("inboundSequence rejects a replayed sequence", false)
    } catch let error as CoreInvokeError {
        switch error {
        case .InboundSequenceMismatch(expected: 1, actual: 0):
            r.check("inboundSequence rejects a replayed sequence with InboundSequenceMismatch(1, 0)", true)
        default:
            r.check("inboundSequence replay maps to InboundSequenceMismatch (got \(error))", false)
        }
    } catch {
        r.check("inboundSequence replay surfaces as CoreInvokeError (got \(error))", false)
    }

    // 9. Nonce store: record once, then the replay is rejected.
    let nonceStore = NonceStore()
    r.check("NonceStore.checkAndRecord accepts a fresh nonce", nonceStore.checkAndRecord(peerId: "peer-a", nonce: "nonce-1"))
    r.check("NonceStore.checkAndRecord rejects the replay", !nonceStore.checkAndRecord(peerId: "peer-a", nonce: "nonce-1"))
    r.check("NonceStore scopes nonces per sender", nonceStore.checkAndRecord(peerId: "peer-b", nonce: "nonce-1"))

    // 10. Dispatch gate.
    r.check("dispatchAllowed(check, spoke-baseline) is true", dispatchAllowed(op: "check", negotiatedCapabilities: ["spoke-baseline"]))
    r.check("dispatchAllowed(check, l2-computable) is false", !dispatchAllowed(op: "check", negotiatedCapabilities: ["l2-computable"]))
    r.check("dispatchAllowed(custom-op, spoke-baseline) is false", !dispatchAllowed(op: "custom-op", negotiatedCapabilities: ["spoke-baseline"]))

    // 11. Required capability lookup.
    r.check("requiredCapability(check) is spoke-baseline", requiredCapability(op: "check") == "spoke-baseline")
    r.check("requiredCapability(project) is l2-computable", requiredCapability(op: "project") == "l2-computable")
    r.check("requiredCapability(custom-op) is nil", requiredCapability(op: "custom-op") == nil)

    // 12. Protocol version.
    r.check("protocolVersion() is 1", protocolVersion() == 1)

    // 13. Response correlation: exact echo passes, mismatch throws.
    try checkResponseCorrelation(
        expectedSessionId: "sess-1", expectedSequence: 0, expectedRequestId: "req-1",
        actualSessionId: "sess-1", actualSequence: 0, actualRequestId: "req-1"
    )
    r.check("checkResponseCorrelation accepts an exact echo", true)
    do {
        try checkResponseCorrelation(
            expectedSessionId: "sess-1", expectedSequence: 0, expectedRequestId: "req-1",
            actualSessionId: "sess-1", actualSequence: 1, actualRequestId: "req-1"
        )
        r.check("checkResponseCorrelation rejects a sequence mismatch", false)
    } catch let error as CoreInvokeError {
        switch error {
        case .CorrelationMismatch:
            r.check("checkResponseCorrelation rejects a mismatch with CoreInvokeError.CorrelationMismatch", true)
        default:
            r.check("correlation mismatch maps to CorrelationMismatch (got \(error))", false)
        }
    } catch {
        r.check("correlation mismatch surfaces as CoreInvokeError (got \(error))", false)
    }

    print("\(r.passed) checks passed")
}

do {
    try run()
} catch {
    print("ERROR: \(error)")
    exit(1)
}
