#if SMOKE_HOST
using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// RemoteAdapterFFI loopback smoke — callback <see cref="Transport"/> over an
/// in-memory pair with the reference ToyWorld smoke host. Smoke-host-only:
/// the tool-faces loopback smoke (no smoke host needed) lives in
/// <see cref="ToolLoopbackSmoke"/> and runs in the default Smoke run.
/// </summary>
public static class LoopbackSmoke
{
    public static void Run()
    {
        var fixture = LoopbackAssert.LoadFixture();
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
            LoopbackAssert.AssertEqual("Established", adapter.State(), "adapter state");
            LoopbackAssert.AssertEqual(fixture.session_id, adapter.SessionId(), "adapter session_id");
            LoopbackAssert.AssertEqual(fixture.session_id, host.SessionId(), "host session_id");
            LoopbackAssert.AssertEqual(fixture.peer_id_host, adapter.RemotePeerId(), "remote_peer_id");

            var remoteManifest = adapter.RemoteManifest();
            if (remoteManifest is null)
            {
                throw new Exception("remote_manifest is null");
            }

            using var manifestDoc = JsonDocument.Parse(remoteManifest);
            LoopbackAssert.AssertEqual(
                "test-host",
                manifestDoc.RootElement.GetProperty("host_id").GetString()!,
                "remote manifest host_id");

            var entryJson = KnowledgeEntryJson(fixture.entry_id, fixture.entry_canonical_name);
            var putJson = adapter.PutKnowledgeEntry(entryJson, null);
            using var putDoc = JsonDocument.Parse(putJson);
            LoopbackAssert.AssertEqual(
                fixture.entry_id,
                putDoc.RootElement.GetProperty("entry_id").GetString()!,
                "putKnowledgeEntry entry_id");

            var getJson = adapter.GetKnowledgeEntry(fixture.entry_id);
            using var getDoc = JsonDocument.Parse(getJson);
            LoopbackAssert.AssertEqual(
                fixture.entry_id,
                getDoc.RootElement.GetProperty("entry_id").GetString()!,
                "getKnowledgeEntry entry_id");
            LoopbackAssert.AssertEqual(
                fixture.entry_canonical_name,
                getDoc.RootElement.GetProperty("canonical_name").GetString()!,
                "getKnowledgeEntry canonical_name");
        }
        finally
        {
            adapter.Close();
            LoopbackAssert.AssertEqual("Closed", adapter.State(), "adapter state after close");
            host.Close();
        }
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
}
#endif
