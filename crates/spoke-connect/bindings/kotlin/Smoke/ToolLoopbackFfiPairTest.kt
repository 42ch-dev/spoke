import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.Test
import kotlin.test.assertEquals
import org.json.JSONObject
import uniffi.spoke_connect.ConnectResponderFfi
import uniffi.spoke_connect.FfiException
import uniffi.spoke_connect.LoopbackTransport
import uniffi.spoke_connect.RemoteAdapterFfi
import uniffi.spoke_connect.ToolHandler
import uniffi.spoke_connect.Transport
import uniffi.spoke_connect.connectRemoteAdapterFfi
import uniffi.spoke_connect.connectResponderFfi
import uniffi.spoke_connect.loopbackTransportPair

/**
 * Tool faces over the loopback pair (D15/D16) — runs in the DEFAULT `gradle
 * test` against the committed production binding (no smoke host needed): both
 * ends are FFI objects — the responder serves a foreign [ToolHandler], the
 * dialer serves reverse invokes through `RemoteAdapterFFI.registerToolHandler`,
 * unregistered tools deny with `op_unsupported`, and a handler-thrown
 * application reject passes through verbatim (parity with
 * `crates/spoke-connect/src/ffi.rs` connect_responder_ffi_tests).
 */
class ToolLoopbackFfiPairTest {
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
    fun toolLoopback_ffiPair_roundTrips() {
        val seedClient = decodeHex(fixture.getString("seed_client_hex"))
        val seedHost = decodeHex(fixture.getString("seed_host_hex"))
        val pubkeyHost = decodeHex(fixture.getString("pubkey_host_hex"))
        val pubkeyClient = decodeHex(fixture.getString("pubkey_client_hex"))
        val peerIdHost = fixture.getString("peer_id_host")
        val peerIdClient = fixture.getString("peer_id_client")

        val pair = loopbackTransportPair()
        // The accept-side constructor returns immediately in `Handshaking`
        // (D16): the dialer hello is the sync point, so the smoke polls
        // `state()` (bounded) to `Established` before invoking; a handshake
        // failure surfaces as `Closed`, never a thrown constructor error.
        val responder: ConnectResponderFfi =
            connectResponderFfi(
                transport = LoopbackCallbackTransport(pair.server()),
                seed = seedHost,
                manifestJson = toolManifestJson("test-responder"),
                allowlist = listOf(peerIdClient),
                peerKeys = mapOf(peerIdClient to pubkeyClient),
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
            waitForState("tool responder handshake", { responder.state() }, "Established")

            // 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
            val responderSum = SumToolHandler()
            responder.registerToolHandler("tools.math.add", responderSum)
            val sumJson = dialer.invokeTool("tools.math.add", """{"a": 1, "b": 2}""")
            assertEquals(3L, JSONObject(sumJson).getLong("sum"))
            assertEquals(1, responderSum.calls(), "responder handler invocation count")

            // 2. Responder FFI invoke_tool -> dialer-side handler registered
            //    via RemoteAdapterFFI.registerToolHandler.
            val dialerSum = SumToolHandler()
            dialer.registerToolHandler("tools.math.add", dialerSum)
            val reverseSumJson = responder.invokeTool("tools.math.add", """{"a": 21, "b": 21}""")
            assertEquals(42L, JSONObject(reverseSumJson).getLong("sum"))
            assertEquals(1, dialerSum.calls(), "dialer handler invocation count")

            // 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
            val denied = assertRejected("unregistered tool deny") {
                dialer.invokeTool("tools.echo.boom", "{}")
            }
            assertEquals("CAPABILITY_PORT_MISSING", denied.code, "unregistered tool deny code")
            assertEquals("op_unsupported", denied.wireCode, "unregistered tool deny wire_code")

            // 4. Handler-thrown application reject passes through verbatim
            //    (kind / wire_code re-hung onto details by the bridge).
            dialer.registerToolHandler(
                "tools.echo.boom",
                ThrowingToolHandler(
                    FfiException.Rejected(
                        code = "REVISION_CONFLICT",
                        detail = "foreign handler rejected",
                        kind = null,
                        wireCode = "op_unsupported",
                    ),
                ),
            )
            val passed = assertRejected("reject passthrough") {
                responder.invokeTool("tools.echo.boom", "{}")
            }
            assertEquals("REVISION_CONFLICT", passed.code, "reject passthrough code")
            assertEquals("foreign handler rejected", passed.detail, "reject passthrough message")
            assertEquals("op_unsupported", passed.wireCode, "reject passthrough wire_code")
        } finally {
            dialer.close()
            responder.close()
            assertEquals("Closed", dialer.state())
            assertEquals("Closed", responder.state())
        }
    }

    /** Tool-carrying manifest — every tool capability also sits in
     * `capabilities[]` so the negotiated set includes the `tools.*` ops
     * (D13 dispatch gate). Mirror of the Rust `tool_manifest` test helper. */
    private fun toolManifestJson(hostId: String): String = """
        {"schema_version":1,"host_id":"$hostId","roles":["data-store"],
        "capabilities":["spoke-baseline","tools.math.add","tools.echo.echo","tools.echo.boom"],
        "namespaces":["math","echo","toy_world"],"extensions":{},
        "tools":[
        {"schema_version":1,"capability_id":"tools.math.add","op":"tools.math.add","description":"Add two integers","input":{"type":"object"},"output":{"type":"object"}},
        {"schema_version":1,"capability_id":"tools.echo.echo","op":"tools.echo.echo","description":"Echo the arguments","input":{"type":"object"},"output":{"type":"object"}},
        {"schema_version":1,"capability_id":"tools.echo.boom","op":"tools.echo.boom","description":"Explodes","input":{"type":"object"},"output":{"type":"object"}}
        ]}
    """.trimIndent()

    /** Bounded poll for the handshake to settle (D16 constructor semantics). */
    private fun waitForState(what: String, state: () -> String, expected: String) {
        val deadline = System.currentTimeMillis() + 5_000
        var last = state()
        while (last != expected) {
            check(System.currentTimeMillis() < deadline) {
                "$what: timed out waiting for $expected (last: $last)"
            }
            Thread.sleep(10)
            last = state()
        }
    }

    private fun assertRejected(what: String, block: () -> Unit): FfiException.Rejected {
        try {
            block()
        } catch (rejected: FfiException.Rejected) {
            return rejected
        }
        throw AssertionError("$what: expected FfiException.Rejected, got a success result")
    }

    /** Foreign-callback tool handler: sums `a` + `b` (Rust `add_handler`
     * parity) and records the invocation count. */
    private class SumToolHandler : ToolHandler {
        private val calls = AtomicInteger(0)

        override fun handle(argumentsJson: String): String {
            calls.incrementAndGet()
            val args = JSONObject(argumentsJson)
            val a = args.optLong("a", 0L)
            val b = args.optLong("b", 0L)
            return """{"sum":${a + b}}"""
        }

        fun calls(): Int = calls.get()
    }

    /** Foreign-callback tool handler that always throws the given
     * application reject (D16 passthrough row). */
    private class ThrowingToolHandler(private val reject: FfiException.Rejected) : ToolHandler {
        override fun handle(argumentsJson: String): String = throw reject
    }
}
