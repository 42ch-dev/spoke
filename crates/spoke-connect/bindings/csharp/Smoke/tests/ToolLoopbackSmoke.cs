using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// Tool faces over the loopback pair (D15/D16), run in the DEFAULT Smoke run
/// against the committed production binding (no smoke host required): both
/// ends are FFI objects — the responder serves a foreign
/// <see cref="ToolHandler"/>, the dialer serves reverse invokes through
/// <c>RemoteAdapterFFI.RegisterToolHandler</c>, unregistered tools deny with
/// <c>op_unsupported</c>, and a handler-thrown application reject passes
/// through verbatim.
/// </summary>
public static class ToolLoopbackSmoke
{
    public static void Run()
    {
        var fixture = LoopbackAssert.LoadFixture();
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
            null,
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
            LoopbackAssert.AssertEqual("Established", dialer.State(), "tool dialer state");
            WaitForState("tool responder handshake", () => responder.State(), "Established");

            // 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
            var responderSum = new SumToolHandler();
            responder.RegisterToolHandler("tools.math.add", responderSum);
            var sumJson = dialer.InvokeTool("tools.math.add", """{"a": 1, "b": 2}""");
            using var sumDoc = JsonDocument.Parse(sumJson);
            LoopbackAssert.AssertEqual(
                3L,
                sumDoc.RootElement.GetProperty("sum").GetInt64(),
                "dialer invoke_tool answered by responder foreign ToolHandler");
            LoopbackAssert.AssertEqual(1, responderSum.Calls, "responder handler invocation count");

            // 2. Responder FFI invoke_tool -> dialer-side handler registered
            //    via RemoteAdapterFFI.RegisterToolHandler.
            var dialerSum = new SumToolHandler();
            dialer.RegisterToolHandler("tools.math.add", dialerSum);
            var reverseSumJson = responder.InvokeTool("tools.math.add", """{"a": 21, "b": 21}""");
            using var reverseSumDoc = JsonDocument.Parse(reverseSumJson);
            LoopbackAssert.AssertEqual(
                42L,
                reverseSumDoc.RootElement.GetProperty("sum").GetInt64(),
                "responder invoke_tool answered by dialer-side handler");
            LoopbackAssert.AssertEqual(1, dialerSum.Calls, "dialer handler invocation count");

            // 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
            var denied = AssertRejected(() => dialer.InvokeTool("tools.echo.boom", "{}"));
            LoopbackAssert.AssertEqual("CAPABILITY_PORT_MISSING", denied.code, "unregistered tool deny code");
            LoopbackAssert.AssertEqual("op_unsupported", denied.wireCode, "unregistered tool deny wire_code");

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
            LoopbackAssert.AssertEqual("REVISION_CONFLICT", passed.code, "reject passthrough code");
            LoopbackAssert.AssertEqual("foreign handler rejected", passed.message, "reject passthrough message");
            LoopbackAssert.AssertEqual("op_unsupported", passed.wireCode, "reject passthrough wire_code");

            // 5. Handler rejects with an unknown code string -> the host
            //    observes the INTERNAL_ERROR downgrade (message preserved,
            //    details re-hung) — the foreign code cannot be represented by
            //    the typed SpokeRejectCode, so the bridge falls back.
            dialer.RegisterToolHandler(
                "tools.echo.boom",
                new ThrowingToolHandler(
                    new FfiException.Rejected(
                        "NOT_A_WIRE_CODE",
                        "unknown code message",
                        null,
                        "op_unsupported")));
            var downgraded = AssertRejected(() => responder.InvokeTool("tools.echo.boom", "{}"));
            LoopbackAssert.AssertEqual("INTERNAL_ERROR", downgraded.code, "unknown-code downgrade code");
            LoopbackAssert.AssertEqual("unknown code message", downgraded.message, "unknown-code downgrade message");
            LoopbackAssert.AssertEqual("op_unsupported", downgraded.wireCode, "unknown-code downgrade wire_code");

            // 6. Handler throws a foreign (non-Rejected) fault -> contained to
            //    INTERNAL_ERROR with no details; the session survives and the
            //    serve loop still answers the next healthy reverse invoke.
            //    Channel caveat: the C# vendored-fork bindgen carries the
            //    stock uniffi callback-error machinery unpatched (no
            //    fielded-error patch script like Kotlin's) — the fielded
            //    ERROR path is proven by steps 4–5 and the plain-fault path
            //    by this step; assert only what the stock trampoline can
            //    express (plain System.Exception -> unexpected callback error).
            dialer.RegisterToolHandler(
                "tools.echo.boom",
                new FaultingToolHandler(new Exception("foreign fault")));
            var contained = AssertRejected(() => responder.InvokeTool("tools.echo.boom", "{}"));
            LoopbackAssert.AssertEqual("INTERNAL_ERROR", contained.code, "foreign-fault containment code");
            LoopbackAssert.AssertEqual(null, contained.wireCode, "foreign-fault containment wire_code (details None)");

            var healthyJson = responder.InvokeTool("tools.math.add", """{"a": 21, "b": 21}""");
            using var healthyDoc = JsonDocument.Parse(healthyJson);
            LoopbackAssert.AssertEqual(
                42L,
                healthyDoc.RootElement.GetProperty("sum").GetInt64(),
                "serve loop survives foreign-fault containment");
            LoopbackAssert.AssertEqual(2, dialerSum.Calls, "dialer handler invocation count after containment");
        }
        finally
        {
            dialer.Close();
            responder.Close();
            LoopbackAssert.AssertEqual("Closed", dialer.State(), "tool dialer state after close");
            LoopbackAssert.AssertEqual("Closed", responder.State(), "tool responder state after close");
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

    /// <summary>Foreign-callback tool handler that always throws a plain
    /// (non-Rejected) fault — the D16 non-contract failure row.</summary>
    private sealed class FaultingToolHandler : ToolHandler
    {
        private readonly Exception _fault;

        public FaultingToolHandler(Exception fault) => _fault = fault;

        public string Handle(string argumentsJson) => throw _fault;
    }
}
