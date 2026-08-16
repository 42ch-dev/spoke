// loopback_smoke.swift — RemoteAdapterFFI loopback round-trip over the callback
// Transport seam with the reference ToyWorld smoke host (parity with
// crates/spoke-connect/src/ffi.rs remote_adapter_ffi_tests). Smoke-host-only:
// the tool-faces loopback smoke (no smoke host needed) lives in
// tool_loopback_smoke.swift and runs in the default macOS smoke.

import Foundation

private struct LoopbackSmokeFixture {
    let seedClient: Data
    let pubkeyHost: Data
    let peerIdHost: String
    let clientManifestJson: String
    let sessionId: String
    let entryId: String
    let entryCanonicalName: String

    static func load() throws -> LoopbackSmokeFixture {
        let fixtureURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("fixtures/loopback-smoke.json")
        let data = try Data(contentsOf: fixtureURL)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "loopback_smoke", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "fixture is not a JSON object",
            ])
        }
        guard
            let seedClientHex = json["seed_client_hex"] as? String,
            let seedClient = hexData(seedClientHex),
            let pubkeyHostHex = json["pubkey_host_hex"] as? String,
            let pubkeyHost = hexData(pubkeyHostHex),
            let peerIdHost = json["peer_id_host"] as? String,
            let clientManifestJson = json["client_manifest_json"] as? String,
            let sessionId = json["session_id"] as? String,
            let entryId = json["entry_id"] as? String,
            let entryCanonicalName = json["entry_canonical_name"] as? String
        else {
            throw NSError(domain: "loopback_smoke", code: 2, userInfo: [
                NSLocalizedDescriptionKey: "fixture missing required fields at \(fixtureURL.path)",
            ])
        }
        return LoopbackSmokeFixture(
            seedClient: seedClient,
            pubkeyHost: pubkeyHost,
            peerIdHost: peerIdHost,
            clientManifestJson: clientManifestJson,
            sessionId: sessionId,
            entryId: entryId,
            entryCanonicalName: entryCanonicalName
        )
    }

    func knowledgeEntryJson() -> String {
        """
        {"schema_version":1,"entry_id":"\(entryId)","entry_type":"character","canonical_name":"\(entryCanonicalName)","status":"provisional","body":{"summary":"Upserted over the loopback: \(entryId)"},"extensions":{}}
        """
    }
}

func runLoopbackRemoteAdapterSmoke(_ r: Reporter) throws {
    let fixture = try LoopbackSmokeFixture.load()

    let pair = loopbackTransportPair()
    let host = startLoopbackSmokeHost(server: pair.server())

    let transport = LoopbackCallbackTransport(inner: pair.client())
    let adapter = try connectRemoteAdapterFfi(
        transport: transport,
        localSeed: fixture.seedClient,
        localManifestJson: fixture.clientManifestJson,
        remotePubkey: fixture.pubkeyHost,
        allowlist: [fixture.peerIdHost],
        invokeTimeoutMs: nil
    )

    r.check("RemoteAdapterFFI state is Established", adapter.state() == "Established")
    r.check("RemoteAdapterFFI session_id matches loopback host", adapter.sessionId() == fixture.sessionId)
    r.check("RemoteAdapterFFI session_id matches smoke host handle", adapter.sessionId() == host.sessionId())
    r.check("RemoteAdapterFFI remote_peer_id matches fixture host peer id", adapter.remotePeerId() == fixture.peerIdHost)

    let remoteManifest = adapter.remoteManifest()
    r.check("RemoteAdapterFFI remote_manifest is present", remoteManifest != nil)
    if let remoteManifest,
       let manifestData = remoteManifest.data(using: .utf8),
       let manifestJson = try? JSONSerialization.jsonObject(with: manifestData) as? [String: Any] {
        r.check("remote manifest host_id is test-host", manifestJson["host_id"] as? String == "test-host")
    } else {
        r.check("remote manifest parses as JSON", false)
    }

    let entryJson = fixture.knowledgeEntryJson()
    let putJson = try adapter.putKnowledgeEntry(entryJson: entryJson, expectedBaseRevision: nil)
    r.check("putKnowledgeEntry returns JSON", !putJson.isEmpty)
    if let putData = putJson.data(using: .utf8),
       let putObject = try? JSONSerialization.jsonObject(with: putData) as? [String: Any] {
        r.check("putKnowledgeEntry echoes entry_id", putObject["entry_id"] as? String == fixture.entryId)
    } else {
        r.check("putKnowledgeEntry response parses as JSON", false)
    }

    let getJson = try adapter.getKnowledgeEntry(entryId: fixture.entryId)
    r.check("getKnowledgeEntry returns JSON", !getJson.isEmpty)
    if let getData = getJson.data(using: .utf8),
       let getObject = try? JSONSerialization.jsonObject(with: getData) as? [String: Any] {
        r.check("getKnowledgeEntry round-trip entry_id", getObject["entry_id"] as? String == fixture.entryId)
        r.check(
            "getKnowledgeEntry round-trip canonical_name",
            getObject["canonical_name"] as? String == fixture.entryCanonicalName
        )
    } else {
        r.check("getKnowledgeEntry response parses as JSON", false)
    }

    adapter.close()
    r.check("RemoteAdapterFFI state is Closed after close", adapter.state() == "Closed")
    host.close()
}
