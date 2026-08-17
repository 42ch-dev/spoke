package smoke_test

import (
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"testing"
	"time"

	sc "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go/generated/spoke_connect"
)

// Tool faces over the loopback pair (D15/D16), run against the committed
// production binding + native (no smoke host required): both ends are FFI
// objects. The responder serves a foreign ToolHandler, the dialer serves
// reverse invokes through RemoteAdapterFfi.RegisterToolHandler, unregistered
// tools deny with op_unsupported, and a handler-thrown application reject
// passes through verbatim (parity with
// crates/spoke-connect/src/ffi.rs connect_responder_ffi_tests).

func TestToolLoopbackFfiPair(t *testing.T) {
	fixture := loadLoopbackFixture(t)
	seedClient := decodeLoopbackHex(t, fixture.SeedClientHex)
	seedHost := decodeLoopbackHex(t, fixture.SeedHostHex)
	pubkeyHost := decodeLoopbackHex(t, fixture.PubkeyHostHex)
	pubkeyClient := decodeLoopbackHex(t, fixture.PubkeyClientHex)

	pair := sc.NewLoopbackTransportPair()
	// The accept-side constructor returns immediately in `Handshaking` (D16):
	// the dialer hello is the sync point, so the smoke polls `state()`
	// (bounded) to `Established` before invoking; a handshake failure
	// surfaces as `Closed`, never a thrown constructor error.
	responder, err := sc.NewConnectResponderFfi(
		&loopbackCallbackTransport{inner: pair.Server()},
		seedHost,
		toolManifestJSON("test-responder"),
		[]string{fixture.PeerIDClient},
		map[string][]byte{fixture.PeerIDClient: pubkeyClient},
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
		t.Fatalf("tool dialer state: got %q want Established", got)
	}
	waitForState(t, "tool responder handshake", responder.State, "Established")

	// 1. Dialer FFI invoke_tool -> responder FFI foreign ToolHandler.
	responderSum := &sumToolHandler{}
	if err := responder.RegisterToolHandler("tools.math.add", responderSum); err != nil {
		t.Fatalf("responder register_tool_handler: %v", err)
	}
	sumJSON, err := dialer.InvokeTool("tools.math.add", `{"a": 1, "b": 2}`)
	if err != nil {
		t.Fatalf("dialer invoke_tool: %v", err)
	}
	assertSum(t, sumJSON, 3, "dialer invoke_tool answered by responder foreign ToolHandler")
	if got := responderSum.calls(); got != 1 {
		t.Fatalf("responder handler invocation count: got %d want 1", got)
	}

	// 2. Responder FFI invoke_tool -> dialer-side handler registered via
	//    RemoteAdapterFfi.RegisterToolHandler.
	dialerSum := &sumToolHandler{}
	if err := dialer.RegisterToolHandler("tools.math.add", dialerSum); err != nil {
		t.Fatalf("dialer register_tool_handler: %v", err)
	}
	reverseSumJSON, err := responder.InvokeTool("tools.math.add", `{"a": 21, "b": 21}`)
	if err != nil {
		t.Fatalf("responder invoke_tool: %v", err)
	}
	assertSum(t, reverseSumJSON, 42, "responder invoke_tool answered by dialer-side handler")
	if got := dialerSum.calls(); got != 1 {
		t.Fatalf("dialer handler invocation count: got %d want 1", got)
	}

	// 3. Negotiated but unregistered tool -> fail-closed op_unsupported.
	_, err = dialer.InvokeTool("tools.echo.boom", `{}`)
	assertRejected(t, err, "CAPABILITY_PORT_MISSING", "op_unsupported")

	// 4. Handler-thrown application reject passes through verbatim (kind /
	//    wire_code re-hung onto details by the bridge).
	rejectWire := "op_unsupported"
	if err := dialer.RegisterToolHandler(
		"tools.echo.boom",
		&throwingToolHandler{err: sc.NewFfiErrorRejected("REVISION_CONFLICT", "foreign handler rejected", nil, &rejectWire)},
	); err != nil {
		t.Fatalf("dialer register reject handler: %v", err)
	}
	_, err = responder.InvokeTool("tools.echo.boom", `{}`)
	var passed *sc.FfiErrorRejected
	if !errors.As(err, &passed) {
		t.Fatalf("expected FfiErrorRejected, got %v", err)
	}
	if passed.Code != "REVISION_CONFLICT" {
		t.Fatalf("reject passthrough code: got %q want REVISION_CONFLICT", passed.Code)
	}
	if passed.Message != "foreign handler rejected" {
		t.Fatalf("reject passthrough message: got %q want %q", passed.Message, "foreign handler rejected")
	}
	if passed.WireCode == nil || *passed.WireCode != "op_unsupported" {
		t.Fatalf("reject passthrough wire_code: got %v want op_unsupported", passed.WireCode)
	}

	// 5. Handler rejects with an unknown code string -> the host observes the
	//    INTERNAL_ERROR downgrade (message preserved, details re-hung): the
	//    foreign code cannot be represented by the typed SpokeRejectCode, so
	//    the bridge falls back.
	unknownWire := "op_unsupported"
	if err := dialer.RegisterToolHandler(
		"tools.echo.boom",
		&throwingToolHandler{err: sc.NewFfiErrorRejected("NOT_A_WIRE_CODE", "unknown code message", nil, &unknownWire)},
	); err != nil {
		t.Fatalf("dialer register unknown-code handler: %v", err)
	}
	_, err = responder.InvokeTool("tools.echo.boom", `{}`)
	var downgraded *sc.FfiErrorRejected
	if !errors.As(err, &downgraded) {
		t.Fatalf("expected FfiErrorRejected for unknown-code downgrade, got %v", err)
	}
	if downgraded.Code != "INTERNAL_ERROR" {
		t.Fatalf("unknown-code downgrade code: got %q want INTERNAL_ERROR", downgraded.Code)
	}
	if downgraded.Message != "unknown code message" {
		t.Fatalf("unknown-code downgrade message: got %q want %q", downgraded.Message, "unknown code message")
	}
	if downgraded.WireCode == nil || *downgraded.WireCode != "op_unsupported" {
		t.Fatalf("unknown-code downgrade wire_code: got %v want op_unsupported", downgraded.WireCode)
	}

	// 6. Handler throws a foreign (non-FfiError) fault -> contained to
	//    INTERNAL_ERROR with no details; the session survives and the serve
	//    loop still answers the next healthy reverse invoke.
	//    Channel caveat: the Go vendored-fork bindgen carries the stock uniffi
	//    callback-error machinery unpatched (no fielded-error patch script
	//    like Kotlin's) — the fielded ERROR path is proven by steps 4–5 and
	//    the plain-fault path by this step; assert only what the stock
	//    trampoline can express (non-FfiError error -> unexpected callback
	//    error).
	if err := dialer.RegisterToolHandler(
		"tools.echo.boom",
		&throwingToolHandler{err: errors.New("foreign fault")},
	); err != nil {
		t.Fatalf("dialer register faulting handler: %v", err)
	}
	_, err = responder.InvokeTool("tools.echo.boom", `{}`)
	var contained *sc.FfiErrorRejected
	if !errors.As(err, &contained) {
		t.Fatalf("expected FfiErrorRejected for foreign-fault containment, got %v", err)
	}
	if contained.Code != "INTERNAL_ERROR" {
		t.Fatalf("foreign-fault containment code: got %q want INTERNAL_ERROR", contained.Code)
	}
	if contained.WireCode != nil {
		t.Fatalf("foreign-fault containment wire_code: got %v want nil (details None)", *contained.WireCode)
	}

	healthyJSON, err := responder.InvokeTool("tools.math.add", `{"a": 21, "b": 21}`)
	if err != nil {
		t.Fatalf("serve loop must survive foreign-fault containment: %v", err)
	}
	assertSum(t, healthyJSON, 42, "serve loop survives foreign-fault containment")
	if got := dialerSum.calls(); got != 2 {
		t.Fatalf("dialer handler invocation count after containment: got %d want 2", got)
	}

	// Post-close state: both FFI faces report Closed after close (Cleanup
	// above double-closes; close is idempotent).
	dialer.Close()
	responder.Close()
	if got := dialer.State(); got != "Closed" {
		t.Fatalf("tool dialer state after close: got %q want Closed", got)
	}
	if got := responder.State(); got != "Closed" {
		t.Fatalf("tool responder state after close: got %q want Closed", got)
	}
}

// toolManifestJSON — tool-carrying manifest: every tool capability also sits
// in capabilities[] so the negotiated set includes the tools.* ops (D13
// dispatch gate). Mirror of the Rust `tool_manifest` test helper.
func toolManifestJSON(hostID string) string {
	manifest := map[string]any{
		"schema_version": 1,
		"host_id":        hostID,
		"roles":          []string{"data-store"},
		"capabilities":   []string{"spoke-baseline", "tools.math.add", "tools.echo.echo", "tools.echo.boom"},
		"namespaces":     []string{"math", "echo", "toy_world"},
		"extensions":     map[string]any{},
		"tools": []map[string]any{
			{
				"schema_version": 1,
				"capability_id":  "tools.math.add",
				"op":             "tools.math.add",
				"description":    "Add two integers",
				"input":          map[string]any{"type": "object"},
				"output":         map[string]any{"type": "object"},
			},
			{
				"schema_version": 1,
				"capability_id":  "tools.echo.echo",
				"op":             "tools.echo.echo",
				"description":    "Echo the arguments",
				"input":          map[string]any{"type": "object"},
				"output":         map[string]any{"type": "object"},
			},
			{
				"schema_version": 1,
				"capability_id":  "tools.echo.boom",
				"op":             "tools.echo.boom",
				"description":    "Explodes",
				"input":          map[string]any{"type": "object"},
				"output":         map[string]any{"type": "object"},
			},
		},
	}
	out, err := json.Marshal(manifest)
	if err != nil {
		panic(fmt.Sprintf("tool manifest marshals: %v", err))
	}
	return string(out)
}

func assertSum(t *testing.T, resultJSON string, want int64, what string) {
	t.Helper()
	var result struct {
		Sum int64 `json:"sum"`
	}
	if err := json.Unmarshal([]byte(resultJSON), &result); err != nil {
		t.Fatalf("%s: parse result %q: %v", what, resultJSON, err)
	}
	if result.Sum != want {
		t.Fatalf("%s: got sum %d want %d", what, result.Sum, want)
	}
}

func assertRejected(t *testing.T, err error, wantCode, wantWire string) {
	t.Helper()
	var rejected *sc.FfiErrorRejected
	if !errors.As(err, &rejected) {
		t.Fatalf("expected FfiErrorRejected, got %v", err)
	}
	if rejected.Code != wantCode {
		t.Fatalf("reject code: got %q want %q", rejected.Code, wantCode)
	}
	if rejected.WireCode == nil || *rejected.WireCode != wantWire {
		t.Fatalf("reject wire_code: got %v want %q", rejected.WireCode, wantWire)
	}
}

// Bounded poll for the handshake to settle (D16 constructor semantics).
func waitForState(t *testing.T, what string, state func() string, expected string) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	last := state()
	for last != expected {
		if time.Now().After(deadline) {
			t.Fatalf("%s: timed out waiting for %q (last: %q)", what, expected, last)
		}
		time.Sleep(10 * time.Millisecond)
		last = state()
	}
}

// sumToolHandler — foreign-callback ToolHandler summing a + b (Rust
// add_handler parity) with an invocation counter.
type sumToolHandler struct {
	mu    sync.Mutex
	count int
}

func (h *sumToolHandler) Handle(argumentsJSON string) (string, error) {
	h.mu.Lock()
	h.count++
	h.mu.Unlock()
	var args struct {
		A int64 `json:"a"`
		B int64 `json:"b"`
	}
	if err := json.Unmarshal([]byte(argumentsJSON), &args); err != nil {
		return "", fmt.Errorf("parse tool arguments: %w", err)
	}
	return fmt.Sprintf(`{"sum":%d}`, args.A+args.B), nil
}

func (h *sumToolHandler) calls() int {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.count
}

// throwingToolHandler — foreign-callback ToolHandler that always returns the
// given application reject (D16 passthrough row).
type throwingToolHandler struct {
	err error
}

func (h *throwingToolHandler) Handle(argumentsJSON string) (string, error) {
	return "", h.err
}
