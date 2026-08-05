#if SMOKE_HOST
using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// RemoteAdapterFFI loopback smoke — callback <see cref="Transport"/> over an
/// in-memory pair with the reference ToyWorld smoke host.
/// </summary>
public static class LoopbackSmoke
{
    private sealed class LoopbackFixture
    {
        public string seed_client_hex { get; set; } = "";
        public string pubkey_host_hex { get; set; } = "";
        public string peer_id_host { get; set; } = "";
        public string client_manifest_json { get; set; } = "";
        public string session_id { get; set; } = "";
        public string entry_id { get; set; } = "";
        public string entry_canonical_name { get; set; } = "";
    }

    private sealed class LoopbackCallbackTransport : Transport
    {
        private readonly LoopbackTransport _inner;

        public LoopbackCallbackTransport(LoopbackTransport inner) => _inner = inner;

        public void Send(byte[] envelope) => _inner.Send(envelope);

        public byte[] Recv() => _inner.Recv();

        public void Close() => _inner.Close();
    }

    public static void Run()
    {
        var fixture = LoadFixture();
        var seedClient = Convert.FromHexString(fixture.seed_client_hex);
        var pubkeyHost = Convert.FromHexString(fixture.pubkey_host_hex);

        var pair = SpokeConnectMethods.LoopbackTransportPair();
        var host = SpokeConnectMethods.StartLoopbackSmokeHost(pair.Server());
        var transport = new LoopbackCallbackTransport(pair.Client());
        var adapter = SpokeConnectMethods.ConnectRemoteAdapterFfi(
            transport,
            seedClient,
            fixture.client_manifest_json,
            pubkeyHost,
            [fixture.peer_id_host],
            null);

        try
        {
            AssertEqual("Established", adapter.State(), "adapter state");
            AssertEqual(fixture.session_id, adapter.SessionId(), "adapter session_id");
            AssertEqual(fixture.session_id, host.SessionId(), "host session_id");
            AssertEqual(fixture.peer_id_host, adapter.RemotePeerId(), "remote_peer_id");

            var remoteManifest = adapter.RemoteManifest();
            if (remoteManifest is null)
            {
                throw new Exception("remote_manifest is null");
            }

            using var manifestDoc = JsonDocument.Parse(remoteManifest);
            AssertEqual(
                "test-host",
                manifestDoc.RootElement.GetProperty("host_id").GetString()!,
                "remote manifest host_id");

            var entryJson = KnowledgeEntryJson(fixture.entry_id, fixture.entry_canonical_name);
            var putJson = adapter.PutKnowledgeEntry(entryJson, null);
            using var putDoc = JsonDocument.Parse(putJson);
            AssertEqual(
                fixture.entry_id,
                putDoc.RootElement.GetProperty("entry_id").GetString()!,
                "putKnowledgeEntry entry_id");

            var getJson = adapter.GetKnowledgeEntry(fixture.entry_id);
            using var getDoc = JsonDocument.Parse(getJson);
            AssertEqual(
                fixture.entry_id,
                getDoc.RootElement.GetProperty("entry_id").GetString()!,
                "getKnowledgeEntry entry_id");
            AssertEqual(
                fixture.entry_canonical_name,
                getDoc.RootElement.GetProperty("canonical_name").GetString()!,
                "getKnowledgeEntry canonical_name");
        }
        finally
        {
            adapter.Close();
            AssertEqual("Closed", adapter.State(), "adapter state after close");
            host.Close();
        }
    }

    private static LoopbackFixture LoadFixture()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "fixtures", "loopback-smoke.json");
        var json = File.ReadAllText(path);
        return JsonSerializer.Deserialize<LoopbackFixture>(json)
            ?? throw new Exception("loopback-smoke.json fixture is empty");
    }

    private static string KnowledgeEntryJson(string entryId, string canonicalName)
    {
        return string.Concat(
            "{\"schema_version\":1,\"entry_id\":\"",
            entryId,
            "\",\"entry_type\":\"character\",\"canonical_name\":\"",
            canonicalName,
            "\",\"status\":\"provisional\",\"body\":{\"summary\":\"Upserted over the loopback: ",
            entryId,
            "\"},\"extensions\":{}}");
    }

    private static void AssertEqual<T>(T expected, T actual, string what)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new Exception($"{what}: expected '{expected}', got '{actual}'");
        }
    }
}
#endif
