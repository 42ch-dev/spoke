using System.Text.Json;
using uniffi.spoke_connect;

namespace SpokeConnectSmoke.Tests;

/// <summary>
/// Optional-port dialer ops + responder ports serving over the loopback pair
/// (D16), run in the DEFAULT Smoke run against the committed production
/// binding (no smoke host required): the responder serves baseline + optional
/// `port.*` families through a foreign <see cref="PortsHandler"/> (user lock),
/// the dialer drives `Project` / `Compute` / `ListForkTimelineEvents`, and the
/// error rows — capability-gate deny, absent-ports fail-closed deny, and
/// foreign-fault containment with serve-loop survival — mirror the Rust
/// `connect_responder_ffi_tests` battery. The router is untouched: optional
/// ops ride the per-peer `RemoteAdapterFFI`.
/// </summary>
public static class PortsLoopbackSmoke
{
    public static void Run()
    {
        var fixture = LoopbackAssert.LoadFixture();
        var seedClient = Convert.FromHexString(fixture.seed_client_hex);
        var seedHost = Convert.FromHexString(fixture.seed_host_hex);
        var pubkeyHost = Convert.FromHexString(fixture.pubkey_host_hex);
        var pubkeyClient = Convert.FromHexString(fixture.pubkey_client_hex);

        RunServingRoundTrips(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunAbsentPortsDeny(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunCapabilityDeny(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
    }

    /// <summary>Baseline + optional round-trips through a foreign
    /// <see cref="PortsHandler"/>, plus the error rows (application-reject
    /// passthrough, malformed-JSON pre-validation, foreign-fault containment
    /// with session survival).</summary>
    private static void RunServingRoundTrips(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var handler = new SmokePortsHandler();
        var (responder, dialer) = DialPair(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, handler);

        try
        {
            LoopbackAssert.AssertEqual("Established", dialer.State(), "ports dialer state");
            WaitForState("ports responder handshake", () => responder.State(), "Established");

            // 1. Baseline round-trip through the foreign ports handler:
            //    put stores the entry JSON in the handler, get serves it back.
            //    The wire carries the canonicalized entry JSON (typed
            //    round-trip), so compare semantically, not byte-wise.
            var entryJson = KnowledgeEntryJson("kb_ffi_ports_put", "FFI Ports Put");
            var putJson = dialer.PutKnowledgeEntry(entryJson, null);
            using var putDoc = JsonDocument.Parse(putJson);
            LoopbackAssert.AssertEqual(
                "kb_ffi_ports_put",
                putDoc.RootElement.GetProperty("entry_id").GetString(),
                "put through the foreign ports handler");
            var getJson = dialer.GetKnowledgeEntry("kb_ffi_ports_put");
            using var getDoc = JsonDocument.Parse(getJson);
            LoopbackAssert.AssertEqual(
                "FFI Ports Put",
                getDoc.RootElement.GetProperty("canonical_name").GetString(),
                "get through the foreign ports handler");

            // 2. Application-reject passthrough: an unknown entry rejects
            //    with the handler's locked code + re-hung kind (ordinary
            //    deny, NOT containment).
            var missing = AssertRejected(() => dialer.GetKnowledgeEntry("kb_ffi_ports_missing"));
            LoopbackAssert.AssertEqual("KNOWLEDGE_ENTRY_NOT_FOUND", missing.code, "unknown entry reject code");
            LoopbackAssert.AssertEqual("store_miss", missing.kind, "unknown entry reject kind (re-hung)");
            LoopbackAssert.AssertEqual(null, missing.wireCode, "unknown entry reject wire_code");

            // 3. Optional dialer ops round-trip through the callback
            //    (l2-computable / l5-fork negotiated by both manifests).
            var projectJson = dialer.Project(ProjectRequestJson());
            using var projectDoc = JsonDocument.Parse(projectJson);
            LoopbackAssert.AssertEqual(
                "sess_ffi_ports", projectDoc.RootElement.GetProperty("session_id").GetString(), "project session_id");
            LoopbackAssert.AssertEqual(
                "kb_ffi_ports_proj", projectDoc.RootElement.GetProperty("entry_id").GetString(), "project entry_id");
            var projectComputable = projectDoc.RootElement.GetProperty("computable");
            LoopbackAssert.AssertEqual(
                2.4, projectComputable.GetProperty("tide_level").GetDouble(), "project computable tide_level");
            LoopbackAssert.AssertEqual(
                38, projectComputable.GetProperty("cargo_tons").GetInt32(), "project computable cargo_tons");

            var computeJson = dialer.Compute(ComputeRequestJson());
            using var computeDoc = JsonDocument.Parse(computeJson);
            AssertComputableState(computeDoc.RootElement.GetProperty("computable"), "compute echoes the request computable");
            AssertComputableState(computeDoc.RootElement.GetProperty("state"), "compute settle state");

            var eventsJson = dialer.ListForkTimelineEvents(ForkScopeJson());
            using var eventsDoc = JsonDocument.Parse(eventsJson);
            var events = eventsDoc.RootElement;
            LoopbackAssert.AssertEqual(1, events.GetArrayLength(), "fork timeline event count");
            LoopbackAssert.AssertEqual(
                "evt_tw_ffi_storm", events[0].GetProperty("timeline_event_id").GetString(), "fork event id");
            LoopbackAssert.AssertEqual(
                "fork_tw_ffi_events", events[0].GetProperty("fork_id").GetString(), "fork event fork_id");

            // 4. Malformed JSON is rejected locally (INVALID_INPUT, zero wire
            //    traffic) — the dialer pre-validation row per op.
            var badProject = AssertRejected(() => dialer.Project("{ not json"));
            LoopbackAssert.AssertEqual("INVALID_INPUT", badProject.code, "malformed project json code");
            LoopbackAssert.AssertEqual(null, badProject.wireCode, "malformed project json wire_code");

            // 5. Foreign-fault containment: the handler faults on
            //    kb_ffi_ports_boom -> INTERNAL_ERROR with no details; the
            //    session survives and the serve loop answers the next
            //    healthy put.
            var contained = AssertRejected(() => dialer.GetKnowledgeEntry("kb_ffi_ports_boom"));
            LoopbackAssert.AssertEqual("INTERNAL_ERROR", contained.code, "foreign-fault containment code");
            LoopbackAssert.AssertEqual(null, contained.kind, "foreign-fault containment kind");
            LoopbackAssert.AssertEqual(null, contained.wireCode, "foreign-fault containment wire_code (details None)");

            var healthyJson = dialer.PutKnowledgeEntry(KnowledgeEntryJson("kb_ffi_ports_after", "After Containment"), null);
            using var healthyDoc = JsonDocument.Parse(healthyJson);
            LoopbackAssert.AssertEqual(
                "kb_ffi_ports_after",
                healthyDoc.RootElement.GetProperty("entry_id").GetString(),
                "serve loop survives foreign-fault containment");
        }
        finally
        {
            dialer.Close();
            responder.Close();
            LoopbackAssert.AssertEqual("Closed", dialer.State(), "ports dialer state after close");
            LoopbackAssert.AssertEqual("Closed", responder.State(), "ports responder state after close");
        }
    }

    /// <summary>Absent-`ports` constructor is still valid (default deny):
    /// the responder is built without a <see cref="PortsHandler"/> while both
    /// manifests negotiate the optional families — the capability gate
    /// passes, the serving probe finds no ports face, and every optional op
    /// denies fail-closed with the preserved `op_unsupported` wire code.</summary>
    private static void RunAbsentPortsDeny(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var (responder, dialer) = DialPair(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, null);

        try
        {
            LoopbackAssert.AssertEqual("Established", dialer.State(), "absent-ports dialer state");
            WaitForState("absent-ports responder handshake", () => responder.State(), "Established");

            AssertOptionalOpsDenied(dialer, "absent-ports deny");
        }
        finally
        {
            dialer.Close();
            responder.Close();
            LoopbackAssert.AssertEqual("Closed", dialer.State(), "absent-ports dialer state after close");
            LoopbackAssert.AssertEqual("Closed", responder.State(), "absent-ports responder state after close");
        }
    }

    /// <summary>Capability-gate deny: default manifests advertise
    /// `spoke-baseline` only, so the negotiated set lacks l2-computable /
    /// l5-fork and every optional op is denied at the responder's dispatch
    /// gate with the preserved `op_unsupported` wire code.</summary>
    private static void RunCapabilityDeny(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var pair = SpokeConnectMethods.LoopbackTransportPair();
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
            LoopbackAssert.AssertEqual("Established", dialer.State(), "capability-deny dialer state");
            WaitForState("capability-deny responder handshake", () => responder.State(), "Established");

            AssertOptionalOpsDenied(dialer, "capability deny");
        }
        finally
        {
            dialer.Close();
            responder.Close();
            LoopbackAssert.AssertEqual("Closed", dialer.State(), "capability-deny dialer state after close");
            LoopbackAssert.AssertEqual("Closed", responder.State(), "capability-deny responder state after close");
        }
    }

    private static void AssertOptionalOpsDenied(RemoteAdapterFfi dialer, string what)
    {
        var project = AssertRejected(() => dialer.Project(ProjectRequestJson()));
        LoopbackAssert.AssertEqual("CAPABILITY_PORT_MISSING", project.code, $"{what}: project deny code");
        LoopbackAssert.AssertEqual("op_unsupported", project.wireCode, $"{what}: project deny wire_code");

        var compute = AssertRejected(() => dialer.Compute(ComputeRequestJson()));
        LoopbackAssert.AssertEqual("CAPABILITY_PORT_MISSING", compute.code, $"{what}: compute deny code");
        LoopbackAssert.AssertEqual("op_unsupported", compute.wireCode, $"{what}: compute deny wire_code");

        var fork = AssertRejected(() => dialer.ListForkTimelineEvents(ForkScopeJson()));
        LoopbackAssert.AssertEqual("CAPABILITY_PORT_MISSING", fork.code, $"{what}: fork deny code");
        LoopbackAssert.AssertEqual("op_unsupported", fork.wireCode, $"{what}: fork deny wire_code");
    }

    private static (ConnectResponderFfi Responder, RemoteAdapterFfi Dialer) DialPair(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture, PortsHandler? ports)
    {
        var pair = SpokeConnectMethods.LoopbackTransportPair();
        // The accept-side constructor returns immediately in `Handshaking`
        // (D16): the dialer hello is the sync point, so the smoke polls
        // `state()` (bounded) to `Established` before invoking.
        var responder = SpokeConnectMethods.ConnectResponderFfi(
            new LoopbackCallbackTransport(pair.Server()),
            seedHost,
            PortsManifestJson("test-responder"),
            [fixture.peer_id_client],
            new Dictionary<string, byte[]> { [fixture.peer_id_client] = pubkeyClient },
            ports,
            null);
        var dialer = SpokeConnectMethods.ConnectRemoteAdapterFfi(
            new LoopbackCallbackTransport(pair.Client()),
            seedClient,
            PortsManifestJson("test-client"),
            pubkeyHost,
            [fixture.peer_id_host],
            null);
        return (responder, dialer);
    }

    /// <summary>Ports-carrying manifest — baseline + optional families, so
    /// the negotiated set includes l2-computable / l5-fork. Mirror of the
    /// Rust `ports_manifest_json` test helper.</summary>
    private static string PortsManifestJson(string hostId)
    {
        var manifest = new Dictionary<string, object?>
        {
            ["schema_version"] = 1,
            ["host_id"] = hostId,
            ["roles"] = new[] { "data-store", "l2-computable" },
            ["capabilities"] = new[] { "spoke-baseline", "l2-computable", "l5-fork" },
            ["namespaces"] = new[] { "toy_world" },
            ["extensions"] = new Dictionary<string, object?>(),
        };
        return JsonSerializer.Serialize(manifest);
    }

    /// <summary>Tool-carrying manifest (baseline + tools only — no optional
    /// families) for the capability-deny session. Mirror of the Rust
    /// `tool_manifest` test helper.</summary>
    private static string ToolManifestJson(string hostId)
    {
        var manifest = new Dictionary<string, object?>
        {
            ["schema_version"] = 1,
            ["host_id"] = hostId,
            ["roles"] = new[] { "data-store" },
            ["capabilities"] = new[] { "spoke-baseline", "tools.math.add" },
            ["namespaces"] = new[] { "math", "toy_world" },
            ["extensions"] = new Dictionary<string, object?>(),
            ["tools"] = new[]
            {
                new Dictionary<string, object?>
                {
                    ["schema_version"] = 1,
                    ["capability_id"] = "tools.math.add",
                    ["op"] = "tools.math.add",
                    ["description"] = "Add two integers",
                    ["input"] = new Dictionary<string, object?> { ["type"] = "object" },
                    ["output"] = new Dictionary<string, object?> { ["type"] = "object" },
                },
            },
        };
        return JsonSerializer.Serialize(manifest);
    }

    private static string KnowledgeEntryJson(string entryId, string canonicalName)
    {
        var entry = new Dictionary<string, object?>
        {
            ["schema_version"] = 1,
            ["entry_id"] = entryId,
            ["entry_type"] = "knowledge",
            ["canonical_name"] = canonicalName,
            ["status"] = "active",
            ["body"] = new Dictionary<string, object?> { ["summary"] = "served through the foreign ports callback" },
            ["extensions"] = new Dictionary<string, object?>(),
        };
        return JsonSerializer.Serialize(entry);
    }

    private static string ProjectRequestJson() =>
        """{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}""";

    private static string ComputeRequestJson() =>
        """{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}""";

    private static string ForkScopeJson() =>
        """{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}""";

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

    private static void AssertComputableState(JsonElement computable, string what)
    {
        LoopbackAssert.AssertEqual(2.5, computable.GetProperty("tide_level").GetDouble(), $"{what}: tide_level");
        LoopbackAssert.AssertEqual(37, computable.GetProperty("cargo_tons").GetInt32(), $"{what}: cargo_tons");
    }

    private static FfiException.Rejected AssertRejected(Func<string> invoke)
    {
        try
        {
            invoke();
            throw new Exception("expected FfiException.Rejected, got success");
        }
        catch (FfiException.Rejected rejected)
        {
            return rejected;
        }
        catch (FfiException other)
        {
            throw new Exception($"expected FfiException.Rejected, got {other.GetType().Name}");
        }
    }

    /// <summary>Foreign-callback ports handler: in-memory knowledge store
    /// plus canned optional-family answers; unknown entries reject with an
    /// application `Rejected` (ordinary deny — not containment);
    /// `kb_ffi_ports_boom` faults (the containment row). Mirror of the Rust
    /// `TestPortsHandler`.</summary>
    private sealed class SmokePortsHandler : PortsHandler
    {
        private readonly Dictionary<string, JsonElement> _entries = new();

        public string GetKnowledgeEntry(string entryId)
        {
            if (entryId == "kb_ffi_ports_boom")
            {
                throw new InvalidOperationException("foreign ports handler fault (containment row)");
            }
            if (_entries.TryGetValue(entryId, out var entry))
            {
                return entry.GetRawText();
            }
            throw new FfiException.Rejected(
                "KNOWLEDGE_ENTRY_NOT_FOUND", $"entry {entryId} not found", "store_miss", null);
        }

        public string PutKnowledgeEntry(string entryJson, ulong? expectedBaseRevision)
        {
            using var doc = JsonDocument.Parse(entryJson);
            var entryId = doc.RootElement.GetProperty("entry_id").GetString()!;
            _entries[entryId] = doc.RootElement.Clone();
            return entryJson;
        }

        public string GetRelation(string relationId) => throw new FfiException.Rejected(
            "INVALID_INPUT", "relation serving not exercised by this test handler", null, null);

        public string PutRelation(string relationJson, ulong? expectedBaseRevision) => throw new FfiException.Rejected(
            "INVALID_INPUT", "relation serving not exercised by this test handler", null, null);

        public string ListKnowledgeEntries(string scopeJson)
        {
            using var stream = new MemoryStream();
            using (var writer = new Utf8JsonWriter(stream))
            {
                writer.WriteStartArray();
                foreach (var entry in _entries.Values)
                {
                    entry.WriteTo(writer);
                }
                writer.WriteEndArray();
            }
            return System.Text.Encoding.UTF8.GetString(stream.ToArray());
        }

        public string ListTimelineEvents(string scopeJson) => "[]";

        public string PutFindings(string findingsJson) => "[]";

        public string ListRules(string[] ruleRefs) => "[]";

        public string ListPeerHostCapabilityManifests() => "[]";

        public string Project(string projectRequestJson)
        {
            using var doc = JsonDocument.Parse(projectRequestJson);
            var root = doc.RootElement;
            var response = new Dictionary<string, object?>
            {
                ["session_id"] = root.GetProperty("session_id").GetString(),
                ["entry_id"] = root.GetProperty("entry_id").GetString(),
                ["computable"] = new Dictionary<string, object?> { ["tide_level"] = 2.4, ["cargo_tons"] = 38 },
            };
            return JsonSerializer.Serialize(response);
        }

        public string Compute(string computeRequestJson)
        {
            using var doc = JsonDocument.Parse(computeRequestJson);
            var root = doc.RootElement;
            var computable = root.GetProperty("computable");
            var response = new Dictionary<string, object?>
            {
                ["session_id"] = root.GetProperty("session_id").GetString(),
                ["entry_id"] = root.GetProperty("entry_id").GetString(),
                ["computable"] = JsonDocument.Parse(computable.GetRawText()).RootElement.Clone(),
                ["state"] = JsonDocument.Parse(computable.GetRawText()).RootElement.Clone(),
            };
            return JsonSerializer.Serialize(response);
        }

        public string ListForkTimelineEvents(string scopeJson)
        {
            using var doc = JsonDocument.Parse(scopeJson);
            var forkId = doc.RootElement.GetProperty("fork_id").GetString();
            if (forkId != "fork_tw_ffi_events")
            {
                return "[]";
            }
            var events = new[]
            {
                new Dictionary<string, object?>
                {
                    ["schema_version"] = 1,
                    ["timeline_event_id"] = "evt_tw_ffi_storm",
                    ["canonical_name"] = "FFI Fork Storm",
                    ["fork_id"] = "fork_tw_ffi_events",
                    ["extensions"] = new Dictionary<string, object?>(),
                },
            };
            return JsonSerializer.Serialize(events);
        }
    }
}
