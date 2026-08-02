using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// Golden-parity assertions for the C# binding: every check reproduces the
/// Rust golden vectors (crates/spoke-connect/src/ffi.rs tests + the
/// connect-identity-proof tool) through the generated binding surface.
///
/// The nonce below is joined from two literals to mirror the Rust fixture
/// pattern (the value is exactly `golden-nonce-000000000001`).
/// </summary>
public static class GoldenParity
{
    /// Ed25519 seed bytes 1..=32 — the canonical golden key pair.
    public static readonly byte[] GoldenSeed =
        Enumerable.Range(1, 32).Select(i => (byte)i).ToArray();

    public static readonly byte[] GoldenPubkey =
    {
        0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9,
        0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b, 0xa7,
        0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7,
        0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04, 0x96, 0x64,
    };

    public const string GoldenPeerId =
        "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf";

    /// base64url (no padding) of the raw 64-byte Ed25519 signature over the
    /// golden hello — the same constant the Rust golden tests assert.
    public const string GoldenSignature =
        "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg";

    /// Golden manifest as canonical JSON — `authority` omitted (absent
    /// optional field), matching the JCS-signed bytes in the core fixtures.
    public const string GoldenManifestJson =
        "{\"capabilities\":[\"spoke-baseline\"],\"extensions\":{},\"host_id\":\"golden-host\",\"namespaces\":[],\"roles\":[\"data-store\"],\"schema_version\":1}";

    public static string GoldenNonce() =>
        string.Concat("golden-nonce", "-", "000000000001");

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
            GoldenSeed, GoldenNonce(), GoldenManifestJson);
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
            GoldenSeed, GoldenNonce(), GoldenManifestJson);
        SpokeConnectMethods.VerifyHelloEd25519(GoldenPubkey, GoldenPeerId, helloJson);
    }

    /// Tamper rejection: a modified hello must fail verification with the
    /// mapped `CoreException.InvalidHelloSignature`.
    public static void AssertTamperedHelloRejected()
    {
        var helloJson = SpokeConnectMethods.SignHelloEd25519(
            GoldenSeed, GoldenNonce(), GoldenManifestJson);
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
