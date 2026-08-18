import java.io.File
import org.json.JSONArray
import org.json.JSONObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import uniffi.spoke_connect.ConnectResponderFfi
import uniffi.spoke_connect.FfiException
import uniffi.spoke_connect.LoopbackTransport
import uniffi.spoke_connect.PortsHandler
import uniffi.spoke_connect.RemoteAdapterFfi
import uniffi.spoke_connect.Transport
import uniffi.spoke_connect.connectRemoteAdapterFfi
import uniffi.spoke_connect.connectResponderFfi
import uniffi.spoke_connect.loopbackTransportPair

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
