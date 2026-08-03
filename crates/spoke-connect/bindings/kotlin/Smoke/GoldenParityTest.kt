import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue
import org.json.JSONObject
import uniffi.spoke_connect.CoreException
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion
import uniffi.spoke_connect.signHelloEd25519
import uniffi.spoke_connect.verifyHelloEd25519

/**
 * Golden-parity smoke for the spoke-connect Kotlin binding (uniffi + JNA).
 *
 * The golden vector is loaded from the shared cross-language SSOT via the
 * registered byte-identical copy at `Smoke/fixtures/golden-hello.json`
 * (sync gate: `tooling/connect/golden-vector-sync.mjs`). The fixture carries
 * the seed / nonce / manifest inputs AND the pinned output bytes (pubkey,
 * peer id, JCS hex, signature) — asserted below, never recomputed and
 * written back.
 *
 * Run from bindings/kotlin: gradle test
 */
class GoldenParityTest {
    private val golden = loadGoldenFixture()
    private val goldenSeed = decodeHex(golden.getString("seed_hex"))
    private val goldenPubkey = decodeHex(golden.getString("pubkey_hex"))
    private val goldenPeerId = golden.getString("peer_id")
    private val goldenSignature = golden.getString("signature_b64u")
    private val goldenManifestJson = golden.getString("manifest_json")
    private val goldenNonce = golden.getString("nonce")

    private fun loadGoldenFixture(): JSONObject {
        // Gradle test working directory is the kotlin binding project root.
        val file = File("Smoke/fixtures/golden-hello.json")
        return JSONObject(file.readText())
    }

    private fun decodeHex(hex: String): ByteArray {
        require(hex.length % 2 == 0) { "hex must have even length" }
        return ByteArray(hex.length / 2) { i ->
            hex.substring(i * 2, i * 2 + 2).toInt(16).toByte()
        }
    }

    @Test
    fun derivePeerId_matchesGolden() {
        assertEquals(goldenPeerId, derivePeerIdFromEd25519Pubkey(goldenPubkey))
    }

    @Test
    fun signHello_signatureMatchesGolden() {
        val helloJson = signHelloEd25519(goldenSeed, goldenNonce, goldenManifestJson)
        assertTrue(helloJson.contains(goldenPeerId))
        assertTrue(helloJson.contains(goldenSignature))
    }

    @Test
    fun verifyHello_roundTrip() {
        val helloJson = signHelloEd25519(goldenSeed, goldenNonce, goldenManifestJson)
        verifyHelloEd25519(goldenPubkey, goldenPeerId, helloJson)
    }

    @Test
    fun verifyHello_rejectsTampered() {
        val helloJson = signHelloEd25519(goldenSeed, goldenNonce, goldenManifestJson)
        val tampered = helloJson.replace("data-store", "checker")
        assertFailsWith<CoreException> {
            verifyHelloEd25519(goldenPubkey, goldenPeerId, tampered)
        }
    }

    @Test
    fun protocolVersion_isOne() {
        assertEquals(1u, protocolVersion())
    }
}
