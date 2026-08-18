package smoke_test

import (
	"encoding/json"
	"errors"
	"fmt"
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

const projectRequestJSON = `{"session_id":"sess_ffi_ports","entry_id":"kb_ffi_ports_proj","state":{"tide_level":2.1,"cargo_tons":40}}`

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
