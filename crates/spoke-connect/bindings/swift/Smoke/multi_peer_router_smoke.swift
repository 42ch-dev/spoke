// multi_peer_router_smoke.swift — MultiPeerRouterFFI routing over loopback peers
// (parity with crates/spoke-connect/src/ffi.rs multi_peer_router_ffi_tests).

import Foundation

private struct MultiPeerRouterSmokeFixture {
    let seedClient: Data
    let peerIdClient: String
    let clientManifestJson: String
    let baselineHostSeed: Data
    let baselinePubkeyHost: Data
    let baselinePeerIdHost: String
    let baselineManifestJson: String
    let computableHostSeed: Data
    let computablePubkeyHost: Data
    let computablePeerIdHost: String
    let computableManifestJson: String
    let alphaHostSeed: Data
    let alphaPubkeyHost: Data
    let alphaPeerIdHost: String
    let alphaManifestJson: String
    let betaHostSeed: Data
    let betaPubkeyHost: Data
    let betaPeerIdHost: String
    let betaManifestJson: String
    let upsertEntryId: String
    let upsertEntryCanonicalName: String
    let tiebreakEntryId: String
    let tiebreakEntryCanonicalName: String
    let noMatchEntryId: String
    let noMatchEntryCanonicalName: String

    static func load() throws -> MultiPeerRouterSmokeFixture {
        let fixtureURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("fixtures/multi-peer-router-smoke.json")
        let data = try Data(contentsOf: fixtureURL)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(domain: "multi_peer_router_smoke", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "fixture is not a JSON object",
            ])
        }
        func req(_ key: String) throws -> String {
            guard let value = json[key] as? String else {
                throw NSError(domain: "multi_peer_router_smoke", code: 2, userInfo: [
                    NSLocalizedDescriptionKey: "fixture missing \(key) at \(fixtureURL.path)",
                ])
            }
            return value
        }
        func reqData(_ key: String) throws -> Data {
            let hex = try req(key)
            guard let data = hexData(hex) else {
                throw NSError(domain: "multi_peer_router_smoke", code: 3, userInfo: [
                    NSLocalizedDescriptionKey: "fixture invalid hex for \(key)",
                ])
            }
            return data
        }
        return MultiPeerRouterSmokeFixture(
            seedClient: try reqData("seed_client_hex"),
            peerIdClient: try req("peer_id_client"),
            clientManifestJson: try req("client_manifest_json"),
            baselineHostSeed: try reqData("baseline_host_seed_hex"),
            baselinePubkeyHost: try reqData("baseline_pubkey_host_hex"),
            baselinePeerIdHost: try req("baseline_peer_id_host"),
            baselineManifestJson: try req("baseline_manifest_json"),
            computableHostSeed: try reqData("computable_host_seed_hex"),
            computablePubkeyHost: try reqData("computable_pubkey_host_hex"),
            computablePeerIdHost: try req("computable_peer_id_host"),
            computableManifestJson: try req("computable_manifest_json"),
            alphaHostSeed: try reqData("alpha_host_seed_hex"),
            alphaPubkeyHost: try reqData("alpha_pubkey_host_hex"),
            alphaPeerIdHost: try req("alpha_peer_id_host"),
            alphaManifestJson: try req("alpha_manifest_json"),
            betaHostSeed: try reqData("beta_host_seed_hex"),
            betaPubkeyHost: try reqData("beta_pubkey_host_hex"),
            betaPeerIdHost: try req("beta_peer_id_host"),
            betaManifestJson: try req("beta_manifest_json"),
            upsertEntryId: try req("upsert_entry_id"),
            upsertEntryCanonicalName: try req("upsert_entry_canonical_name"),
            tiebreakEntryId: try req("tiebreak_entry_id"),
            tiebreakEntryCanonicalName: try req("tiebreak_entry_canonical_name"),
            noMatchEntryId: try req("no_match_entry_id"),
            noMatchEntryCanonicalName: try req("no_match_entry_canonical_name")
        )
    }

    func knowledgeEntryJson(entryId: String, canonicalName: String) -> String {
        let summary = "Upserted over the loopback: \(entryId)"
        return """
        {"schema_version":1,"entry_id":"\(entryId)","entry_type":"character","canonical_name":"\(canonicalName)","status":"provisional","body":{"summary":"\(summary)"},"extensions":{}}
        """
    }
}

private func dialLoopbackPeer(
    fixture: MultiPeerRouterSmokeFixture,
    hostSeed: Data,
    hostPubkey: Data,
    hostPeerId: String,
    hostManifestJson: String
) throws -> (RemoteAdapterFfi, LoopbackSmokeHost) {
    let pair = loopbackTransportPair()
    let host = try startLoopbackSmokeHostVariant(
        server: pair.server(),
        hostSeed: hostSeed,
        hostManifestJson: hostManifestJson
    )
    let transport = LoopbackCallbackTransport(inner: pair.client())
    let adapter = try connectRemoteAdapterFfi(
        transport: transport,
        localSeed: fixture.seedClient,
        localManifestJson: fixture.clientManifestJson,
        remotePubkey: hostPubkey,
        allowlist: [hostPeerId],
        invokeTimeoutMs: nil
    )
    return (adapter, host)
}

private func orchestrateUpsertViaRouter(
    router: MultiPeerRouterFfi,
    entryId: String,
    canonicalName: String,
    fixture: MultiPeerRouterSmokeFixture
) throws {
    let entryJson = fixture.knowledgeEntryJson(entryId: entryId, canonicalName: canonicalName)
    do {
        _ = try router.getKnowledgeEntry(entryId: entryId)
    } catch let error as FfiError {
        switch error {
        case .Rejected(let code, _, _, _):
            if code != "KNOWLEDGE_ENTRY_NOT_FOUND" {
                throw error
            }
        default:
            throw error
        }
    }
    _ = try router.putKnowledgeEntry(entryJson: entryJson, expectedBaseRevision: nil)
}

private func adapterHasEntry(adapter: RemoteAdapterFfi, entryId: String) -> Bool {
    do {
        _ = try adapter.getKnowledgeEntry(entryId: entryId)
        return true
    } catch let error as FfiError {
        switch error {
        case .Rejected(let code, _, _, _):
            // F-001: explicit false for any Rejected — do not invert to `code != "KNOWLEDGE_ENTRY_NOT_FOUND"`.
            _ = code
            return false
        default:
            return false
        }
    } catch {
        return false
    }
}

func runMultiPeerRouterSmoke(_ r: Reporter) throws {
    let fixture = try MultiPeerRouterSmokeFixture.load()

    let (baselineAdapter, baselineHost) = try dialLoopbackPeer(
        fixture: fixture,
        hostSeed: fixture.baselineHostSeed,
        hostPubkey: fixture.baselinePubkeyHost,
        hostPeerId: fixture.baselinePeerIdHost,
        hostManifestJson: fixture.baselineManifestJson
    )
    let (computableAdapter, computableHost) = try dialLoopbackPeer(
        fixture: fixture,
        hostSeed: fixture.computableHostSeed,
        hostPubkey: fixture.computablePubkeyHost,
        hostPeerId: fixture.computablePeerIdHost,
        hostManifestJson: fixture.computableManifestJson
    )

    let router = newMultiPeerRouterFfi()
    _ = try router.registerPeer(adapter: baselineAdapter)
    _ = try router.registerPeer(adapter: computableAdapter)
    r.check("router lists two peers", router.listPeers().count == 2)

    try orchestrateUpsertViaRouter(
        router: router,
        entryId: fixture.upsertEntryId,
        canonicalName: fixture.upsertEntryCanonicalName,
        fixture: fixture
    )
    r.check(
        "baseline peer stores upsert routed by capability",
        adapterHasEntry(adapter: baselineAdapter, entryId: fixture.upsertEntryId)
    )
    r.check(
        "computable peer did not receive baseline upsert",
        !adapterHasEntry(adapter: computableAdapter, entryId: fixture.upsertEntryId)
    )

    baselineAdapter.close()
    computableAdapter.close()
    baselineHost.close()
    computableHost.close()

    let (alphaAdapter, alphaHost) = try dialLoopbackPeer(
        fixture: fixture,
        hostSeed: fixture.alphaHostSeed,
        hostPubkey: fixture.alphaPubkeyHost,
        hostPeerId: fixture.alphaPeerIdHost,
        hostManifestJson: fixture.alphaManifestJson
    )
    let (betaAdapter, betaHost) = try dialLoopbackPeer(
        fixture: fixture,
        hostSeed: fixture.betaHostSeed,
        hostPubkey: fixture.betaPubkeyHost,
        hostPeerId: fixture.betaPeerIdHost,
        hostManifestJson: fixture.betaManifestJson
    )

    let tieRouter = newMultiPeerRouterFfi()
    _ = try tieRouter.registerPeer(adapter: alphaAdapter)
    _ = try tieRouter.registerPeer(adapter: betaAdapter)

    try orchestrateUpsertViaRouter(
        router: tieRouter,
        entryId: fixture.tiebreakEntryId,
        canonicalName: fixture.tiebreakEntryCanonicalName,
        fixture: fixture
    )

    let alphaWins = fixture.alphaPeerIdHost < fixture.betaPeerIdHost
    let winner = alphaWins ? alphaAdapter : betaAdapter
    let loser = alphaWins ? betaAdapter : alphaAdapter
    r.check(
        "tie-break routes upsert to lowest peer_id adapter",
        adapterHasEntry(adapter: winner, entryId: fixture.tiebreakEntryId)
    )
    r.check(
        "tie-break skips higher peer_id adapter",
        !adapterHasEntry(adapter: loser, entryId: fixture.tiebreakEntryId)
    )

    alphaAdapter.close()
    betaAdapter.close()
    alphaHost.close()
    betaHost.close()

    let (onlyComputableAdapter, onlyComputableHost) = try dialLoopbackPeer(
        fixture: fixture,
        hostSeed: fixture.computableHostSeed,
        hostPubkey: fixture.computablePubkeyHost,
        hostPeerId: fixture.computablePeerIdHost,
        hostManifestJson: fixture.computableManifestJson
    )
    let rejectRouter = newMultiPeerRouterFfi()
    _ = try rejectRouter.registerPeer(adapter: onlyComputableAdapter)

    let rejectJson = fixture.knowledgeEntryJson(
        entryId: fixture.noMatchEntryId,
        canonicalName: fixture.noMatchEntryCanonicalName
    )
    do {
        _ = try rejectRouter.putKnowledgeEntry(entryJson: rejectJson, expectedBaseRevision: nil)
        r.check("no_capable_peer rejects baseline upsert", false)
    } catch let error as FfiError {
        switch error {
        case .Rejected(let code, _, let kind, let wireCode):
            r.check("no_capable_peer code is CAPABILITY_PORT_MISSING", code == "CAPABILITY_PORT_MISSING")
            r.check("no_capable_peer kind is no_capable_peer", kind == "no_capable_peer")
            r.check("no_capable_peer wire_code is no_capable_peer", wireCode == "no_capable_peer")
        default:
            r.check("no_capable_peer surfaces as FfiError.Rejected (got \(error))", false)
        }
    }

    onlyComputableAdapter.close()
    onlyComputableHost.close()
}
