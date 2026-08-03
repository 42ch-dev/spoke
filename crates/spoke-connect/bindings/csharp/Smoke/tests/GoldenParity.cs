using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// Golden-parity assertions for the C# binding: every check reproduces the
/// Rust golden vectors through the generated binding surface.
///
/// The golden vector is loaded from the shared cross-language SSOT via the
/// registered byte-identical copy at `fixtures/golden-hello.json` (copied to
/// the output directory; sync gate: `tooling/connect/golden-vector-sync.mjs`).
/// The fixture carries the seed / nonce / manifest inputs AND the pinned
/// output bytes (pubkey, peer id, JCS hex, signature) — asserted below,
/// never recomputed and written back.
/// </summary>
public static class GoldenParity
{
    private sealed class GoldenFixture
    {
        public string seed_hex { get; set; } = "";
        public string nonce { get; set; } = "";
        public string manifest_json { get; set; } = "";
        public string pubkey_hex { get; set; } = "";
        public string peer_id { get; set; } = "";
        public string jcs_hex { get; set; } = "";
        public string signature_b64u { get; set; } = "";
    }

    private static readonly GoldenFixture Golden = LoadFixture();

    private static GoldenFixture LoadFixture()
    {
        var path = Path.Combine(
            AppContext.BaseDirectory, "fixtures", "golden-hello.json");
        var json = File.ReadAllText(path);
        return JsonSerializer.Deserialize<GoldenFixture>(json)
            ?? throw new Exception("golden-hello.json fixture is empty");
    }

    /// Ed25519 seed bytes 1..=32 — the canonical golden key pair.
    public static byte[] GoldenSeed => Convert.FromHexString(Golden.seed_hex);

    public static byte[] GoldenPubkey => Convert.FromHexString(Golden.pubkey_hex);

    public static string GoldenPeerId => Golden.peer_id;

    /// base64url (no padding) of the raw 64-byte Ed25519 signature over the
    /// golden hello — the same constant the Rust golden tests assert.
    public static string GoldenSignature => Golden.signature_b64u;

    /// Golden manifest as canonical JSON — `authority` omitted (absent
    /// optional field), matching the JCS-signed bytes in the core fixtures.
    public static string GoldenManifestJson => Golden.manifest_json;

    public static string GoldenNonce => Golden.nonce;

    public const ulong ProtocolVersion = 1;

    /// Derive: the golden pubkey must produce the golden peer_id.
    public static void AssertDerivePeerId()
    {
        var actual = SpokeConnectMethods.DerivePeerIdFromEd25519Pubkey(GoldenPubkey);
        AssertEqual(GoldenPeerId, actual, "derive_peer_id");
    }

    /// Sign: the signed hello envelope must embed the golden peer_id and the
    /// golden signature (same signature bytes the Rust golden test asserts).
    public static void AssertSignHelloSignature()
    {
        var helloJson = SpokeConnectMethods.SignHelloEd25519(
            GoldenSeed, GoldenNonce, GoldenManifestJson);
        using var doc = JsonDocument.Parse(helloJson);
        var root = doc.RootElement;

        AssertEqual(GoldenPeerId, root.GetProperty("peer_id").GetString()!,
            "sign_hello peer_id");
        AssertEqual(GoldenSignature, root.GetProperty("signature").GetString()!,
            "sign_hello signature");
    }

    /// Verify: the signed hello must verify against the golden pubkey /
    /// peer_id without throwing.
    public static void AssertVerifyHello()
    {
        var helloJson = SpokeConnectMethods.SignHelloEd25519(
            GoldenSeed, GoldenNonce, GoldenManifestJson);
        SpokeConnectMethods.VerifyHelloEd25519(GoldenPubkey, GoldenPeerId, helloJson);
    }

    /// Tamper rejection: a modified hello must fail verification with the
    /// mapped `CoreException.InvalidHelloSignature`.
    public static void AssertTamperedHelloRejected()
    {
        var helloJson = SpokeConnectMethods.SignHelloEd25519(
            GoldenSeed, GoldenNonce, GoldenManifestJson);
        var tampered = helloJson.Replace("data-store", "checker");

        try
        {
            SpokeConnectMethods.VerifyHelloEd25519(GoldenPubkey, GoldenPeerId, tampered);
        }
        catch (CoreException.InvalidHelloSignature)
        {
            return; // expected
        }
        throw new Exception("tampered hello was accepted (expected InvalidHelloSignature)");
    }

    /// Protocol version: the wire protocol is 1.
    public static void AssertProtocolVersion()
    {
        AssertEqual(ProtocolVersion, SpokeConnectMethods.ProtocolVersion(),
            "protocol_version");
    }

    /// Allowlist gate (is_allowlisted): listed peers accepted, unlisted peers
    /// rejected, empty allowlist fails closed. Exercises `string[]` collection
    /// marshalling across the FFI boundary.
    public static void AssertAllowlistBehavior()
    {
        AssertTrue(SpokeConnectMethods.IsAllowlisted(new[] { "peer-a", "peer-b" }, "peer-a"),
            "allowlist accepts a listed peer");
        AssertTrue(!SpokeConnectMethods.IsAllowlisted(new[] { "peer-a" }, "peer-c"),
            "allowlist rejects an unlisted peer");
        AssertTrue(!SpokeConnectMethods.IsAllowlisted(Array.Empty<string>(), "peer-a"),
            "empty allowlist fails closed");
    }

    /// Dispatch gate (dispatch_allowed): the core-op table is honored and an
    /// unknown op is denied.
    public static void AssertDispatchGate()
    {
        AssertTrue(SpokeConnectMethods.DispatchAllowed("check", new[] { "spoke-baseline" }),
            "dispatch allowed with required capability");
        AssertTrue(!SpokeConnectMethods.DispatchAllowed("check", new[] { "l2-computable" }),
            "dispatch denied without required capability");
        AssertTrue(!SpokeConnectMethods.DispatchAllowed("custom-op", new[] { "spoke-baseline" }),
            "unknown op denied");
    }

    /// Capability table (required_capability): Some for core ops, None for
    /// product-defined ops. Exercises `string?` optional marshalling.
    public static void AssertRequiredCapability()
    {
        AssertEqual("spoke-baseline", SpokeConnectMethods.RequiredCapability("check"),
            "required_capability Some");
        AssertEqual<string?>(null, SpokeConnectMethods.RequiredCapability("custom-op"),
            "required_capability None");
    }

    /// Response correlation (check_response_correlation): an exact echo
    /// passes; a mismatched sequence raises the mapped `CorrelationMismatch`.
    public static void AssertResponseCorrelation()
    {
        SpokeConnectMethods.CheckResponseCorrelation(
            "sess-1", 0UL, "req-1", "sess-1", 0UL, "req-1");

        try
        {
            SpokeConnectMethods.CheckResponseCorrelation(
                "sess-1", 0UL, "req-1", "sess-1", 1UL, "req-1");
        }
        catch (CoreInvokeException.CorrelationMismatch)
        {
            return; // expected
        }
        throw new Exception("mismatched correlation accepted (expected CorrelationMismatch)");
    }

    /// Nonce store lifecycle (NonceStore object): first use records, replay is
    /// rejected, and nonces are scoped per sender peer_id.
    public static void AssertNonceStoreLifecycle()
    {
        using var store = new NonceStore();
        AssertTrue(store.CheckAndRecord("peer-a", "nonce-1"), "first nonce recorded");
        AssertTrue(!store.CheckAndRecord("peer-a", "nonce-1"), "replay rejected");
        AssertTrue(store.CheckAndRecord("peer-b", "nonce-1"), "nonce scoped per peer");
    }

    /// Sequence objects (OutboundSequence / InboundSequence): outbound
    /// allocates 0 then 1; inbound accepts 0, advances the expectation to 1,
    /// and rejects a replay with the fielded `InboundSequenceMismatch` error.
    public static void AssertSequenceObjectsLifecycle()
    {
        using var outbound = new OutboundSequence();
        AssertEqual(0UL, outbound.Allocate(), "outbound first allocate");
        AssertEqual(1UL, outbound.Allocate(), "outbound second allocate");

        using var inbound = new InboundSequence();
        AssertEqual(1UL, inbound.Advance(0), "inbound advance returns next expectation");
        try
        {
            inbound.Advance(0);
        }
        catch (CoreInvokeException.InboundSequenceMismatch ex)
        {
            AssertEqual(1UL, ex.expected, "inbound mismatch expected");
            AssertEqual(0L, ex.actual, "inbound mismatch actual");
            return; // expected
        }
        throw new Exception("replayed inbound sequence accepted (expected InboundSequenceMismatch)");
    }

    private static void AssertEqual<T>(T expected, T actual, string what)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new Exception(
                $"{what}: expected '{expected}', got '{actual}'");
        }
    }

    private static void AssertTrue(bool value, string what)
    {
        if (!value)
        {
            throw new Exception($"{what}: expected true, got false");
        }
    }
}
