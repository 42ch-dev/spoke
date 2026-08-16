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

        RunToolFaces(fixture);
    }

    /// <summary>
    /// Tool faces over the loopback pair (D15/D16): both ends are FFI objects —
    /// the responder serves a foreign <see cref="ToolHandler"/>, the dialer
    /// serves reverse invokes through <c>RemoteAdapterFFI.RegisterToolHandler</c>,
    /// unregistered tools deny with <c>op_unsupported</c>, and a handler-thrown
    /// application reject passes through verbatim.
    /// </summary>
    private static void RunToolFaces(LoopbackFixture fixture)
    {
        var seedClient = Convert.FromHexString(fixture.seed_client_hex);
        var seedHost = Convert.FromHexString(fixture.seed_host_hex);
        var pubkeyHost = Convert.FromHexString(fixture.pubkey_host_hex);
        var pubkeyClient = Convert.FromHexString(fixture.pubkey_client_hex);

        var pair = SpokeConnectMethods.LoopbackTransportPair();
        // The accept-side constructor returns immediately in `Handshaking`
        // (D16): the dialer hello is the sync point, so the smoke polls
        // `state()` (bounded) to `Established` before invoking.
        var responder = SpokeConnectMethods.ConnectResponderFfi(
            new LoopbackCallbackTransport(pair.Server()),
            seedHost,
            ToolManifestJson("test-responder"),
            [fixture.peer_id_client],
            new Dictionary<string, byte[]> { [fixture.peer_id_client] = pubkeyClient },
            null);
        var dialer = SpokeConnectMethods.ConnectRemoteAdapterFfi(
            new LoopbackCallbackTransport(pair.Client()),
            seedClient,
            ToolManifestJson("test-client"),
            pubkeyHost,
            [fixture.peer_id_host],
            null);

        try
        {
            AssertEqual("Established", dialer.State(), "tool dialer state");
            WaitForState("tool responder handshake", () => responder.State(), "Established");

            // 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
            var responderSum = new SumToolHandler();
            responder.RegisterToolHandler("tools.math.add", responderSum);
            var sumJson = dialer.InvokeTool("tools.math.add", """{"a": 1, "b": 2}""");
            using var sumDoc = JsonDocument.Parse(sumJson);
            AssertEqual(
                3L,
                sumDoc.RootElement.GetProperty("sum").GetInt64(),
                "dialer invoke_tool answered by responder foreign ToolHandler");
            AssertEqual(1, responderSum.Calls, "responder handler invocation count");

            // 2. Responder FFI invoke_tool -> dialer-side handler registered
            //    via RemoteAdapterFFI.RegisterToolHandler.
            var dialerSum = new SumToolHandler();
            dialer.RegisterToolHandler("tools.math.add", dialerSum);
            var reverseSumJson = responder.InvokeTool("tools.math.add", """{"a": 21, "b": 21}""");
            using var reverseSumDoc = JsonDocument.Parse(reverseSumJson);
            AssertEqual(
                42L,
                reverseSumDoc.RootElement.GetProperty("sum").GetInt64(),
                "responder invoke_tool answered by dialer-side handler");
            AssertEqual(1, dialerSum.Calls, "dialer handler invocation count");

            // 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
            var denied = AssertRejected(() => dialer.InvokeTool("tools.echo.boom", "{}"));
            AssertEqual("CAPABILITY_PORT_MISSING", denied.code, "unregistered tool deny code");
            AssertEqual("op_unsupported", denied.wireCode, "unregistered tool deny wire_code");

            // 4. Handler-thrown application reject passes through verbatim
            //    (kind / wire_code re-hung onto details by the bridge).
            dialer.RegisterToolHandler(
                "tools.echo.boom",
                new ThrowingToolHandler(
                    new FfiException.Rejected(
                        "REVISION_CONFLICT",
                        "foreign handler rejected",
                        null,
                        "op_unsupported")));
            var passed = AssertRejected(() => responder.InvokeTool("tools.echo.boom", "{}"));
            AssertEqual("REVISION_CONFLICT", passed.code, "reject passthrough code");
            AssertEqual("foreign handler rejected", passed.message, "reject passthrough message");
            AssertEqual("op_unsupported", passed.wireCode, "reject passthrough wire_code");
        }
        finally
        {
            dialer.Close();
            responder.Close();
            AssertEqual("Closed", dialer.State(), "tool dialer state after close");
            AssertEqual("Closed", responder.State(), "tool responder state after close");
        }
    }

    /// <summary>Tool-carrying manifest — every tool capability also sits in
    /// `capabilities[]` so the negotiated set includes the `tools.*` ops
    /// (D13 dispatch gate). Mirror of the Rust `tool_manifest` test helper.</summary>
    private static string ToolManifestJson(string hostId)
    {
        return """
            {"schema_version":1,"host_id":"__HOST_ID__","roles":["data-store"],
            "capabilities":["spoke-baseline","tools.math.add","tools.echo.echo","tools.echo.boom"],
            "namespaces":["math","echo","toy_world"],"extensions":{},
            "tools":[
            {"schema_version":1,"capability_id":"tools.math.add","op":"tools.math.add","description":"Add two integers","input":{"type":"object"},"output":{"type":"object"}},
            {"schema_version":1,"capability_id":"tools.echo.echo","op":"tools.echo.echo","description":"Echo the arguments","input":{"type":"object"},"output":{"type":"object"}},
            {"schema_version":1,"capability_id":"tools.echo.boom","op":"tools.echo.boom","description":"Explodes","input":{"type":"object"},"output":{"type":"object"}}
            ]}
            """.Replace("__HOST_ID__", hostId);
    }

    /// <summary>Bounded poll for the handshake to settle — the responder
    /// constructor returns immediately in `Handshaking` (D16); a handshake
    /// failure surfaces as `Closed`, never a thrown constructor error.</summary>
    private static void WaitForState(string what, Func<string> state, string expected)
    {
        var deadline = DateTime.UtcNow.AddSeconds(5);
        var last = state();
        while (last != expected)
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new Exception($"{what}: timed out waiting for {expected} (last: {last})");
            }
            Thread.Sleep(10);
            last = state();
        }
    }

    private static FfiException.Rejected AssertRejected(Func<string> invoke)
    {
        try
        {
            _ = invoke();
            throw new Exception("expected FfiException.Rejected, got a success result");
        }
        catch (FfiException.Rejected rejected)
        {
            return rejected;
        }
    }

    /// <summary>Foreign-callback tool handler: sums `a` + `b` (Rust
    /// `add_handler` parity) and records the invocation count.</summary>
    private sealed class SumToolHandler : ToolHandler
    {
        private int _calls;

        public int Calls => _calls;

        public string Handle(string argumentsJson)
        {
            Interlocked.Increment(ref _calls);
            using var doc = JsonDocument.Parse(argumentsJson);
            var root = doc.RootElement;
            long a = root.TryGetProperty("a", out var aEl) && aEl.ValueKind == JsonValueKind.Number
                ? aEl.GetInt64()
                : 0;
            long b = root.TryGetProperty("b", out var bEl) && bEl.ValueKind == JsonValueKind.Number
                ? bEl.GetInt64()
                : 0;
            return $$"""{"sum": {{a + b}}}""";
        }
    }

    /// <summary>Foreign-callback tool handler that always throws the given
    /// application reject (D16 passthrough row).</summary>
    private sealed class ThrowingToolHandler : ToolHandler
    {
        private readonly FfiException.Rejected _reject;

        public ThrowingToolHandler(FfiException.Rejected reject) => _reject = reject;

        public string Handle(string argumentsJson) => throw _reject;
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
