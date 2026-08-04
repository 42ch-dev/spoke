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
}
