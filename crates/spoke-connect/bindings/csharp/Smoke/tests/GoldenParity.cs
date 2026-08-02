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

    private static void AssertEqual<T>(T expected, T actual, string what)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new Exception(
                $"{what}: expected '{expected}', got '{actual}'");
        }
    }
}
