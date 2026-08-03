import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue
import uniffi.spoke_connect.CoreException
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion
import uniffi.spoke_connect.signHelloEd25519
import uniffi.spoke_connect.verifyHelloEd25519

/**
 * Golden-parity smoke for the spoke-connect Kotlin binding (uniffi + JNA).
 *
 * Vectors match crates/spoke-connect/src/ffi.rs tests and other binding smokes.
 * Run from bindings/kotlin: gradle test
 */
class GoldenParityTest {
    private val goldenSeed = ByteArray(32) { (it + 1).toByte() }
    private val goldenPubkey =
        byteArrayOf(
            0x79,
            0xB5.toByte(),
            0x56,
            0x2E,
            0x8F.toByte(),
            0xE6.toByte(),
            0x54,
            0xF9.toByte(),
            0x40,
            0x78,
            0xB1.toByte(),
            0x12,
            0xE8.toByte(),
            0xA9.toByte(),
            0x8B.toByte(),
            0xA7.toByte(),
            0x90.toByte(),
            0x1F,
            0x85.toByte(),
            0x3A,
            0xE6.toByte(),
            0x95.toByte(),
            0xBE.toByte(),
            0xD7.toByte(),
            0xE0.toByte(),
            0xE3.toByte(),
            0x91.toByte(),
            0x0B,
            0xAD.toByte(),
            0x04,
            0x96.toByte(),
            0x64,
        )
    private val goldenPeerId = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"
    private val goldenSignature =
        "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg"
    private val goldenManifestJson =
        """{"capabilities":["spoke-baseline"],"extensions":{},"host_id":"golden-host","namespaces":[],"roles":["data-store"],"schema_version":1}"""
    private val goldenNonce = "golden-nonce-000000000001"

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
