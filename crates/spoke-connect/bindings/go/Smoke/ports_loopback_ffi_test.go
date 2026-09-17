package smoke_test

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"testing"

	sc "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go/generated/spoke_connect"
)

// Optional-port dialer ops + responder ports serving over the loopback pair
// (D16), run in the DEFAULT go test suite against the committed production
// binding + native (no smoke host required): the responder serves baseline +
// optional port.* families through a foreign PortsHandler (user lock), the
// dialer drives Project / Compute / ListForkTimelineEvents, and the error
// rows — capability-gate deny, absent-ports fail-closed deny, and
// foreign-fault containment with serve-loop survival — mirror the Rust
// connect_responder_ffi_tests battery. The router is untouched: optional ops
// ride the per-peer RemoteAdapterFfi.
//
// The KE remote subtests mirror the Rust `ke_remote_ffi_tests` battery: the
// `extract` service face round-trips a provisional batch, its three
// unavailability rows stay distinct (unnegotiated flag / absent ports probe /
// callback application refusal), and the existing Scope query carries the
// `ke-ownership` gate (OQ-FFI-1). The host-local loader value never crosses
// the wire: a loader-only canary is checked absent against the recorded
// envelopes, and no loader callback is exported.

func TestPortsLoopbackFfiPair(t *testing.T) {
	fixture := loadLoopbackFixture(t)
	seedClient := decodeLoopbackHex(t, fixture.SeedClientHex)
	seedHost := decodeLoopbackHex(t, fixture.SeedHostHex)
	pubkeyHost := decodeLoopbackHex(t, fixture.PubkeyHostHex)
	pubkeyClient := decodeLoopbackHex(t, fixture.PubkeyClientHex)

	t.Run("serves baseline and optional families through a foreign PortsHandler", func(t *testing.T) {
		handler := &smokePortsHandler{entries: map[string]json.RawMessage{}}
		var ports sc.PortsHandler = handler
		responder, dialer := dialPortsPair(t, seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, &ports)

		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		if got := dialer.State(); got != "Established" {
			t.Fatalf("ports dialer state: got %q want Established", got)
		}
		waitForState(t, "ports responder handshake", responder.State, "Established")

		// 1. Baseline round-trip through the foreign ports handler: put
		//    stores the entry JSON in the handler, get serves it back. The
		//    wire carries the canonicalized entry JSON (typed round-trip), so
		//    compare semantically, not byte-wise.
		entryJSON := `{"schema_version":1,"entry_id":"kb_ffi_ports_put","entry_type":"knowledge","canonical_name":"FFI Ports Put","status":"active","body":{"summary":"served through the foreign ports callback"},"extensions":{}}`
		putJSON, err := dialer.PutKnowledgeEntry(entryJSON, nil)
		if err != nil {
			t.Fatalf("put through the foreign ports handler: %v", err)
		}
		if got := jsonField(t, putJSON, "entry_id"); got != "kb_ffi_ports_put" {
			t.Fatalf("put through the foreign ports handler: entry_id got %q", got)
		}
		getJSON, err := dialer.GetKnowledgeEntry("kb_ffi_ports_put")
		if err != nil {
			t.Fatalf("get through the foreign ports handler: %v", err)
		}
		if got := jsonField(t, getJSON, "canonical_name"); got != "FFI Ports Put" {
			t.Fatalf("get through the foreign ports handler: canonical_name got %q", got)
		}

		// 2. Application-reject passthrough: an unknown entry rejects with
		//    the handler's locked code + re-hung kind (ordinary deny, NOT
		//    containment).
		_, err = dialer.GetKnowledgeEntry("kb_ffi_ports_missing")
		var missing *sc.FfiErrorRejected
		if !errors.As(err, &missing) {
			t.Fatalf("expected FfiErrorRejected for unknown entry, got %v", err)
		}
		if missing.Code != "KNOWLEDGE_ENTRY_NOT_FOUND" {
			t.Fatalf("unknown entry reject code: got %q want KNOWLEDGE_ENTRY_NOT_FOUND", missing.Code)
		}
		if missing.Kind == nil || *missing.Kind != "store_miss" {
			t.Fatalf("unknown entry reject kind: got %v want store_miss (re-hung)", missing.Kind)
		}
		if missing.WireCode != nil {
			t.Fatalf("unknown entry reject wire_code: got %v want nil", *missing.WireCode)
		}

		// 3. Optional dialer ops round-trip through the callback
		//    (l2-computable / l5-fork negotiated by both manifests).
		projectJSON, err := dialer.Project(`{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}`)
		if err != nil {
			t.Fatalf("project through the foreign ports handler: %v", err)
		}
		if got := jsonField(t, projectJSON, "session_id"); got != "sess_ffi_ports" {
			t.Fatalf("project session_id: got %q", got)
		}
		if got := jsonField(t, projectJSON, "entry_id"); got != "kb_ffi_ports_proj" {
			t.Fatalf("project entry_id: got %q", got)
		}
		if got := jsonField(t, projectJSON, "computable.tide_level"); got != "2.4" {
			t.Fatalf("project computable tide_level: got %q want 2.4", got)
		}
		if got := jsonField(t, projectJSON, "computable.cargo_tons"); got != "38" {
			t.Fatalf("project computable cargo_tons: got %q want 38", got)
		}

		computeJSON, err := dialer.Compute(`{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}`)
		if err != nil {
			t.Fatalf("compute through the foreign ports handler: %v", err)
		}
		for _, field := range []string{"computable", "state"} {
			if got := jsonField(t, computeJSON, field+".tide_level"); got != "2.5" {
				t.Fatalf("compute %s tide_level: got %q want 2.5", field, got)
			}
			if got := jsonField(t, computeJSON, field+".cargo_tons"); got != "37" {
				t.Fatalf("compute %s cargo_tons: got %q want 37", field, got)
			}
		}

		eventsJSON, err := dialer.ListForkTimelineEvents(`{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}`)
		if err != nil {
			t.Fatalf("fork round-trip through the foreign ports handler: %v", err)
		}
		if got := jsonField(t, eventsJSON, "0.timeline_event_id"); got != "evt_tw_ffi_storm" {
			t.Fatalf("fork event id: got %q want evt_tw_ffi_storm", got)
		}
		if got := jsonField(t, eventsJSON, "0.fork_id"); got != "fork_tw_ffi_events" {
			t.Fatalf("fork event fork_id: got %q", got)
		}

		// 4. Malformed JSON is rejected locally (INVALID_INPUT, zero wire
		//    traffic) — the dialer pre-validation row per op.
		_, err = dialer.Project(`{ not json`)
		var invalid *sc.FfiErrorRejected
		if !errors.As(err, &invalid) {
			t.Fatalf("expected FfiErrorRejected for malformed project json, got %v", err)
		}
		if invalid.Code != "INVALID_INPUT" {
			t.Fatalf("malformed project json code: got %q want INVALID_INPUT", invalid.Code)
		}
		if invalid.WireCode != nil {
			t.Fatalf("malformed project json wire_code: got %v want nil", *invalid.WireCode)
		}

		// 5. Foreign-fault containment: the handler faults on
		//    kb_ffi_ports_boom -> INTERNAL_ERROR with no details; the session
		//    survives and the serve loop answers the next healthy put.
		_, err = dialer.GetKnowledgeEntry("kb_ffi_ports_boom")
		var contained *sc.FfiErrorRejected
		if !errors.As(err, &contained) {
			t.Fatalf("expected FfiErrorRejected for foreign-fault containment, got %v", err)
		}
		if contained.Code != "INTERNAL_ERROR" {
			t.Fatalf("foreign-fault containment code: got %q want INTERNAL_ERROR", contained.Code)
		}
		if contained.Kind != nil || contained.WireCode != nil {
			t.Fatalf("foreign-fault containment details: got kind=%v wire_code=%v want both nil", contained.Kind, contained.WireCode)
		}

		healthyJSON, err := dialer.PutKnowledgeEntry(`{"schema_version":1,"entry_id":"kb_ffi_ports_after","entry_type":"knowledge","canonical_name":"After Containment","status":"active","body":{"summary":"served through the foreign ports callback"},"extensions":{}}`, nil)
		if err != nil {
			t.Fatalf("serve loop must survive foreign-fault containment: %v", err)
		}
		if got := jsonField(t, healthyJSON, "entry_id"); got != "kb_ffi_ports_after" {
			t.Fatalf("post-containment put entry_id: got %q", got)
		}

		// Post-close state: both FFI faces report Closed after close.
		dialer.Close()
		responder.Close()
		if got := dialer.State(); got != "Closed" {
			t.Fatalf("ports dialer state after close: got %q want Closed", got)
		}
		if got := responder.State(); got != "Closed" {
			t.Fatalf("ports responder state after close: got %q want Closed", got)
		}
	})

	t.Run("absent-ports constructor is valid and denies fail-closed", func(t *testing.T) {
		// Optional families negotiated, but the responder is built WITHOUT a
		// PortsHandler: the capability gate passes, the serving probe finds
		// no ports face, and every optional op denies with the preserved
		// op_unsupported wire code (the documented absent-ports default).
		responder, dialer := dialPortsPair(t, seedClient, seedHost, pubkeyHost, pubkeyClient, fixture, nil)

		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		if got := dialer.State(); got != "Established" {
			t.Fatalf("absent-ports dialer state: got %q want Established", got)
		}
		waitForState(t, "absent-ports responder handshake", responder.State, "Established")

		assertOptionalOpsDenied(t, dialer, "absent-ports deny")
	})

	t.Run("capability-gate deny for optional ops", func(t *testing.T) {
		// Default manifests advertise spoke-baseline only, so the negotiated
		// set lacks l2-computable / l5-fork and every optional op is denied
		// at the responder's dispatch gate with the preserved op_unsupported
		// wire code.
		pair := sc.NewLoopbackTransportPair()
		responder, err := sc.NewConnectResponderFfi(
			&loopbackCallbackTransport{inner: pair.Server()},
			seedHost,
			toolManifestJSON("test-responder"),
			[]string{fixture.PeerIDClient},
			map[string][]byte{fixture.PeerIDClient: pubkeyClient},
			nil,
			nil,
		)
		if err != nil {
			t.Fatalf("connect responder ffi: %v", err)
		}
		dialer, err := sc.ConnectRemoteAdapterFfi(
			&loopbackCallbackTransport{inner: pair.Client()},
			seedClient,
			toolManifestJSON("test-client"),
			pubkeyHost,
			[]string{fixture.PeerIDHost},
			nil,
		)
		if err != nil {
			t.Fatalf("connect remote adapter ffi: %v", err)
		}

		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		if got := dialer.State(); got != "Established" {
			t.Fatalf("capability-deny dialer state: got %q want Established", got)
		}
		waitForState(t, "capability-deny responder handshake", responder.State, "Established")

		assertOptionalOpsDenied(t, dialer, "capability deny")
	})

	// KE remote (F1/F2/F3) matrix — the `extract` service face and the
	// `ke-ownership` gate on the existing Scope query.
	runKeRemoteSubtests(t, fixture)
}

// dialPortsPair — loopback pair through both FFI faces with an optional
// foreign PortsHandler; both manifests declare the optional families. Mirror
// of the Rust dial_responder_ffi_with_ports test helper.
func dialPortsPair(t *testing.T, seedClient, seedHost, pubkeyHost, pubkeyClient []byte, fixture loopbackFixture, ports *sc.PortsHandler) (*sc.ConnectResponderFfi, *sc.RemoteAdapterFfi) {
	t.Helper()
	pair := sc.NewLoopbackTransportPair()
	responder, err := sc.NewConnectResponderFfi(
		&loopbackCallbackTransport{inner: pair.Server()},
		seedHost,
		portsManifestJSON("test-responder"),
		[]string{fixture.PeerIDClient},
		map[string][]byte{fixture.PeerIDClient: pubkeyClient},
		ports,
		nil,
	)
	if err != nil {
		t.Fatalf("connect responder ffi: %v", err)
	}
	dialer, err := sc.ConnectRemoteAdapterFfi(
		&loopbackCallbackTransport{inner: pair.Client()},
		seedClient,
		portsManifestJSON("test-client"),
		pubkeyHost,
		[]string{fixture.PeerIDHost},
		nil,
	)
	if err != nil {
		t.Fatalf("connect remote adapter ffi: %v", err)
	}
	return responder, dialer
}

func assertOptionalOpsDenied(t *testing.T, dialer *sc.RemoteAdapterFfi, what string) {
	t.Helper()
	cases := []struct {
		name string
		call func(string) (string, error)
		json string
	}{
		{"project", dialer.Project, `{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}`},
		{"compute", dialer.Compute, `{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_cmp","computable":{"tide_level":2.5,"cargo_tons":37},"settle":true}`},
		{"listForkTimelineEvents", dialer.ListForkTimelineEvents, `{"scope_id":"pkt_tw_scope","fork_id":"fork_tw_ffi_events"}`},
	}
	for _, c := range cases {
		_, err := c.call(c.json)
		if err == nil {
			t.Fatalf("%s: %s must deny", what, c.name)
		}
		assertRejected(t, err, "CAPABILITY_PORT_MISSING", "op_unsupported")
	}
}

// jsonField — dotted-path JSON field lookup rendered as a string (helper for
// semantic assertions; a missing field fails the test).
func jsonField(t *testing.T, raw string, path string) string {
	t.Helper()
	var value any
	if err := json.Unmarshal([]byte(raw), &value); err != nil {
		t.Fatalf("parse json %q: %v", raw, err)
	}
	var current any = value
	for _, part := range strings.Split(path, ".") {
		switch node := current.(type) {
		case map[string]any:
			var ok bool
			current, ok = node[part]
			if !ok {
				t.Fatalf("json path %q: missing field %q", path, part)
			}
		case []any:
			var index int
			if _, err := fmt.Sscanf(part, "%d", &index); err != nil || index < 0 || index >= len(node) {
				t.Fatalf("json path %q: invalid array index %q", path, part)
			}
			current = node[index]
		default:
			t.Fatalf("json path %q: %q is neither object nor array", path, part)
		}
	}
	return fmt.Sprint(current)
}

// portsManifestJSON — ports-carrying manifest: baseline + optional families,
// so the negotiated set includes l2-computable / l5-fork. Mirror of the Rust
// ports_manifest_json test helper.
func portsManifestJSON(hostID string) string {
	return `{"schema_version":1,"host_id":"` + hostID + `","roles":["data-store","l2-computable"],"capabilities":["spoke-baseline","l2-computable","l5-fork"],"namespaces":["toy_world"],"extensions":{}}`
}

// smokePortsHandler — foreign-callback PortsHandler: in-memory knowledge
// store plus canned optional-family answers; unknown entries reject with an
// application FfiErrorRejected (ordinary deny — not containment);
// kb_ffi_ports_boom faults (the containment row). Mirror of the Rust
// TestPortsHandler.
type smokePortsHandler struct {
	entries map[string]json.RawMessage
}

func (h *smokePortsHandler) GetKnowledgeEntry(entryID string) (string, error) {
	if entryID == "kb_ffi_ports_boom" {
		return "", errors.New("foreign ports handler fault (containment row)")
	}
	if entry, ok := h.entries[entryID]; ok {
		return string(entry), nil
	}
	kind := "store_miss"
	return "", sc.NewFfiErrorRejected("KNOWLEDGE_ENTRY_NOT_FOUND", fmt.Sprintf("entry %s not found", entryID), &kind, nil)
}

func (h *smokePortsHandler) PutKnowledgeEntry(entryJSON string, expectedBaseRevision *uint64) (string, error) {
	var entry map[string]any
	if err := json.Unmarshal([]byte(entryJSON), &entry); err != nil {
		return "", err
	}
	entryID, _ := entry["entry_id"].(string)
	h.entries[entryID] = json.RawMessage(entryJSON)
	return entryJSON, nil
}

func (h *smokePortsHandler) GetRelation(relationID string) (string, error) {
	return "", sc.NewFfiErrorRejected("INVALID_INPUT", "relation serving not exercised by this test handler", nil, nil)
}

func (h *smokePortsHandler) PutRelation(relationJSON string, expectedBaseRevision *uint64) (string, error) {
	return "", sc.NewFfiErrorRejected("INVALID_INPUT", "relation serving not exercised by this test handler", nil, nil)
}

func (h *smokePortsHandler) ListKnowledgeEntries(scopeJSON string) (string, error) {
	entries := make([]json.RawMessage, 0, len(h.entries))
	for _, entry := range h.entries {
		entries = append(entries, entry)
	}
	out, err := json.Marshal(entries)
	if err != nil {
		return "", err
	}
	return string(out), nil
}

func (h *smokePortsHandler) ListTimelineEvents(scopeJSON string) (string, error) {
	return "[]", nil
}

func (h *smokePortsHandler) PutFindings(findingsJSON string) (string, error) {
	return "[]", nil
}

func (h *smokePortsHandler) ListRules(ruleRefs []string) (string, error) {
	return "[]", nil
}

func (h *smokePortsHandler) ListPeerHostCapabilityManifests() (string, error) {
	return "[]", nil
}

func (h *smokePortsHandler) Project(projectRequestJSON string) (string, error) {
	var request map[string]any
	if err := json.Unmarshal([]byte(projectRequestJSON), &request); err != nil {
		return "", err
	}
	return `{"session_id":"` + request["session_id"].(string) + `","entry_id":"` + request["entry_id"].(string) + `","computable":{"tide_level":2.4,"cargo_tons":38}}`, nil
}

func (h *smokePortsHandler) Compute(computeRequestJSON string) (string, error) {
	var request map[string]any
	if err := json.Unmarshal([]byte(computeRequestJSON), &request); err != nil {
		return "", err
	}
	computable, err := json.Marshal(request["computable"])
	if err != nil {
		return "", err
	}
	return `{"session_id":"` + request["session_id"].(string) + `","entry_id":"` + request["entry_id"].(string) + `","computable":` + string(computable) + `,"state":` + string(computable) + `}`, nil
}

func (h *smokePortsHandler) ListForkTimelineEvents(scopeJSON string) (string, error) {
	var scope map[string]any
	if err := json.Unmarshal([]byte(scopeJSON), &scope); err != nil {
		return "", err
	}
	if scope["fork_id"] != "fork_tw_ffi_events" {
		return "[]", nil
	}
	return `[{"schema_version":1,"timeline_event_id":"evt_tw_ffi_storm","canonical_name":"FFI Fork Storm","fork_id":"fork_tw_ffi_events","extensions":{}}]`, nil
}

// ── KE remote (F1/F2/F3) ───────────────────────────────────────────────────
//
// The `extract` service face and the `ke-ownership` gate on the existing
// Scope query, mirroring the Rust `ke_remote_ffi_tests` battery
// (`crates/spoke-connect/src/ffi.rs`). The generated `PortsHandler`
// interface carries no loader method — source loading is the serving host's
// own business — so the loader value stays a private handler field and the
// canary below exists nowhere else: not in the request, not in the expected
// response. It is asserted absent from the recorded envelopes.

const keLoaderCanary = "ke-ffi-loader-canary-go"

// Extract — the existing ports smoke double serves only the optional `port.*`
// families, so `extract` is an ordinary application refusal (never the
// absent-provider probe deny). The method lives beside the KE handler it
// matches because Go methods belong to the package, not the file.
func (h *smokePortsHandler) Extract(extractRequestJSON string) (string, error) {
	return "", sc.NewFfiErrorRejected(
		"CAPABILITY_PORT_MISSING",
		"this ports smoke double does not serve extraction",
		nil,
		nil,
	)
}

// keManifestJSON — both peers' hello manifests: the baseline capability plus
// whatever the scenario negotiates. A capability must appear in *both*
// hellos to be negotiated, so the omitted side is how the deny scenarios are
// set up. Mirror of the Rust `ke_manifest_json` test helper.
func keManifestJSON(hostID string, capabilities []string) string {
	all := append([]string{"spoke-baseline"}, capabilities...)
	out, err := json.Marshal(map[string]any{
		"schema_version": 1,
		"host_id":        hostID,
		"roles":          []string{"data-store"},
		"capabilities":   all,
		"namespaces":     []string{"toy_world"},
		"extensions":     map[string]any{},
	})
	if err != nil {
		panic(err)
	}
	return string(out)
}

// keExtractRequestJSON — an `extract` request for the scenario's run id: the
// payload is the `ExtractRequest` itself and `sources` carry references only.
func keExtractRequestJSON(runID string) string {
	return `{"run_id":"` + runID + `","sources":[` +
		`{"schema_version":1,"source_id":"manuscript/ch1","extensions":{}},` +
		`{"schema_version":1,"source_id":"manuscript/ch2","extensions":{}}]}`
}

// keViewpointScopeJSON — a Scope carrying a reader viewpoint plus opaque
// extension values the callback must receive unchanged.
func keViewpointScopeJSON() string {
	return `{"scope_id":"toy-scope-001","viewpoint":"kb_tw_mira","entry_types":["note"],` +
		`"extensions":{"product":{"viewpoint":"decoy","owner":"someone"}}}`
}

// keEntry — one served KnowledgeEntry; `visibility` rides the opaque
// extensions and is the host's own filtering input, not a wire contract.
func keEntry(entryID, canonicalName, visibility string) map[string]any {
	return map[string]any{
		"schema_version": 1,
		"entry_id":       entryID,
		"entry_type":     "knowledge",
		"canonical_name": canonicalName,
		"status":         "active",
		"body":           map[string]any{"summary": "served through the foreign ports callback"},
		"extensions":     map[string]any{"visibility": map[string]any{"scope": visibility}},
	}
}

// recordingTransport — dialer-side transport that records every envelope in
// both directions, so the smoke can assert what did and did not reach the
// wire.
type recordingTransport struct {
	inner  *sc.LoopbackTransport
	frames [][]byte
}

func (t *recordingTransport) Send(envelope []byte) error {
	t.frames = append(t.frames, envelope)
	return t.inner.Send(envelope)
}

func (t *recordingTransport) Recv() ([]byte, error) {
	envelope, err := t.inner.Recv()
	if err == nil {
		t.frames = append(t.frames, envelope)
	}
	return envelope, err
}

func (t *recordingTransport) Close() error {
	return t.inner.Close()
}

// keRemotePortsHandler — foreign-callback PortsHandler for the KE remote
// matrix: a private host-local loader value, the `extract` service face, and
// a viewpoint-filtered knowledge store. Mirror of the Rust
// `KeRemotePortsHandler` double.
type keRemotePortsHandler struct {
	declineExtract  bool
	extractRequests []map[string]any
	scopeCalls      []map[string]any
	loadedCanary    string
	entries         []map[string]any
}

func newKeRemotePortsHandler(declineExtract bool) *keRemotePortsHandler {
	return &keRemotePortsHandler{
		declineExtract: declineExtract,
		loadedCanary:   keLoaderCanary,
		entries: []map[string]any{
			keEntry("kb_ffi_ke_foreign", "FFI KE Foreign", "private:kb_tw_other"),
			keEntry("kb_ffi_ke_own", "FFI KE Own", "private:kb_tw_mira"),
			keEntry("kb_ffi_ke_shared", "FFI KE Shared", "shared"),
		},
	}
}

func (h *keRemotePortsHandler) Extract(extractRequestJSON string) (string, error) {
	var request map[string]any
	if err := json.Unmarshal([]byte(extractRequestJSON), &request); err != nil {
		return "", err
	}
	h.extractRequests = append(h.extractRequests, request)
	if h.declineExtract {
		return "", sc.NewFfiErrorRejected(
			"CAPABILITY_PORT_MISSING",
			"this binding does not serve extraction",
			nil,
			nil,
		)
	}
	// The loader runs inside the host service: its value feeds the extractor
	// here and is never a transport argument or callback value.
	if h.loadedCanary == "" {
		return "", errors.New("host loader value is missing")
	}
	runID, _ := request["run_id"].(string)
	sources, _ := request["sources"].([]any)
	candidates := make([]map[string]any, 0, len(sources))
	for index := range sources {
		candidates = append(candidates, map[string]any{
			"schema_version": 1,
			"entry_id":       fmt.Sprintf("ke-ffi-%s-%d", runID, index),
			"entry_type":     "note",
			"canonical_name": fmt.Sprintf("Extracted note %d", index),
			"status":         "provisional",
			"body":           map[string]any{"summary": fmt.Sprintf("provisional candidate %d", index)},
			"extensions":     map[string]any{},
		})
	}
	out, err := json.Marshal(map[string]any{
		"candidates": candidates,
		"run":        map[string]any{"run_id": runID},
	})
	if err != nil {
		return "", err
	}
	return string(out), nil
}

func (h *keRemotePortsHandler) ListKnowledgeEntries(scopeJSON string) (string, error) {
	var scope map[string]any
	if err := json.Unmarshal([]byte(scopeJSON), &scope); err != nil {
		return "", err
	}
	h.scopeCalls = append(h.scopeCalls, scope)
	viewpoint, _ := scope["viewpoint"].(string)
	visible := make([]map[string]any, 0, len(h.entries))
	for _, entry := range h.entries {
		visibility := entry["extensions"].(map[string]any)["visibility"].(map[string]any)["scope"]
		if visibility == "shared" || visibility == "private:"+viewpoint {
			visible = append(visible, entry)
		}
	}
	sort.Slice(visible, func(i, j int) bool {
		return visible[i]["entry_id"].(string) < visible[j]["entry_id"].(string)
	})
	out, err := json.Marshal(visible)
	if err != nil {
		return "", err
	}
	return string(out), nil
}

// keUnserved — catalogue ops this double does not serve: an ordinary
// application reject (`wire_code` nil), never containment.
func (h *keRemotePortsHandler) keUnserved(op string) (string, error) {
	return "", sc.NewFfiErrorRejected(
		"INVALID_INPUT",
		op+" is not served by this test handler",
		nil,
		nil,
	)
}

func (h *keRemotePortsHandler) GetKnowledgeEntry(entryID string) (string, error) {
	return h.keUnserved("get_knowledge_entry")
}

func (h *keRemotePortsHandler) PutKnowledgeEntry(entryJSON string, expectedBaseRevision *uint64) (string, error) {
	return h.keUnserved("put_knowledge_entry")
}

func (h *keRemotePortsHandler) GetRelation(relationID string) (string, error) {
	return h.keUnserved("get_relation")
}

func (h *keRemotePortsHandler) PutRelation(relationJSON string, expectedBaseRevision *uint64) (string, error) {
	return h.keUnserved("put_relation")
}

func (h *keRemotePortsHandler) ListTimelineEvents(scopeJSON string) (string, error) {
	return h.keUnserved("list_timeline_events")
}

func (h *keRemotePortsHandler) PutFindings(findingsJSON string) (string, error) {
	return h.keUnserved("put_findings")
}

func (h *keRemotePortsHandler) ListRules(ruleRefs []string) (string, error) {
	return h.keUnserved("list_rules")
}

func (h *keRemotePortsHandler) ListPeerHostCapabilityManifests() (string, error) {
	return h.keUnserved("list_peer_host_capability_manifests")
}

func (h *keRemotePortsHandler) Project(projectRequestJSON string) (string, error) {
	return h.keUnserved("project")
}

func (h *keRemotePortsHandler) Compute(computeRequestJSON string) (string, error) {
	return h.keUnserved("compute")
}

func (h *keRemotePortsHandler) ListForkTimelineEvents(scopeJSON string) (string, error) {
	return h.keUnserved("list_fork_timeline_events")
}

// dialKePair — loopback pair through both FFI faces with the scenario's
// negotiated capabilities and optional foreign PortsHandler. Mirror of the
// Rust `dial_ke_remote` test helper.
func dialKePair(
	t *testing.T,
	fixture loopbackFixture,
	clientCapabilities, responderCapabilities []string,
	ports *sc.PortsHandler,
) (*sc.ConnectResponderFfi, *sc.RemoteAdapterFfi, *recordingTransport) {
	t.Helper()
	pair := sc.NewLoopbackTransportPair()
	responder, err := sc.NewConnectResponderFfi(
		&loopbackCallbackTransport{inner: pair.Server()},
		decodeLoopbackHex(t, fixture.SeedHostHex),
		keManifestJSON("test-responder", responderCapabilities),
		[]string{fixture.PeerIDClient},
		map[string][]byte{fixture.PeerIDClient: decodeLoopbackHex(t, fixture.PubkeyClientHex)},
		ports,
		nil,
	)
	if err != nil {
		t.Fatalf("ke responder ffi: %v", err)
	}
	transport := &recordingTransport{inner: pair.Client()}
	dialer, err := sc.ConnectRemoteAdapterFfi(
		transport,
		decodeLoopbackHex(t, fixture.SeedClientHex),
		keManifestJSON("test-client", clientCapabilities),
		decodeLoopbackHex(t, fixture.PubkeyHostHex),
		[]string{fixture.PeerIDHost},
		nil,
	)
	if err != nil {
		t.Fatalf("ke dial ffi: %v", err)
	}
	if got := dialer.State(); got != "Established" {
		t.Fatalf("ke dialer state: got %q want Established", got)
	}
	waitForState(t, "ke responder handshake", responder.State, "Established")
	return responder, dialer, transport
}

// assertUnavailableCapability — the unavailable-capability row shared by the
// unnegotiated and absent-provider denials (the callback-refusal row differs
// in `wire_code` and is asserted at its own case).
func assertUnavailableCapability(t *testing.T, err error, wireCode string, what string) {
	t.Helper()
	var rejected *sc.FfiErrorRejected
	if !errors.As(err, &rejected) {
		t.Fatalf("%s: expected FfiErrorRejected, got %v", what, err)
	}
	if rejected.Code != "CAPABILITY_PORT_MISSING" {
		t.Fatalf("%s: code got %q want CAPABILITY_PORT_MISSING", what, rejected.Code)
	}
	if rejected.Kind != nil {
		t.Fatalf("%s: kind got %v want nil", what, *rejected.Kind)
	}
	if rejected.WireCode == nil || *rejected.WireCode != wireCode {
		t.Fatalf("%s: wire_code got %v want %q", what, rejected.WireCode, wireCode)
	}
}

// keValue — dotted-path lookup into a decoded KE remote response (a missing
// field fails the test rather than yielding a zero value).
func keValue(t *testing.T, raw string, path ...string) any {
	t.Helper()
	var current any
	if err := json.Unmarshal([]byte(raw), &current); err != nil {
		t.Fatalf("parse json %q: %v", raw, err)
	}
	for _, part := range path {
		switch node := current.(type) {
		case map[string]any:
			value, ok := node[part]
			if !ok {
				t.Fatalf("json path %v: missing field %q", path, part)
			}
			current = value
		case []any:
			index := 0
			if _, err := fmt.Sscanf(part, "%d", &index); err != nil || index < 0 || index >= len(node) {
				t.Fatalf("json path %v: invalid array index %q", path, part)
			}
			current = node[index]
		default:
			t.Fatalf("json path %v: %q is neither object nor array", path, part)
		}
	}
	return current
}

func keStringAt(t *testing.T, raw string, path ...string) string {
	t.Helper()
	value, ok := keValue(t, raw, path...).(string)
	if !ok {
		t.Fatalf("json path %v: not a string in %q", path, raw)
	}
	return value
}

func keEntryIDs(t *testing.T, raw string) []string {
	t.Helper()
	items, ok := keValue(t, raw).([]any)
	if !ok {
		t.Fatalf("served entries are not a JSON array: %q", raw)
	}
	ids := make([]string, 0, len(items))
	for index, item := range items {
		entry, ok := item.(map[string]any)
		if !ok {
			t.Fatalf("served entry %d is not an object", index)
		}
		id, ok := entry["entry_id"].(string)
		if !ok {
			t.Fatalf("served entry %d has no entry_id", index)
		}
		ids = append(ids, id)
	}
	return ids
}

// runKeRemoteSubtests — the frozen KE remote matrix, registered as subtests of
// the ports-loopback pair test so the documented `-run` entry exercises it.
func runKeRemoteSubtests(t *testing.T, fixture loopbackFixture) {
	t.Helper()

	t.Run("ke remote extract round-trips the provisional batch", func(t *testing.T) {
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, []string{"ke-extraction"}, []string{"ke-extraction"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		response, err := dialer.Extract(keExtractRequestJSON("run-ffi-ke-1"))
		if err != nil {
			t.Fatalf("extract through the foreign callback: %v", err)
		}
		// Observable, deterministic response behaviour: one provisional
		// candidate per declared source reference, carrying the run id.
		if got := keStringAt(t, response, "run", "run_id"); got != "run-ffi-ke-1" {
			t.Fatalf("extract run_id: got %q want run-ffi-ke-1", got)
		}
		candidates, ok := keValue(t, response, "candidates").([]any)
		if !ok || len(candidates) != 2 {
			t.Fatalf("extract candidates: got %v want two entries", keValue(t, response, "candidates"))
		}
		wantIDs := []string{"ke-ffi-run-ffi-ke-1-0", "ke-ffi-run-ffi-ke-1-1"}
		for index, want := range wantIDs {
			if got := keStringAt(t, response, "candidates", fmt.Sprint(index), "entry_id"); got != want {
				t.Fatalf("candidate %d entry_id: got %q want %q", index, got, want)
			}
			if got := keStringAt(t, response, "candidates", fmt.Sprint(index), "status"); got != "provisional" {
				t.Fatalf("candidate %d status: got %q want provisional", index, got)
			}
			if got := keStringAt(t, response, "candidates", fmt.Sprint(index), "entry_type"); got != "note" {
				t.Fatalf("candidate %d entry_type: got %q want note", index, got)
			}
		}

		// The callback received the `ExtractRequest` itself: no wrapper and no
		// loader value — only references travelled.
		if len(handler.extractRequests) != 1 {
			t.Fatalf("callback extract calls: got %d want 1", len(handler.extractRequests))
		}
		request := handler.extractRequests[0]
		if got, _ := request["run_id"].(string); got != "run-ffi-ke-1" {
			t.Fatalf("callback run_id: got %q", got)
		}
		sources, _ := request["sources"].([]any)
		if len(sources) != 2 {
			t.Fatalf("callback sources: got %d want 2", len(sources))
		}
		if got := sources[0].(map[string]any)["source_id"]; got != "manuscript/ch1" {
			t.Fatalf("callback source_id: got %v", got)
		}
		for _, wrapper := range []string{"request", "arguments", "input", "loaded_input"} {
			if _, present := request[wrapper]; present {
				t.Fatalf("callback request carries a %q wrapper", wrapper)
			}
		}
	})

	t.Run("ke remote extract rejects malformed request json with zero wire traffic", func(t *testing.T) {
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, transport := dialKePair(t, fixture, []string{"ke-extraction"}, []string{"ke-extraction"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		framesBefore := len(transport.frames)
		_, err := dialer.Extract("{ not json")
		var rejected *sc.FfiErrorRejected
		if !errors.As(err, &rejected) {
			t.Fatalf("expected FfiErrorRejected for malformed extract json, got %v", err)
		}
		if rejected.Code != "INVALID_INPUT" {
			t.Fatalf("malformed extract json code: got %q want INVALID_INPUT", rejected.Code)
		}
		if rejected.Kind != nil || rejected.WireCode != nil {
			t.Fatalf("malformed extract json details: got kind=%v wire_code=%v want both nil", rejected.Kind, rejected.WireCode)
		}
		if len(handler.extractRequests) != 0 {
			t.Fatalf("the malformed request must not reach the callback")
		}
		if len(transport.frames) != framesBefore {
			t.Fatalf("the malformed request must not reach the wire (frames %d -> %d)", framesBefore, len(transport.frames))
		}
	})

	t.Run("ke remote extract denies when the responding hello omits ke-extraction", func(t *testing.T) {
		// Otherwise identical manifests with one flag omitted: the dialing
		// hello declares `ke-extraction`, the responding hello does not.
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, []string{"ke-extraction"}, nil, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		_, err := dialer.Extract(keExtractRequestJSON("run-ffi-unnegotiated"))
		assertUnavailableCapability(t, err, "op_unsupported", "unnegotiated (responder omits the flag)")
		if len(handler.extractRequests) != 0 {
			t.Fatalf("the capability gate denies before the callback")
		}
	})

	t.Run("ke remote extract denies when the dialing hello omits ke-extraction", func(t *testing.T) {
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, nil, []string{"ke-extraction"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		_, err := dialer.Extract(keExtractRequestJSON("run-ffi-unnegotiated"))
		assertUnavailableCapability(t, err, "op_unsupported", "unnegotiated (dialer omits the flag)")
		if len(handler.extractRequests) != 0 {
			t.Fatalf("the capability gate denies before the callback")
		}
	})

	t.Run("ke remote extract probe denies when ports are absent", func(t *testing.T) {
		// `ke-extraction` is negotiated in both hellos but the responder has
		// no ports face at all: the absent-provider probe deny.
		responder, dialer, _ := dialKePair(t, fixture, []string{"ke-extraction"}, []string{"ke-extraction"}, nil)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		_, err := dialer.Extract(keExtractRequestJSON("run-ffi-absent-ports"))
		assertUnavailableCapability(t, err, "op_unsupported", "absent-ports probe deny")
	})

	t.Run("ke remote extract passes a callback application refusal through", func(t *testing.T) {
		handler := newKeRemotePortsHandler(true)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, []string{"ke-extraction"}, []string{"ke-extraction"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		_, err := dialer.Extract(keExtractRequestJSON("run-ffi-refusal"))
		var rejected *sc.FfiErrorRejected
		if !errors.As(err, &rejected) {
			t.Fatalf("expected FfiErrorRejected for the callback refusal, got %v", err)
		}
		if rejected.Code != "CAPABILITY_PORT_MISSING" {
			t.Fatalf("refusal code: got %q want CAPABILITY_PORT_MISSING", rejected.Code)
		}
		if rejected.Kind != nil {
			t.Fatalf("refusal kind: got %v want nil", *rejected.Kind)
		}
		if rejected.WireCode != nil {
			t.Fatalf("refusal wire_code: got %v want nil (not a missing-method probe deny)", *rejected.WireCode)
		}
		if !strings.Contains(rejected.Message, "does not serve extraction") {
			t.Fatalf("refusal message must survive: %q", rejected.Message)
		}
		if len(handler.extractRequests) != 1 {
			t.Fatalf("a refusal is the callback's own answer, not a probe deny (calls %d)", len(handler.extractRequests))
		}
	})

	t.Run("ke remote scope viewpoint serves with ownership negotiated", func(t *testing.T) {
		// OQ-FFI-1: the existing Scope method carries the ownership gate — no
		// ownership-specific method or callback is added.
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, []string{"ke-ownership"}, []string{"ke-ownership"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		served, err := dialer.ListKnowledgeEntries(keViewpointScopeJSON())
		if err != nil {
			t.Fatalf("a negotiated viewpoint request must serve: %v", err)
		}
		gotIDs := keEntryIDs(t, served)
		wantIDs := []string{"kb_ffi_ke_own", "kb_ffi_ke_shared"}
		if len(gotIDs) != len(wantIDs) || gotIDs[0] != wantIDs[0] || gotIDs[1] != wantIDs[1] {
			t.Fatalf("served entries: got %v want %v (the host's viewpoint filter)", gotIDs, wantIDs)
		}
		if got := keStringAt(t, served, "0", "canonical_name"); got != "FFI KE Own" {
			t.Fatalf("served canonical_name: got %q", got)
		}

		// The callback received the declared Scope unchanged: the viewpoint
		// and the opaque extension values are not stripped to make the request
		// succeed, and no extraction call was involved.
		if len(handler.scopeCalls) != 1 {
			t.Fatalf("callback scope calls: got %d want 1", len(handler.scopeCalls))
		}
		scope := handler.scopeCalls[0]
		if got := scope["viewpoint"]; got != "kb_tw_mira" {
			t.Fatalf("callback viewpoint: got %v", got)
		}
		entryTypes, _ := scope["entry_types"].([]any)
		if len(entryTypes) != 1 || entryTypes[0] != "note" {
			t.Fatalf("callback entry_types: got %v", scope["entry_types"])
		}
		product := scope["extensions"].(map[string]any)["product"].(map[string]any)
		if product["viewpoint"] != "decoy" || product["owner"] != "someone" {
			t.Fatalf("callback extensions must be preserved unchanged: got %v", product)
		}
		if len(handler.extractRequests) != 0 {
			t.Fatalf("the Scope query must not invoke extract")
		}
	})

	t.Run("ke remote scope viewpoint denies when the dialing hello omits ke-ownership", func(t *testing.T) {
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, nil, []string{"ke-ownership"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		_, err := dialer.ListKnowledgeEntries(keViewpointScopeJSON())
		assertUnavailableCapability(t, err, "op_unsupported", "ownership deny (dialer omits the flag)")
		if len(handler.scopeCalls) != 0 {
			t.Fatalf("the gate refuses before the callback is reached")
		}
	})

	t.Run("ke remote scope viewpoint denies when the responding hello omits ke-ownership", func(t *testing.T) {
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, _ := dialKePair(t, fixture, []string{"ke-ownership"}, nil, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		_, err := dialer.ListKnowledgeEntries(keViewpointScopeJSON())
		assertUnavailableCapability(t, err, "op_unsupported", "ownership deny (responder omits the flag)")
		if len(handler.scopeCalls) != 0 {
			t.Fatalf("the gate refuses before the callback is reached")
		}
	})

	t.Run("ke remote loader canary never reaches the wire", func(t *testing.T) {
		handler := newKeRemotePortsHandler(false)
		var ports sc.PortsHandler = handler
		responder, dialer, transport := dialKePair(t, fixture, []string{"ke-extraction"}, []string{"ke-extraction"}, &ports)
		t.Cleanup(func() {
			dialer.Close()
			responder.Close()
		})

		response, err := dialer.Extract(keExtractRequestJSON("run-ffi-canary"))
		if err != nil {
			t.Fatalf("extract for the canary row: %v", err)
		}
		if !strings.Contains(response, "run-ffi-canary") {
			t.Fatalf("the round-trip really happened: %q", response)
		}
		if strings.Contains(response, keLoaderCanary) {
			t.Fatalf("the loader value is not serialized into the response")
		}
		if len(transport.frames) == 0 {
			t.Fatalf("the dialer recorded the session envelopes")
		}
		// Positive control: the recording really observes the plaintext wire
		// (the invoke request's run id is visible), so the canary absence
		// below is a real observation, not a vacuous one.
		recorded := string(bytes.Join(transport.frames, nil))
		if !strings.Contains(recorded, "run-ffi-canary") {
			t.Fatalf("the recorded envelopes must carry the request")
		}
		if strings.Contains(recorded, keLoaderCanary) {
			t.Fatalf("the loader canary must never appear in a recorded envelope")
		}
	})
}
