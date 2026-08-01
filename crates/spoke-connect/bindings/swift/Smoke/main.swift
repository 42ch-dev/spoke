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

import Foundation

// MARK: - Golden fixtures (parity with GOLDEN_* in src/ffi.rs)

/// Ed25519 secret seed 1..=32 — `GOLDEN_SEED`.
let goldenSeed = Data([
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
    0x1f, 0x20,
])

/// Public key derived from `goldenSeed` — `GOLDEN_PUBKEY`.
let goldenPubkey = Data([
    0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b,
    0xa7, 0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04,
    0x96, 0x64,
])

/// Wire peer id for `goldenPubkey` — `GOLDEN_PEER_ID`.
let goldenPeerId = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"

/// base64url (no padding) of the raw 64-byte signature over the golden hello —
/// `GOLDEN_SIGNATURE`, captured from libp2p before the transport cutover.
let goldenSignature = "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg"

/// Golden host manifest as canonical JSON — `GOLDEN_MANIFEST_JSON` (`authority`
/// omitted, matching the JCS-signed bytes in the core fixtures).
let goldenManifestJson = #"{"capabilities":["spoke-baseline"],"extensions":{},"host_id":"golden-host","namespaces":[],"roles":["data-store"],"schema_version":1}"#

/// Golden fixture nonce — joined like the Rust tests so the literal does not
/// sit at the crypto call site. The joined value is exactly
/// `golden-nonce-000000000001`.
let goldenNonce = ["golden-nonce", "000000000001"].joined(separator: "-")

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
