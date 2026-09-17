import java.io.File
import org.json.JSONArray
import org.json.JSONObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import uniffi.spoke_connect.ConnectResponderFfi
import uniffi.spoke_connect.FfiException
import uniffi.spoke_connect.LoopbackTransport
import uniffi.spoke_connect.PortsHandler
import uniffi.spoke_connect.RemoteAdapterFfi
import uniffi.spoke_connect.Transport
import uniffi.spoke_connect.connectRemoteAdapterFfi
import uniffi.spoke_connect.connectResponderFfi
import uniffi.spoke_connect.loopbackTransportPair

/** Loader-only canary: it exists nowhere else — not in the request, not in the
 * expected response — so its absence from the recorded envelopes is the "no
 * loader value on the wire" witness (F1). */
private const val KE_LOADER_CANARY = "ke-ffi-loader-canary-kotlin"

/**
 * Optional-port dialer ops + responder ports serving over the loopback pair
 * (D16) — runs in the DEFAULT `gradle test` against the committed production
 * binding (no smoke host needed): the responder serves baseline + optional
 * `port.*` families through a foreign [PortsHandler] (user lock), the dialer
 * drives `project` / `compute` / `listForkTimelineEvents`, and the error rows
 * — capability-gate deny, absent-ports fail-closed deny, and foreign-fault
 * containment with serve-loop survival — mirror the Rust
 * `connect_responder_ffi_tests` battery (parity with
 * `crates/spoke-connect/src/ffi.rs`).
 *
 * The KE remote cases mirror the Rust `ke_remote_ffi_tests` battery: the
 * `extract` service face round-trips a provisional batch, its three
 * unavailability rows stay distinct (unnegotiated flag / absent ports probe /
 * callback application refusal), and the existing Scope query carries the
 * `ke-ownership` gate (OQ-FFI-1). The host-local loader value never crosses the
 * wire: a loader-only canary is checked absent against the recorded envelopes,
 * and no loader callback is exported.
 */
class PortsLoopbackFfiPairTest {
    private val fixture = loadLoopbackFixture()

    private fun loadLoopbackFixture(): JSONObject {
        val file = File("Smoke/fixtures/loopback-smoke.json")
        return JSONObject(file.readText())
    }

    private fun decodeHex(hex: String): ByteArray {
        require(hex.length % 2 == 0) { "hex must have even length" }
        return ByteArray(hex.length / 2) { i ->
            hex.substring(i * 2, i * 2 + 2).toInt(16).toByte()
        }
    }

    /** Foreign-callback transport delegating to the client end of a loopback pair. */
    private class LoopbackCallbackTransport(
        private val inner: LoopbackTransport,
    ) : Transport {
        override fun send(envelope: ByteArray) {
            inner.send(envelope)
        }

        override fun recv(): ByteArray {
            return inner.recv()
        }

        override fun close() {
            inner.close()
        }
    }

    @Test
    fun portsLoopback_servesBaselineAndOptionalFamiliesThroughForeignHandler() {
        val seedClient = decodeHex(fixture.getString("seed_client_hex"))
        val seedHost = decodeHex(fixture.getString("seed_host_hex"))
        val pubkeyHost = decodeHex(fixture.getString("pubkey_host_hex"))
        val pubkeyClient = decodeHex(fixture.getString("pubkey_client_hex"))
        val peerIdHost = fixture.getString("peer_id_host")
        val peerIdClient = fixture.getString("peer_id_client")

        val handler = SmokePortsHandler()
        val (responder, dialer) =
            dialPortsPair(seedClient, seedHost, pubkeyHost, pubkeyClient, peerIdHost, peerIdClient, handler)

        try {
            assertEquals("Established", dialer.state())
            waitForState("ports responder handshake", { responder.state() }, "Established")

            // 1. Baseline round-trip through the foreign ports handler: put
            //    stores the entry JSON in the handler, get serves it back.
            //    The wire carries the canonicalized entry JSON (typed
            //    round-trip), so compare semantically, not byte-wise.
            val entryJson = knowledgeEntryJson("kb_ffi_ports_put", "FFI Ports Put")
            val putJson = dialer.putKnowledgeEntry(entryJson, null)
            assertEquals("kb_ffi_ports_put", JSONObject(putJson).getString("entry_id"))
            val getJson = dialer.getKnowledgeEntry("kb_ffi_ports_put")
            assertEquals("FFI Ports Put", JSONObject(getJson).getString("canonical_name"))

            // 2. Application-reject passthrough: an unknown entry rejects
            //    with the handler's locked code + re-hung kind (ordinary
            //    deny, NOT containment).
            val missing = assertRejected("unknown entry reject") {
                dialer.getKnowledgeEntry("kb_ffi_ports_missing")
            }
            assertEquals("KNOWLEDGE_ENTRY_NOT_FOUND", missing.code, "unknown entry reject code")
            assertEquals("store_miss", missing.kind, "unknown entry reject kind (re-hung)")
            assertNull(missing.wireCode, "unknown entry reject wire_code")

            // 3. Optional dialer ops round-trip through the callback
            //    (l2-computable / l5-fork negotiated by both manifests).
            val projectJson = dialer.project(
                """{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}""",
            )
            val project = JSONObject(projectJson)
            assertEquals("sess_ffi_ports", project.getString("session_id"))
            assertEquals("kb_ffi_ports_proj", project.getString("entry_id"))
            val projectComputable = project.getJSONObject("computable")
            assertEquals(2.4, projectComputable.getDouble("tide_level"))
            assertEquals(38, projectComputable.getInt("cargo_tons"))

            val computeJson = dialer.compute(
                """{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}""",
            )
            val compute = JSONObject(computeJson)
            for (field in listOf("computable", "state")) {
                val value = compute.getJSONObject(field)
                assertEquals(2.5, value.getDouble("tide_level"), "compute $field tide_level")
                assertEquals(37, value.getInt("cargo_tons"), "compute $field cargo_tons")
            }

            val eventsJson = dialer.listForkTimelineEvents(
                """{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}""",
            )
            val events = JSONArray(eventsJson)
            assertEquals(1, events.length(), "fork timeline event count")
            assertEquals("evt_tw_ffi_storm", events.getJSONObject(0).getString("timeline_event_id"))
            assertEquals("fork_tw_ffi_events", events.getJSONObject(0).getString("fork_id"))

            // 4. Malformed JSON is rejected locally (INVALID_INPUT, zero wire
            //    traffic) — the dialer pre-validation row per op.
            val badProject = assertRejected("malformed project json") {
                dialer.project("{ not json")
            }
            assertEquals("INVALID_INPUT", badProject.code, "malformed project json code")
            assertNull(badProject.wireCode, "malformed project json wire_code")

            // 5. Foreign-fault containment: the handler faults on
            //    kb_ffi_ports_boom -> INTERNAL_ERROR with no details; the
            //    session survives and the serve loop answers the next
            //    healthy put.
            val contained = assertRejected("foreign-fault containment") {
                dialer.getKnowledgeEntry("kb_ffi_ports_boom")
            }
            assertEquals("INTERNAL_ERROR", contained.code, "foreign-fault containment code")
            assertNull(contained.kind, "foreign-fault containment kind")
            assertNull(contained.wireCode, "foreign-fault containment wire_code (details None)")

            val healthyJson = dialer.putKnowledgeEntry(
                knowledgeEntryJson("kb_ffi_ports_after", "After Containment"),
                null,
            )
            assertEquals(
                "kb_ffi_ports_after",
                JSONObject(healthyJson).getString("entry_id"),
                "serve loop survives foreign-fault containment",
            )
        } finally {
            dialer.close()
            responder.close()
            assertEquals("Closed", dialer.state())
            assertEquals("Closed", responder.state())
        }
    }

    @Test
    fun portsLoopback_absentPortsConstructorDeniesFailClosed() {
        val seedClient = decodeHex(fixture.getString("seed_client_hex"))
        val seedHost = decodeHex(fixture.getString("seed_host_hex"))
        val pubkeyHost = decodeHex(fixture.getString("pubkey_host_hex"))
        val pubkeyClient = decodeHex(fixture.getString("pubkey_client_hex"))
        val peerIdHost = fixture.getString("peer_id_host")
        val peerIdClient = fixture.getString("peer_id_client")

        // Optional families negotiated, but the responder is built WITHOUT a
        // PortsHandler: the capability gate passes, the serving probe finds
        // no ports face, and every optional op denies with the preserved
        // op_unsupported wire code (the documented absent-ports default).
        val (responder, dialer) =
            dialPortsPair(seedClient, seedHost, pubkeyHost, pubkeyClient, peerIdHost, peerIdClient, null)

        try {
            assertEquals("Established", dialer.state())
            waitForState("absent-ports responder handshake", { responder.state() }, "Established")

            assertOptionalOpsDenied(dialer, "absent-ports deny")
        } finally {
            dialer.close()
            responder.close()
            assertEquals("Closed", dialer.state())
            assertEquals("Closed", responder.state())
        }
    }

    @Test
    fun portsLoopback_capabilityGateDeniesOptionalOps() {
        val seedClient = decodeHex(fixture.getString("seed_client_hex"))
        val seedHost = decodeHex(fixture.getString("seed_host_hex"))
        val pubkeyHost = decodeHex(fixture.getString("pubkey_host_hex"))
        val pubkeyClient = decodeHex(fixture.getString("pubkey_client_hex"))
        val peerIdHost = fixture.getString("peer_id_host")
        val peerIdClient = fixture.getString("peer_id_client")

        // Default manifests advertise spoke-baseline only, so the negotiated
        // set lacks l2-computable / l5-fork and every optional op is denied
        // at the responder's dispatch gate with the preserved op_unsupported
        // wire code.
        val pair = loopbackTransportPair()
        val responder: ConnectResponderFfi =
            connectResponderFfi(
                transport = LoopbackCallbackTransport(pair.server()),
                seed = seedHost,
                manifestJson = toolManifestJson("test-responder"),
                allowlist = listOf(peerIdClient),
                peerKeys = mapOf(peerIdClient to pubkeyClient),
                ports = null,
                invokeTimeoutMs = null,
            )
        val dialer: RemoteAdapterFfi =
            connectRemoteAdapterFfi(
                transport = LoopbackCallbackTransport(pair.client()),
                localSeed = seedClient,
                localManifestJson = toolManifestJson("test-client"),
                remotePubkey = pubkeyHost,
                allowlist = listOf(peerIdHost),
                invokeTimeoutMs = null,
            )

        try {
            assertEquals("Established", dialer.state())
            waitForState("capability-deny responder handshake", { responder.state() }, "Established")

            assertOptionalOpsDenied(dialer, "capability deny")
        } finally {
            dialer.close()
            responder.close()
            assertEquals("Closed", dialer.state())
            assertEquals("Closed", responder.state())
        }
    }

    // ── KE remote (F1/F2/F3) ──────────────────────────────────────────────
    //
    // The `extract` service face and the `ke-ownership` gate on the existing
    // Scope query, mirroring the Rust `ke_remote_ffi_tests` battery
    // (`crates/spoke-connect/src/ffi.rs`). The generated `PortsHandler`
    // interface carries no loader method — source loading is the serving
    // host's own business — so the loader value stays a private handler field
    // and the canary below exists nowhere else: not in the request, not in the
    // expected response. It is asserted absent from the recorded envelopes.

    /** Both peers' hello manifests: the baseline capability plus whatever the
     * scenario negotiates. A capability must appear in *both* hellos to be
     * negotiated, so the omitted side is how the deny scenarios are set up.
     * Mirror of the Rust `ke_manifest_json` test helper. */
    private fun keManifestJson(hostId: String, capabilities: List<String>): String {
        val capabilitiesJson = JSONArray().put("spoke-baseline")
        capabilities.forEach { capabilitiesJson.put(it) }
        return JSONObject()
            .put("schema_version", 1)
            .put("host_id", hostId)
            .put("roles", JSONArray().put("data-store"))
            .put("capabilities", capabilitiesJson)
            .put("namespaces", JSONArray().put("toy_world"))
            .put("extensions", JSONObject())
            .toString()
    }

    /** An `extract` request for the scenario's run id: the payload is the
     * `ExtractRequest` itself and `sources` carry references only. */
    private fun keExtractRequestJson(runId: String): String = JSONObject()
        .put("run_id", runId)
        .put(
            "sources",
            JSONArray()
                .put(JSONObject().put("schema_version", 1).put("source_id", "manuscript/ch1").put("extensions", JSONObject()))
                .put(JSONObject().put("schema_version", 1).put("source_id", "manuscript/ch2").put("extensions", JSONObject())),
        )
        .toString()

    /** A Scope carrying a reader viewpoint plus opaque extension values the
     * callback must receive unchanged. */
    private fun keViewpointScopeJson(): String = JSONObject()
        .put("scope_id", "toy-scope-001")
        .put("viewpoint", "kb_tw_mira")
        .put("entry_types", JSONArray().put("note"))
        .put(
            "extensions",
            JSONObject().put("product", JSONObject().put("viewpoint", "decoy").put("owner", "someone")),
        )
        .toString()

    /** Loopback pair through both FFI faces with the scenario's negotiated
     * capabilities and optional foreign [PortsHandler]. Mirror of the Rust
     * `dial_ke_remote` test helper. */
    private fun dialKePair(
        clientCapabilities: List<String>,
        responderCapabilities: List<String>,
        ports: PortsHandler?,
    ): Triple<ConnectResponderFfi, RemoteAdapterFfi, RecordingLoopbackTransport> {
        val pair = loopbackTransportPair()
        val responder: ConnectResponderFfi =
            connectResponderFfi(
                transport = LoopbackCallbackTransport(pair.server()),
                seed = decodeHex(fixture.getString("seed_host_hex")),
                manifestJson = keManifestJson("test-responder", responderCapabilities),
                allowlist = listOf(fixture.getString("peer_id_client")),
                peerKeys = mapOf(fixture.getString("peer_id_client") to decodeHex(fixture.getString("pubkey_client_hex"))),
                ports = ports,
                invokeTimeoutMs = null,
            )
        val transport = RecordingLoopbackTransport(pair.client())
        val dialer: RemoteAdapterFfi =
            connectRemoteAdapterFfi(
                transport = transport,
                localSeed = decodeHex(fixture.getString("seed_client_hex")),
                localManifestJson = keManifestJson("test-client", clientCapabilities),
                remotePubkey = decodeHex(fixture.getString("pubkey_host_hex")),
                allowlist = listOf(fixture.getString("peer_id_host")),
                invokeTimeoutMs = null,
            )
        assertEquals("Established", dialer.state(), "ke dialer state")
        waitForState("ke responder handshake", { responder.state() }, "Established")
        return Triple(responder, dialer, transport)
    }

    /** The unavailable-capability row shared by the unnegotiated and
     * absent-provider denials (the callback-refusal row differs in `wireCode`
     * and is asserted at its own case). */
    private fun assertUnavailableCapability(what: String, wireCode: String, block: () -> Unit) {
        val denied = assertRejected(what) { block() }
        assertEquals("CAPABILITY_PORT_MISSING", denied.code, "$what: deny code")
        assertNull(denied.kind, "$what: deny kind")
        assertEquals(wireCode, denied.wireCode, "$what: deny wire_code")
    }

    @Test
    fun keRemote_extractRoundTripsTheProvisionalBatch() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, _) =
            dialKePair(listOf("ke-extraction"), listOf("ke-extraction"), handler)

        try {
            val response = JSONObject(dialer.extract(keExtractRequestJson("run-ffi-ke-1")))

            // Observable, deterministic response behaviour: one provisional
            // candidate per declared source reference, carrying the run id.
            assertEquals("run-ffi-ke-1", response.getJSONObject("run").getString("run_id"), "ke extract run_id")
            val candidates = response.getJSONArray("candidates")
            assertEquals(2, candidates.length(), "ke extract candidates (one per source)")
            val wantEntryIds = listOf("ke-ffi-run-ffi-ke-1-0", "ke-ffi-run-ffi-ke-1-1")
            for (index in wantEntryIds.indices) {
                val candidate = candidates.getJSONObject(index)
                assertEquals(wantEntryIds[index], candidate.getString("entry_id"), "candidate $index entry_id")
                assertEquals("provisional", candidate.getString("status"), "candidate $index status")
                assertEquals("note", candidate.getString("entry_type"), "candidate $index entry_type")
            }

            // The callback received the `ExtractRequest` itself: no wrapper and
            // no loader value — only references travelled.
            val requests = handler.extractRequests
            assertEquals(1, requests.size, "ke extract callback reached exactly once")
            val request = requests.first()
            assertEquals("run-ffi-ke-1", request.getString("run_id"), "ke extract callback run_id")
            val sources = request.getJSONArray("sources")
            assertEquals(2, sources.length(), "ke extract callback sources")
            assertEquals("manuscript/ch1", sources.getJSONObject(0).getString("source_id"), "ke extract source reference")
            for (wrapper in listOf("request", "arguments", "input", "loaded_input")) {
                assertFalse(request.has(wrapper), "ke extract callback request carries no $wrapper wrapper")
            }
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_extractRejectsMalformedRequestJsonWithZeroWireTraffic() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, transport) =
            dialKePair(listOf("ke-extraction"), listOf("ke-extraction"), handler)

        try {
            val framesBefore = transport.frames.size
            val malformed = assertRejected("ke malformed extract json") { dialer.extract("{ not json") }
            assertEquals("INVALID_INPUT", malformed.code, "ke malformed extract json code")
            assertNull(malformed.kind, "ke malformed extract json kind")
            assertNull(malformed.wireCode, "ke malformed extract json wire_code")
            assertTrue(handler.extractRequests.isEmpty(), "ke malformed extract never reaches the callback")
            assertEquals(framesBefore, transport.frames.size, "ke malformed extract never reaches the wire")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_extractDeniesWhenTheRespondingHelloOmitsKeExtraction() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, _) = dialKePair(listOf("ke-extraction"), emptyList(), handler)

        try {
            assertUnavailableCapability("ke extract unnegotiated (responder omits the flag)", "op_unsupported") {
                dialer.extract(keExtractRequestJson("run-ffi-unnegotiated"))
            }
            assertTrue(handler.extractRequests.isEmpty(), "the gate denies before the callback")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_extractDeniesWhenTheDialingHelloOmitsKeExtraction() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, _) = dialKePair(emptyList(), listOf("ke-extraction"), handler)

        try {
            assertUnavailableCapability("ke extract unnegotiated (dialer omits the flag)", "op_unsupported") {
                dialer.extract(keExtractRequestJson("run-ffi-unnegotiated"))
            }
            assertTrue(handler.extractRequests.isEmpty(), "the gate denies before the callback")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_extractProbeDeniesWhenPortsAreAbsent() {
        val (responder, dialer, _) =
            dialKePair(listOf("ke-extraction"), listOf("ke-extraction"), null)

        try {
            assertUnavailableCapability("ke extract absent-ports probe deny", "op_unsupported") {
                dialer.extract(keExtractRequestJson("run-ffi-absent-ports"))
            }
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_extractPassesACallbackApplicationRefusalThrough() {
        val handler = KeRemotePortsHandler(declineExtract = true)
        val (responder, dialer, _) =
            dialKePair(listOf("ke-extraction"), listOf("ke-extraction"), handler)

        try {
            val refused = assertRejected("ke extract callback refusal") {
                dialer.extract(keExtractRequestJson("run-ffi-refusal"))
            }
            assertEquals("CAPABILITY_PORT_MISSING", refused.code, "ke extract refusal code")
            assertNull(refused.kind, "ke extract refusal kind")
            assertNull(refused.wireCode, "ke extract refusal wire_code (not a probe deny)")
            assertTrue(
                refused.detail.contains("does not serve extraction"),
                "ke extract refusal message survives: ${refused.detail}",
            )
            assertEquals(1, handler.extractRequests.size, "a refusal is the callback's own answer")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_scopeViewpointServesWithOwnershipNegotiated() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, _) =
            dialKePair(listOf("ke-ownership"), listOf("ke-ownership"), handler)

        try {
            val served = JSONArray(dialer.listKnowledgeEntries(keViewpointScopeJson()))
            assertEquals(2, served.length(), "ke ownership allow serves exactly two entries")
            assertEquals("kb_ffi_ke_own", served.getJSONObject(0).getString("entry_id"), "own-private entry")
            assertEquals("FFI KE Own", served.getJSONObject(0).getString("canonical_name"), "canonical_name")
            assertEquals("kb_ffi_ke_shared", served.getJSONObject(1).getString("entry_id"), "shared entry")

            // The callback received the declared Scope unchanged: the viewpoint
            // and the opaque extension values are not stripped to make the
            // request succeed, and no extraction call was involved.
            val scopes = handler.scopeCalls
            assertEquals(1, scopes.size, "ke ownership allow reaches the callback exactly once")
            val scope = scopes.first()
            assertEquals("kb_tw_mira", scope.getString("viewpoint"), "ke ownership allow viewpoint")
            assertEquals("note", scope.getJSONArray("entry_types").getString(0), "ke ownership allow entry_types")
            val product = scope.getJSONObject("extensions").getJSONObject("product")
            assertEquals("decoy", product.getString("viewpoint"), "ke ownership allow decoy viewpoint")
            assertEquals("someone", product.getString("owner"), "ke ownership allow opaque owner")
            assertTrue(handler.extractRequests.isEmpty(), "ke ownership allow never invokes extract")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_scopeViewpointDeniesWhenTheDialingHelloOmitsKeOwnership() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, _) = dialKePair(emptyList(), listOf("ke-ownership"), handler)

        try {
            assertUnavailableCapability("ke ownership deny (dialer omits the flag)", "op_unsupported") {
                dialer.listKnowledgeEntries(keViewpointScopeJson())
            }
            assertTrue(handler.scopeCalls.isEmpty(), "the gate refuses before the callback is reached")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_scopeViewpointDeniesWhenTheRespondingHelloOmitsKeOwnership() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, _) = dialKePair(listOf("ke-ownership"), emptyList(), handler)

        try {
            assertUnavailableCapability("ke ownership deny (responder omits the flag)", "op_unsupported") {
                dialer.listKnowledgeEntries(keViewpointScopeJson())
            }
            assertTrue(handler.scopeCalls.isEmpty(), "the gate refuses before the callback is reached")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    @Test
    fun keRemote_loaderCanaryNeverReachesTheWire() {
        val handler = KeRemotePortsHandler()
        val (responder, dialer, transport) =
            dialKePair(listOf("ke-extraction"), listOf("ke-extraction"), handler)

        try {
            val response = dialer.extract(keExtractRequestJson("run-ffi-canary"))
            assertTrue(response.contains("run-ffi-canary"), "ke canary round-trip really happened")
            assertFalse(response.contains(KE_LOADER_CANARY), "ke canary is not serialized into the response")
            val frames = transport.frames
            assertTrue(frames.isNotEmpty(), "ke canary row records the envelopes")
            val recorded = frames.joinToString("") { String(it, Charsets.UTF_8) }
            // Positive control: the recording really observes the plaintext
            // wire (the invoke request's run id is visible), so the canary
            // absence below is a real observation, not a vacuous one.
            assertTrue(recorded.contains("run-ffi-canary"), "ke canary positive control: the request is on the wire")
            assertFalse(recorded.contains(KE_LOADER_CANARY), "ke loader canary never reaches the wire")
        } finally {
            dialer.close()
            responder.close()
        }
    }

    /** Dialer-side transport that records every envelope in both directions,
     * so the smoke can assert what did and did not reach the wire. */
    private class RecordingLoopbackTransport(
        private val inner: LoopbackTransport,
    ) : Transport {
        private val lock = Any()
        private val recorded = mutableListOf<ByteArray>()

        val frames: List<ByteArray>
            get() = synchronized(lock) { recorded.toList() }

        override fun send(envelope: ByteArray) {
            synchronized(lock) { recorded.add(envelope) }
            inner.send(envelope)
        }

        override fun recv(): ByteArray {
            val envelope = inner.recv()
            synchronized(lock) { recorded.add(envelope) }
            return envelope
        }

        override fun close() {
            inner.close()
        }
    }

    /** Foreign-callback ports handler for the KE remote matrix: a private
     * host-local loader value, the `extract` service face, and a
     * viewpoint-filtered knowledge store. Mirror of the Rust
     * `KeRemotePortsHandler` double. */
    private class KeRemotePortsHandler(private val declineExtract: Boolean = false) : PortsHandler {
        /** Catalogue recipe; `visibility` is the host's own filtering input,
         * not a wire contract. Ordered by entry id for a deterministic batch. */
        private val catalog = listOf(
            Triple("kb_ffi_ke_foreign", "FFI KE Foreign", "private:kb_tw_other"),
            Triple("kb_ffi_ke_own", "FFI KE Own", "private:kb_tw_mira"),
            Triple("kb_ffi_ke_shared", "FFI KE Shared", "shared"),
        )

        private val lock = Any()
        private val requested = mutableListOf<JSONObject>()
        private val scopes = mutableListOf<JSONObject>()
        /** The serving host's loaded in-process input (F1): only this host
         * sees it, and the canary is unique to the loader. */
        private val loadedCanary = KE_LOADER_CANARY

        val extractRequests: List<JSONObject>
            get() = synchronized(lock) { requested.toList() }

        val scopeCalls: List<JSONObject>
            get() = synchronized(lock) { scopes.toList() }

        override fun extract(extractRequestJson: String): String {
            val request = JSONObject(extractRequestJson)
            synchronized(lock) { requested.add(request) }
            if (declineExtract) {
                throw FfiException.Rejected(
                    code = "CAPABILITY_PORT_MISSING",
                    detail = "this binding does not serve extraction",
                    kind = null,
                    wireCode = null,
                )
            }
            // The loader runs inside the host service: its value feeds the
            // extractor here and is never a transport argument or callback value.
            check(loadedCanary.isNotEmpty()) { "host loader value is missing" }
            val runId = request.getString("run_id")
            val sources = request.getJSONArray("sources")
            val candidates = JSONArray()
            for (index in 0 until sources.length()) {
                candidates.put(
                    JSONObject()
                        .put("schema_version", 1)
                        .put("entry_id", "ke-ffi-$runId-$index")
                        .put("entry_type", "note")
                        .put("canonical_name", "Extracted note $index")
                        .put("status", "provisional")
                        .put("body", JSONObject().put("summary", "provisional candidate $index"))
                        .put("extensions", JSONObject()),
                )
            }
            return JSONObject()
                .put("candidates", candidates)
                .put("run", JSONObject().put("run_id", runId))
                .toString()
        }

        override fun listKnowledgeEntries(scopeJson: String): String {
            val scope = JSONObject(scopeJson)
            synchronized(lock) { scopes.add(scope) }
            val viewpoint = scope.optString("viewpoint", "")
            val served = JSONArray()
            for ((entryId, canonicalName, visibility) in catalog) {
                if (visibility != "shared" && visibility != "private:$viewpoint") continue
                served.put(
                    JSONObject()
                        .put("schema_version", 1)
                        .put("entry_id", entryId)
                        .put("entry_type", "knowledge")
                        .put("canonical_name", canonicalName)
                        .put("status", "active")
                        .put("body", JSONObject().put("summary", "served through the foreign ports callback"))
                        .put("extensions", JSONObject().put("visibility", JSONObject().put("scope", visibility))),
                )
            }
            return served.toString()
        }

        /** Catalogue ops this double does not serve: an ordinary application
         * reject (`wireCode` null), never containment. */
        private fun unserved(op: String): FfiException = FfiException.Rejected(
            code = "INVALID_INPUT",
            detail = "$op is not served by this test handler",
            kind = null,
            wireCode = null,
        )

        override fun getKnowledgeEntry(entryId: String): String = throw unserved("get_knowledge_entry")

        override fun putKnowledgeEntry(entryJson: String, expectedBaseRevision: ULong?): String =
            throw unserved("put_knowledge_entry")

        override fun getRelation(relationId: String): String = throw unserved("get_relation")

        override fun putRelation(relationJson: String, expectedBaseRevision: ULong?): String =
            throw unserved("put_relation")

        override fun listTimelineEvents(scopeJson: String): String = throw unserved("list_timeline_events")

        override fun putFindings(findingsJson: String): String = throw unserved("put_findings")

        override fun listRules(ruleRefs: List<String>): String = throw unserved("list_rules")

        override fun listPeerHostCapabilityManifests(): String = throw unserved("list_peer_host_capability_manifests")

        override fun project(projectRequestJson: String): String = throw unserved("project")

        override fun compute(computeRequestJson: String): String = throw unserved("compute")

        override fun listForkTimelineEvents(scopeJson: String): String = throw unserved("list_fork_timeline_events")
    }

    private fun assertOptionalOpsDenied(dialer: RemoteAdapterFfi, what: String) {
        val cases = listOf(
            "project" to { dialer.project(PROJECT_REQUEST_JSON) },
            "compute" to { dialer.compute(COMPUTE_REQUEST_JSON) },
            "listForkTimelineEvents" to { dialer.listForkTimelineEvents(FORK_SCOPE_JSON) },
        )
        for ((name, invoke) in cases) {
            val denied = assertRejected("$what: $name deny") { invoke() }
            assertEquals("CAPABILITY_PORT_MISSING", denied.code, "$what: $name deny code")
            assertEquals("op_unsupported", denied.wireCode, "$what: $name deny wire_code")
        }
    }

    /** Loopback pair through both FFI faces with an optional foreign
     * [PortsHandler]; both manifests declare the optional families. Mirror
     * of the Rust `dial_responder_ffi_with_ports` test helper. */
    private fun dialPortsPair(
        seedClient: ByteArray,
        seedHost: ByteArray,
        pubkeyHost: ByteArray,
        pubkeyClient: ByteArray,
        peerIdHost: String,
        peerIdClient: String,
        ports: PortsHandler?,
    ): Pair<ConnectResponderFfi, RemoteAdapterFfi> {
        val pair = loopbackTransportPair()
        val responder: ConnectResponderFfi =
            connectResponderFfi(
                transport = LoopbackCallbackTransport(pair.server()),
                seed = seedHost,
                manifestJson = portsManifestJson("test-responder"),
                allowlist = listOf(peerIdClient),
                peerKeys = mapOf(peerIdClient to pubkeyClient),
                ports = ports,
                invokeTimeoutMs = null,
            )
        val dialer: RemoteAdapterFfi =
            connectRemoteAdapterFfi(
                transport = LoopbackCallbackTransport(pair.client()),
                localSeed = seedClient,
                localManifestJson = portsManifestJson("test-client"),
                remotePubkey = pubkeyHost,
                allowlist = listOf(peerIdHost),
                invokeTimeoutMs = null,
            )
        return responder to dialer
    }

    /** Ports-carrying manifest — baseline + optional families, so the
     * negotiated set includes l2-computable / l5-fork. Mirror of the Rust
     * `ports_manifest_json` test helper. */
    private fun portsManifestJson(hostId: String): String = """
        {"schema_version":1,"host_id":"$hostId","roles":["data-store","l2-computable"],
        "capabilities":["spoke-baseline","l2-computable","l5-fork"],
        "namespaces":["toy_world"],"extensions":{}}
    """.trimIndent()

    /** Tool-carrying manifest (baseline + tools only — no optional families)
     * for the capability-deny session. */
    private fun toolManifestJson(hostId: String): String = """
        {"schema_version":1,"host_id":"$hostId","roles":["data-store"],
        "capabilities":["spoke-baseline","tools.math.add"],
        "namespaces":["math","toy_world"],"extensions":{},
        "tools":[{"schema_version":1,"capability_id":"tools.math.add","op":"tools.math.add",
        "description":"Add two integers","input":{"type":"object"},"output":{"type":"object"}}]}
    """.trimIndent()

    private fun knowledgeEntryJson(entryId: String, canonicalName: String): String = """
        {"schema_version":1,"entry_id":"$entryId","entry_type":"knowledge",
        "canonical_name":"$canonicalName","status":"active",
        "body":{"summary":"served through the foreign ports callback"},"extensions":{}}
    """.trimIndent()

    /** Bounded poll for the handshake to settle (D16 constructor semantics). */
    private fun waitForState(what: String, state: () -> String, expected: String) {
        val deadline = System.nanoTime() + 5_000_000_000L
        var last = state()
        while (last != expected) {
            if (System.nanoTime() > deadline) {
                throw AssertionError("$what: timed out waiting for $expected (last: $last)")
            }
            Thread.sleep(10)
            last = state()
        }
    }

    private fun assertRejected(what: String, block: () -> Unit): FfiException.Rejected {
        return try {
            block()
            throw AssertionError("$what: expected FfiException.Rejected, got success")
        } catch (rejected: FfiException.Rejected) {
            rejected
        }
    }

    /** Foreign-callback ports handler: in-memory knowledge store plus canned
     * optional-family answers; unknown entries reject with an application
     * `Rejected` (ordinary deny — not containment); `kb_ffi_ports_boom`
     * faults (the containment row). Mirror of the Rust `TestPortsHandler`. */
    private class SmokePortsHandler : PortsHandler {
        private val entries = mutableMapOf<String, JSONObject>()

        override fun getKnowledgeEntry(entryId: String): String {
            if (entryId == "kb_ffi_ports_boom") {
                throw RuntimeException("foreign ports handler fault (containment row)")
            }
            return entries[entryId]?.toString()
                ?: throw FfiException.Rejected(
                    code = "KNOWLEDGE_ENTRY_NOT_FOUND",
                    detail = "entry $entryId not found",
                    kind = "store_miss",
                    wireCode = null,
                )
        }

        override fun putKnowledgeEntry(entryJson: String, expectedBaseRevision: ULong?): String {
            val entry = JSONObject(entryJson)
            entries[entry.getString("entry_id")] = entry
            return entryJson
        }

        override fun getRelation(relationId: String): String = throw FfiException.Rejected(
            code = "INVALID_INPUT",
            detail = "relation serving not exercised by this test handler",
            kind = null,
            wireCode = null,
        )

        override fun putRelation(relationJson: String, expectedBaseRevision: ULong?): String = throw FfiException.Rejected(
            code = "INVALID_INPUT",
            detail = "relation serving not exercised by this test handler",
            kind = null,
            wireCode = null,
        )

        override fun listKnowledgeEntries(scopeJson: String): String = JSONArray(entries.values.toList()).toString()

        override fun listTimelineEvents(scopeJson: String): String = "[]"

        override fun putFindings(findingsJson: String): String = "[]"

        override fun listRules(ruleRefs: List<String>): String = "[]"

        override fun listPeerHostCapabilityManifests(): String = "[]"

        override fun project(projectRequestJson: String): String {
            val request = JSONObject(projectRequestJson)
            return JSONObject()
                .put("session_id", request.getString("session_id"))
                .put("entry_id", request.getString("entry_id"))
                .put("computable", JSONObject().put("tide_level", 2.4).put("cargo_tons", 38))
                .toString()
        }

        override fun compute(computeRequestJson: String): String {
            val request = JSONObject(computeRequestJson)
            val computable = request.getJSONObject("computable")
            return JSONObject()
                .put("session_id", request.getString("session_id"))
                .put("entry_id", request.getString("entry_id"))
                .put("computable", computable)
                .put("state", computable)
                .toString()
        }

        override fun listForkTimelineEvents(scopeJson: String): String {
            val scope = JSONObject(scopeJson)
            if (scope.getString("fork_id") != "fork_tw_ffi_events") {
                return "[]"
            }
            return JSONArray()
                .put(
                    JSONObject()
                        .put("schema_version", 1)
                        .put("timeline_event_id", "evt_tw_ffi_storm")
                        .put("canonical_name", "FFI Fork Storm")
                        .put("fork_id", "fork_tw_ffi_events")
                        .put("extensions", JSONObject()),
                )
                .toString()
        }

        /** This double serves only the optional `port.*` families, so `extract`
         * is an ordinary application refusal (never the absent-provider probe
         * deny). The regenerated [PortsHandler] interface requires the method. */
        override fun extract(extractRequestJson: String): String = throw FfiException.Rejected(
            code = "CAPABILITY_PORT_MISSING",
            detail = "this ports smoke double does not serve extraction",
            kind = null,
            wireCode = null,
        )
    }

    private companion object {
        const val PROJECT_REQUEST_JSON =
            """{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}"""
        const val COMPUTE_REQUEST_JSON =
            """{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}"""
        const val FORK_SCOPE_JSON =
            """{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}"""
    }
}
