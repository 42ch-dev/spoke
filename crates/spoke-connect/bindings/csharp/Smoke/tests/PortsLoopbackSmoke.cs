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
///
/// The KE remote cases mirror the Rust `ke_remote_ffi_tests` battery: the
/// `extract` service face round-trips a provisional batch, its three
/// unavailability rows stay distinct (unnegotiated flag / absent ports probe /
/// callback application refusal), and the existing Scope query carries the
/// `ke-ownership` gate (OQ-FFI-1). The host-local loader value never crosses
/// the wire: a loader-only canary is checked absent against the recorded
/// envelopes, and no loader callback is exported.
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
        RunKeRemote(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
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

        /// <summary>This double serves only the optional `port.*` families, so
        /// `extract` is an ordinary application refusal (never the
        /// absent-provider probe deny). The regenerated
        /// <see cref="PortsHandler"/> interface requires the method.</summary>
        public string Extract(string extractRequestJson) => throw new FfiException.Rejected(
            "CAPABILITY_PORT_MISSING", "this ports smoke double does not serve extraction", null, null);
    }

    // ── KE remote (F1/F2/F3) ────────────────────────────────────────────────
    //
    // The `extract` service face and the `ke-ownership` gate on the existing
    // Scope query, mirroring the Rust `ke_remote_ffi_tests` battery
    // (`crates/spoke-connect/src/ffi.rs`). The generated `PortsHandler`
    // interface carries no loader method — source loading is the serving
    // host's own business — so the loader value stays a private handler field
    // and the canary below exists nowhere else: not in the request, not in the
    // expected response. It is asserted absent from the recorded envelopes.

    private const string KeLoaderCanary = "ke-ffi-loader-canary-csharp";

    /// <summary>Both peers' hello manifests: the baseline capability plus
    /// whatever the scenario negotiates. A capability must appear in *both*
    /// hellos to be negotiated, so the omitted side is how the deny scenarios
    /// are set up. Mirror of the Rust `ke_manifest_json` test helper.</summary>
    private static string KeManifestJson(string hostId, string[] capabilities)
    {
        var all = new List<string> { "spoke-baseline" };
        all.AddRange(capabilities);
        var manifest = new Dictionary<string, object?>
        {
            ["schema_version"] = 1,
            ["host_id"] = hostId,
            ["roles"] = new[] { "data-store" },
            ["capabilities"] = all,
            ["namespaces"] = new[] { "toy_world" },
            ["extensions"] = new Dictionary<string, object?>(),
        };
        return JsonSerializer.Serialize(manifest);
    }

    /// <summary>An `extract` request for the scenario's run id: the payload is
    /// the `ExtractRequest` itself and `sources` carry references only.</summary>
    private static string KeExtractRequestJson(string runId) =>
        $$"""{"run_id":"{{runId}}","sources":[{"schema_version":1,"source_id":"manuscript/ch1","extensions":{}},{"schema_version":1,"source_id":"manuscript/ch2","extensions":{}}]}""";

    /// <summary>A Scope carrying a reader viewpoint plus opaque extension values
    /// the callback must receive unchanged.</summary>
    private static string KeViewpointScopeJson() =>
        """{"scope_id":"toy-scope-001","viewpoint":"kb_tw_mira","entry_types":["note"],"extensions":{"product":{"viewpoint":"decoy","owner":"someone"}}}""";

    /// <summary>Loopback pair through both FFI faces with the scenario's
    /// negotiated capabilities and optional foreign <see cref="PortsHandler"/>.
    /// Mirror of the Rust `dial_ke_remote` test helper.</summary>
    private static (ConnectResponderFfi Responder, RemoteAdapterFfi Dialer, RecordingLoopbackTransport Transport) DialKePair(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture,
        string[] clientCapabilities, string[] responderCapabilities, PortsHandler? ports)
    {
        var pair = SpokeConnectMethods.LoopbackTransportPair();
        var responder = SpokeConnectMethods.ConnectResponderFfi(
            new LoopbackCallbackTransport(pair.Server()),
            seedHost,
            KeManifestJson("test-responder", responderCapabilities),
            [fixture.peer_id_client],
            new Dictionary<string, byte[]> { [fixture.peer_id_client] = pubkeyClient },
            ports,
            null);
        var transport = new RecordingLoopbackTransport(pair.Client());
        var dialer = SpokeConnectMethods.ConnectRemoteAdapterFfi(
            transport,
            seedClient,
            KeManifestJson("test-client", clientCapabilities),
            pubkeyHost,
            [fixture.peer_id_host],
            null);
        LoopbackAssert.AssertEqual("Established", dialer.State(), "ke dialer state");
        WaitForState("ke responder handshake", () => responder.State(), "Established");
        return (responder, dialer, transport);
    }

    /// <summary>The unavailable-capability row shared by the unnegotiated and
    /// absent-provider denials (the callback-refusal row differs in
    /// `wireCode` and is asserted at its own case).</summary>
    private static FfiException.Rejected AssertUnavailableCapability(Func<string> invoke, string wireCode, string what)
    {
        var rejected = AssertRejected(invoke);
        LoopbackAssert.AssertEqual("CAPABILITY_PORT_MISSING", rejected.code, $"{what}: deny code");
        LoopbackAssert.AssertEqual(null, rejected.kind, $"{what}: deny kind");
        LoopbackAssert.AssertEqual(wireCode, rejected.wireCode, $"{what}: deny wire_code");
        return rejected;
    }

    /// <summary>The frozen KE remote matrix over both FFI faces.</summary>
    private static void RunKeRemote(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        RunKeExtractRoundTrip(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunKeExtractMalformedRequest(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunKeExtractUnnegotiated(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, "responder");
        RunKeExtractUnnegotiated(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, "dialer");
        RunKeExtractAbsentPorts(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunKeExtractCallbackRefusal(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunKeOwnershipAllow(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
        RunKeOwnershipDeny(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, "dialer");
        RunKeOwnershipDeny(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, "responder");
        RunKeLoaderCanary(seedClient, seedHost, pubkeyHost, pubkeyClient, fixture);
    }

    /// <summary>Extract round-trip through the foreign callback: the request is
    /// the `ExtractRequest` itself and the response is the assembled batch.</summary>
    private static void RunKeExtractRoundTrip(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var handler = new KeRemotePortsHandler();
        var (responder, dialer, _) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            ["ke-extraction"], ["ke-extraction"], handler);
        try
        {
            using var responseDoc = JsonDocument.Parse(dialer.Extract(KeExtractRequestJson("run-ffi-ke-1")));
            var response = responseDoc.RootElement;

            // Observable, deterministic response behaviour: one provisional
            // candidate per declared source reference, carrying the run id.
            LoopbackAssert.AssertEqual(
                "run-ffi-ke-1", response.GetProperty("run").GetProperty("run_id").GetString(), "ke extract run_id");
            var candidates = response.GetProperty("candidates");
            LoopbackAssert.AssertEqual(2, candidates.GetArrayLength(), "ke extract candidates (one per source)");
            var wantEntryIds = new[] { "ke-ffi-run-ffi-ke-1-0", "ke-ffi-run-ffi-ke-1-1" };
            for (var index = 0; index < wantEntryIds.Length; index++)
            {
                LoopbackAssert.AssertEqual(
                    wantEntryIds[index], candidates[index].GetProperty("entry_id").GetString(),
                    $"ke extract candidate {index} entry_id");
                LoopbackAssert.AssertEqual(
                    "provisional", candidates[index].GetProperty("status").GetString(),
                    $"ke extract candidate {index} status");
                LoopbackAssert.AssertEqual(
                    "note", candidates[index].GetProperty("entry_type").GetString(),
                    $"ke extract candidate {index} entry_type");
            }

            // The callback received the `ExtractRequest` itself: no wrapper and
            // no loader value — only references travelled.
            var requests = handler.ExtractRequests();
            LoopbackAssert.AssertEqual(1, requests.Count, "ke extract callback reached exactly once");
            var request = requests[0];
            LoopbackAssert.AssertEqual("run-ffi-ke-1", request.GetProperty("run_id").GetString(), "ke extract callback run_id");
            LoopbackAssert.AssertEqual(2, request.GetProperty("sources").GetArrayLength(), "ke extract callback sources");
            LoopbackAssert.AssertEqual(
                "manuscript/ch1", request.GetProperty("sources")[0].GetProperty("source_id").GetString(),
                "ke extract callback source reference");
            foreach (var wrapper in new[] { "request", "arguments", "input", "loaded_input" })
            {
                LoopbackAssert.AssertEqual(
                    false, request.TryGetProperty(wrapper, out _),
                    $"ke extract callback request carries no {wrapper} wrapper");
            }
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>Malformed request JSON: rejected locally with zero wire
    /// traffic.</summary>
    private static void RunKeExtractMalformedRequest(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var handler = new KeRemotePortsHandler();
        var (responder, dialer, transport) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            ["ke-extraction"], ["ke-extraction"], handler);
        try
        {
            var framesBefore = transport.Frames().Count;
            var malformed = AssertRejected(() => dialer.Extract("{ not json"));
            LoopbackAssert.AssertEqual("INVALID_INPUT", malformed.code, "ke malformed extract json code");
            LoopbackAssert.AssertEqual(null, malformed.kind, "ke malformed extract json kind");
            LoopbackAssert.AssertEqual(null, malformed.wireCode, "ke malformed extract json wire_code");
            LoopbackAssert.AssertEqual(0, handler.ExtractRequests().Count, "ke malformed extract never reaches the callback");
            LoopbackAssert.AssertEqual(
                framesBefore, transport.Frames().Count, "ke malformed extract never reaches the wire");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>Unnegotiated `ke-extraction`: otherwise identical manifests with
    /// one flag omitted, in both directions.</summary>
    private static void RunKeExtractUnnegotiated(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture, string omittedSide)
    {
        var handler = new KeRemotePortsHandler();
        string[] clientCapabilities = omittedSide == "dialer" ? Array.Empty<string>() : new[] { "ke-extraction" };
        string[] responderCapabilities = omittedSide == "responder" ? Array.Empty<string>() : new[] { "ke-extraction" };
        var (responder, dialer, _) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            clientCapabilities, responderCapabilities, handler);
        try
        {
            var what = $"ke extract unnegotiated ({omittedSide} omits the flag)";
            AssertUnavailableCapability(
                () => dialer.Extract(KeExtractRequestJson("run-ffi-unnegotiated")), "op_unsupported", what);
            LoopbackAssert.AssertEqual(0, handler.ExtractRequests().Count, $"{what}: gate denies before the callback");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>Negotiated in both hellos, no ports face at all: the
    /// absent-provider probe deny.</summary>
    private static void RunKeExtractAbsentPorts(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var (responder, dialer, _) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            ["ke-extraction"], ["ke-extraction"], null);
        try
        {
            AssertUnavailableCapability(
                () => dialer.Extract(KeExtractRequestJson("run-ffi-absent-ports")),
                "op_unsupported",
                "ke extract absent-ports probe deny");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>Negotiated, callback declines: an ordinary application refusal,
    /// not a missing-method probe deny.</summary>
    private static void RunKeExtractCallbackRefusal(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var handler = new KeRemotePortsHandler(declineExtract: true);
        var (responder, dialer, _) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            ["ke-extraction"], ["ke-extraction"], handler);
        try
        {
            var refused = AssertRejected(() => dialer.Extract(KeExtractRequestJson("run-ffi-refusal")));
            LoopbackAssert.AssertEqual("CAPABILITY_PORT_MISSING", refused.code, "ke extract refusal code");
            LoopbackAssert.AssertEqual(null, refused.kind, "ke extract refusal kind");
            LoopbackAssert.AssertEqual(null, refused.wireCode, "ke extract refusal wire_code (not a probe deny)");
            LoopbackAssert.AssertEqual(
                true, refused.message.Contains("does not serve extraction"), "ke extract refusal message survives");
            LoopbackAssert.AssertEqual(
                1, handler.ExtractRequests().Count, "a refusal is the callback's own answer, not a probe deny");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>OQ-FFI-1 allow: the existing Scope method carries the ownership
    /// gate — no ownership-specific method or callback is added.</summary>
    private static void RunKeOwnershipAllow(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var handler = new KeRemotePortsHandler();
        var (responder, dialer, _) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            ["ke-ownership"], ["ke-ownership"], handler);
        try
        {
            using var servedDoc = JsonDocument.Parse(dialer.ListKnowledgeEntries(KeViewpointScopeJson()));
            var served = servedDoc.RootElement;
            LoopbackAssert.AssertEqual(2, served.GetArrayLength(), "ke ownership allow serves exactly two entries");
            LoopbackAssert.AssertEqual(
                "kb_ffi_ke_own", served[0].GetProperty("entry_id").GetString(), "ke ownership allow own-private entry");
            LoopbackAssert.AssertEqual(
                "FFI KE Own", served[0].GetProperty("canonical_name").GetString(), "ke ownership allow canonical_name");
            LoopbackAssert.AssertEqual(
                "kb_ffi_ke_shared", served[1].GetProperty("entry_id").GetString(), "ke ownership allow shared entry");

            // The callback received the declared Scope unchanged: the viewpoint
            // and the opaque extension values are not stripped to make the
            // request succeed, and no extraction call was involved.
            var scopes = handler.ScopeCalls();
            LoopbackAssert.AssertEqual(1, scopes.Count, "ke ownership allow reaches the callback exactly once");
            var scope = scopes[0];
            LoopbackAssert.AssertEqual("kb_tw_mira", scope.GetProperty("viewpoint").GetString(), "ke ownership allow viewpoint");
            LoopbackAssert.AssertEqual(
                "note", scope.GetProperty("entry_types")[0].GetString(), "ke ownership allow entry_types");
            var product = scope.GetProperty("extensions").GetProperty("product");
            LoopbackAssert.AssertEqual("decoy", product.GetProperty("viewpoint").GetString(), "ke ownership allow decoy viewpoint");
            LoopbackAssert.AssertEqual("someone", product.GetProperty("owner").GetString(), "ke ownership allow opaque owner");
            LoopbackAssert.AssertEqual(0, handler.ExtractRequests().Count, "ke ownership allow never invokes extract");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>OQ-FFI-1 deny: either hello omitting `ke-ownership` refuses
    /// before the callback runs.</summary>
    private static void RunKeOwnershipDeny(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture, string omittedSide)
    {
        var handler = new KeRemotePortsHandler();
        string[] clientCapabilities = omittedSide == "dialer" ? Array.Empty<string>() : new[] { "ke-ownership" };
        string[] responderCapabilities = omittedSide == "responder" ? Array.Empty<string>() : new[] { "ke-ownership" };
        var (responder, dialer, _) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            clientCapabilities, responderCapabilities, handler);
        try
        {
            var what = $"ke ownership deny ({omittedSide} omits the flag)";
            AssertUnavailableCapability(
                () => dialer.ListKnowledgeEntries(KeViewpointScopeJson()), "op_unsupported", what);
            LoopbackAssert.AssertEqual(0, handler.ScopeCalls().Count, $"{what}: refuses before the callback");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>The host-local loader value never crosses the wire.</summary>
    private static void RunKeLoaderCanary(
        byte[] seedClient, byte[] seedHost, byte[] pubkeyHost, byte[] pubkeyClient, LoopbackFixture fixture)
    {
        var handler = new KeRemotePortsHandler();
        var (responder, dialer, transport) = DialKePair(
            seedClient, seedHost, pubkeyHost, pubkeyClient, fixture,
            ["ke-extraction"], ["ke-extraction"], handler);
        try
        {
            var response = dialer.Extract(KeExtractRequestJson("run-ffi-canary"));
            LoopbackAssert.AssertEqual(true, response.Contains("run-ffi-canary"), "ke canary round-trip really happened");
            LoopbackAssert.AssertEqual(
                false, response.Contains(KeLoaderCanary), "ke canary is not serialized into the response");
            var frames = transport.Frames();
            LoopbackAssert.AssertEqual(true, frames.Count > 0, "ke canary row records the envelopes");
            var recorded = string.Concat(frames.ConvertAll(frame => System.Text.Encoding.UTF8.GetString(frame)));
            // Positive control: the recording really observes the plaintext
            // wire (the invoke request's run id is visible), so the canary
            // absence below is a real observation, not a vacuous one.
            LoopbackAssert.AssertEqual(
                true, recorded.Contains("run-ffi-canary"), "ke canary positive control: the request is on the wire");
            LoopbackAssert.AssertEqual(
                false, recorded.Contains(KeLoaderCanary), "ke loader canary never reaches the wire");
        }
        finally
        {
            dialer.Close();
            responder.Close();
        }
    }

    /// <summary>Dialer-side transport that records every envelope in both
    /// directions, so the smoke can assert what did and did not reach the
    /// wire.</summary>
    private sealed class RecordingLoopbackTransport : Transport
    {
        private readonly LoopbackTransport _inner;
        private readonly object _lock = new();
        private readonly List<byte[]> _frames = new();

        public RecordingLoopbackTransport(LoopbackTransport inner) => _inner = inner;

        public List<byte[]> Frames()
        {
            lock (_lock)
            {
                return new List<byte[]>(_frames);
            }
        }

        public void Send(byte[] envelope)
        {
            lock (_lock)
            {
                _frames.Add(envelope);
            }
            _inner.Send(envelope);
        }

        public byte[] Recv()
        {
            var envelope = _inner.Recv();
            lock (_lock)
            {
                _frames.Add(envelope);
            }
            return envelope;
        }

        public void Close() => _inner.Close();
    }

    /// <summary>Foreign-callback ports handler for the KE remote matrix: a
    /// private host-local loader value, the `extract` service face, and a
    /// viewpoint-filtered knowledge store. Mirror of the Rust
    /// `KeRemotePortsHandler` double.</summary>
    private sealed class KeRemotePortsHandler : PortsHandler
    {
        /// <summary>One served KnowledgeEntry recipe; `visibility` is the host's
        /// own filtering input, not a wire contract. Ordered by entry id so the
        /// served batch is deterministic.</summary>
        private static readonly (string EntryId, string CanonicalName, string Visibility)[] Catalog =
        [
            ("kb_ffi_ke_foreign", "FFI KE Foreign", "private:kb_tw_other"),
            ("kb_ffi_ke_own", "FFI KE Own", "private:kb_tw_mira"),
            ("kb_ffi_ke_shared", "FFI KE Shared", "shared"),
        ];

        private readonly object _lock = new();
        private readonly bool _declineExtract;
        private readonly List<JsonElement> _extractRequests = new();
        private readonly List<JsonElement> _scopeCalls = new();
        private readonly string _loadedCanary = KeLoaderCanary;

        public KeRemotePortsHandler(bool declineExtract = false) => _declineExtract = declineExtract;

        public List<JsonElement> ExtractRequests()
        {
            lock (_lock)
            {
                return new List<JsonElement>(_extractRequests);
            }
        }

        public List<JsonElement> ScopeCalls()
        {
            lock (_lock)
            {
                return new List<JsonElement>(_scopeCalls);
            }
        }

        public string Extract(string extractRequestJson)
        {
            using var doc = JsonDocument.Parse(extractRequestJson);
            var request = doc.RootElement;
            lock (_lock)
            {
                _extractRequests.Add(request.Clone());
            }
            if (_declineExtract)
            {
                throw new FfiException.Rejected(
                    "CAPABILITY_PORT_MISSING", "this binding does not serve extraction", null, null);
            }
            // The loader runs inside the host service: its value feeds the
            // extractor here and is never a transport argument or callback value.
            if (string.IsNullOrEmpty(_loadedCanary))
            {
                throw new Exception("host loader value is missing");
            }
            var runId = request.GetProperty("run_id").GetString()!;
            var sourceCount = request.GetProperty("sources").GetArrayLength();
            var candidates = new List<Dictionary<string, object?>>();
            for (var index = 0; index < sourceCount; index++)
            {
                candidates.Add(new Dictionary<string, object?>
                {
                    ["schema_version"] = 1,
                    ["entry_id"] = $"ke-ffi-{runId}-{index}",
                    ["entry_type"] = "note",
                    ["canonical_name"] = $"Extracted note {index}",
                    ["status"] = "provisional",
                    ["body"] = new Dictionary<string, object?> { ["summary"] = $"provisional candidate {index}" },
                    ["extensions"] = new Dictionary<string, object?>(),
                });
            }
            return JsonSerializer.Serialize(new Dictionary<string, object?>
            {
                ["candidates"] = candidates,
                ["run"] = new Dictionary<string, object?> { ["run_id"] = runId },
            });
        }

        public string ListKnowledgeEntries(string scopeJson)
        {
            using var doc = JsonDocument.Parse(scopeJson);
            var scope = doc.RootElement;
            lock (_lock)
            {
                _scopeCalls.Add(scope.Clone());
            }
            var viewpoint = scope.TryGetProperty("viewpoint", out var viewpointElement)
                ? viewpointElement.GetString()
                : null;
            var served = new List<Dictionary<string, object?>>();
            foreach (var entry in Catalog)
            {
                if (entry.Visibility != "shared" && entry.Visibility != $"private:{viewpoint}")
                {
                    continue;
                }
                served.Add(KeEntryJson(entry.EntryId, entry.CanonicalName, entry.Visibility));
            }
            return JsonSerializer.Serialize(served);
        }

        /// <summary>One served KnowledgeEntry; `visibility` rides the opaque
        /// extensions and is the host's own filtering input.</summary>
        private static Dictionary<string, object?> KeEntryJson(string entryId, string canonicalName, string visibility) =>
            new()
            {
                ["schema_version"] = 1,
                ["entry_id"] = entryId,
                ["entry_type"] = "knowledge",
                ["canonical_name"] = canonicalName,
                ["status"] = "active",
                ["body"] = new Dictionary<string, object?> { ["summary"] = "served through the foreign ports callback" },
                ["extensions"] = new Dictionary<string, object?>
                {
                    ["visibility"] = new Dictionary<string, object?> { ["scope"] = visibility },
                },
            };

        /// <summary>Catalogue ops this double does not serve: an ordinary
        /// application reject (`wireCode` null), never containment.</summary>
        private static FfiException Unserved(string op) => new FfiException.Rejected(
            "INVALID_INPUT", $"{op} is not served by this test handler", null, null);

        public string GetKnowledgeEntry(string entryId) => throw Unserved("get_knowledge_entry");

        public string PutKnowledgeEntry(string entryJson, ulong? expectedBaseRevision) =>
            throw Unserved("put_knowledge_entry");

        public string GetRelation(string relationId) => throw Unserved("get_relation");

        public string PutRelation(string relationJson, ulong? expectedBaseRevision) =>
            throw Unserved("put_relation");

        public string ListTimelineEvents(string scopeJson) => throw Unserved("list_timeline_events");

        public string PutFindings(string findingsJson) => throw Unserved("put_findings");

        public string ListRules(string[] ruleRefs) => throw Unserved("list_rules");

        public string ListPeerHostCapabilityManifests() => throw Unserved("list_peer_host_capability_manifests");

        public string Project(string projectRequestJson) => throw Unserved("project");

        public string Compute(string computeRequestJson) => throw Unserved("compute");

        public string ListForkTimelineEvents(string scopeJson) => throw Unserved("list_fork_timeline_events");
    }
}
