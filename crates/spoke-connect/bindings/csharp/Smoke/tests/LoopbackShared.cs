using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// Shared loopback harness pieces — used by the smoke-host-tagged
/// RemoteAdapterFFI put/get smoke and the tool-faces loopback smoke (which
/// runs in the default Smoke run against the committed production binding,
/// no smoke host needed).
/// </summary>
internal sealed class LoopbackFixture
{
    public string seed_client_hex { get; set; } = "";
    public string seed_host_hex { get; set; } = "";
    public string pubkey_host_hex { get; set; } = "";
    public string pubkey_client_hex { get; set; } = "";
    public string peer_id_host { get; set; } = "";
    public string peer_id_client { get; set; } = "";
    public string client_manifest_json { get; set; } = "";
    public string session_id { get; set; } = "";
    public string entry_id { get; set; } = "";
    public string entry_canonical_name { get; set; } = "";
}

internal sealed class LoopbackCallbackTransport : Transport
{
    private readonly LoopbackTransport _inner;

    public LoopbackCallbackTransport(LoopbackTransport inner) => _inner = inner;

    public void Send(byte[] envelope) => _inner.Send(envelope);

    public byte[] Recv() => _inner.Recv();

    public void Close() => _inner.Close();
}

internal static class LoopbackAssert
{
    public static LoopbackFixture LoadFixture()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "fixtures", "loopback-smoke.json");
        var json = File.ReadAllText(path);
        return JsonSerializer.Deserialize<LoopbackFixture>(json)
            ?? throw new Exception("loopback-smoke.json fixture is empty");
    }

    public static void AssertEqual<T>(T expected, T actual, string what)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new Exception($"{what}: expected '{expected}', got '{actual}'");
        }
    }
}
