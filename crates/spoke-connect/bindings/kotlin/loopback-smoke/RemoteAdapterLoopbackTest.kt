import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import org.json.JSONObject
import uniffi.spoke_connect.LoopbackTransport
import uniffi.spoke_connect.RemoteAdapterFfi
import uniffi.spoke_connect.Transport
import uniffi.spoke_connect.connectRemoteAdapterFfi
import uniffi.spoke_connect.loopbackTransportPair
import uniffi.spoke_connect.startLoopbackSmokeHost

/**
 * RemoteAdapterFFI loopback smoke — callback [Transport] over an in-memory pair
 * with the reference ToyWorld smoke host (parity with Swift `loopback_smoke.swift`
 * and `crates/spoke-connect/src/ffi.rs` remote_adapter_ffi_tests).
 *
 * Requires a smoke-host cdylib (`ffi-smoke-host`) and Kotlin bindings regenerated
 * from that cdylib — see [Smoke/README.md](../README.md).
 */
class RemoteAdapterLoopbackTest {
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

    private fun knowledgeEntryJson(): String {
        val entryId = fixture.getString("entry_id")
        val canonicalName = fixture.getString("entry_canonical_name")
        return """
            {"schema_version":1,"entry_id":"$entryId","entry_type":"character","canonical_name":"$canonicalName","status":"provisional","body":{"summary":"Upserted over the loopback: $entryId"},"extensions":{}}
        """.trimIndent()
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
    fun remoteAdapter_putGet_roundTrip() {
        val seedClient = decodeHex(fixture.getString("seed_client_hex"))
        val pubkeyHost = decodeHex(fixture.getString("pubkey_host_hex"))
        val peerIdHost = fixture.getString("peer_id_host")
        val clientManifestJson = fixture.getString("client_manifest_json")
        val sessionId = fixture.getString("session_id")
        val entryId = fixture.getString("entry_id")
        val entryCanonicalName = fixture.getString("entry_canonical_name")

        val pair = loopbackTransportPair()
        val host = startLoopbackSmokeHost(pair.server())

        val transport = LoopbackCallbackTransport(pair.client())
        val adapter: RemoteAdapterFfi =
            connectRemoteAdapterFfi(
                transport = transport,
                localSeed = seedClient,
                localManifestJson = clientManifestJson,
                remotePubkey = pubkeyHost,
                allowlist = listOf(peerIdHost),
                invokeTimeoutMs = null,
            )

        try {
            assertEquals("Established", adapter.state())
            assertEquals(sessionId, adapter.sessionId())
            assertEquals(sessionId, host.sessionId())
            assertEquals(peerIdHost, adapter.remotePeerId())

            val remoteManifest = adapter.remoteManifest()
            assertNotNull(remoteManifest)
            val manifestJson = JSONObject(remoteManifest)
            assertEquals("test-host", manifestJson.getString("host_id"))

            val putJson = adapter.putKnowledgeEntry(knowledgeEntryJson(), expectedBaseRevision = null)
            assertTrue(putJson.isNotEmpty())
            val putObject = JSONObject(putJson)
            assertEquals(entryId, putObject.getString("entry_id"))

            val getJson = adapter.getKnowledgeEntry(entryId)
            assertTrue(getJson.isNotEmpty())
            val getObject = JSONObject(getJson)
            assertEquals(entryId, getObject.getString("entry_id"))
            assertEquals(entryCanonicalName, getObject.getString("canonical_name"))
        } finally {
            adapter.close()
            assertEquals("Closed", adapter.state())
            host.close()
        }
    }
}
