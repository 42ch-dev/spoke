//go:build smokehost

package smoke_test

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	sc "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go/generated/spoke_connect"
)

type loopbackFixture struct {
	SeedClientHex      string `json:"seed_client_hex"`
	PubkeyHostHex      string `json:"pubkey_host_hex"`
	PeerIDHost         string `json:"peer_id_host"`
	ClientManifestJSON string `json:"client_manifest_json"`
	SessionID          string `json:"session_id"`
	EntryID            string `json:"entry_id"`
	EntryCanonicalName string `json:"entry_canonical_name"`
}

type loopbackCallbackTransport struct {
	inner *sc.LoopbackTransport
}

func (t *loopbackCallbackTransport) Send(envelope []byte) error {
	return t.inner.Send(envelope)
}

func (t *loopbackCallbackTransport) Recv() ([]byte, error) {
	return t.inner.Recv()
}

func (t *loopbackCallbackTransport) Close() error {
	return t.inner.Close()
}

func loadLoopbackFixture(t *testing.T) loopbackFixture {
	t.Helper()
	path := filepath.Join("fixtures", "loopback-smoke.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read loopback fixture: %v", err)
	}
	var fixture loopbackFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parse loopback fixture: %v", err)
	}
	return fixture
}

func decodeLoopbackHex(t *testing.T, value string) []byte {
	t.Helper()
	if len(value)%2 != 0 {
		t.Fatalf("hex must have even length")
	}
	out, err := hex.DecodeString(value)
	if err != nil {
		t.Fatalf("decode hex: %v", err)
	}
	return out
}

func knowledgeEntryJSON(entryID, canonicalName string) string {
	return strings.TrimSpace(`{"schema_version":1,"entry_id":"` + entryID +
		`","entry_type":"character","canonical_name":"` + canonicalName +
		`","status":"provisional","body":{"summary":"Upserted over the loopback: ` + entryID +
		`"},"extensions":{}}`)
}

func TestRemoteAdapterLoopbackPutGetRoundTrip(t *testing.T) {
	fixture := loadLoopbackFixture(t)
	seedClient := decodeLoopbackHex(t, fixture.SeedClientHex)
	pubkeyHost := decodeLoopbackHex(t, fixture.PubkeyHostHex)

	pair := sc.NewLoopbackTransportPair()
	host := sc.StartLoopbackSmokeHost(pair.Server())
	transport := &loopbackCallbackTransport{inner: pair.Client()}
	adapter, err := sc.ConnectRemoteAdapterFfi(
		transport,
		seedClient,
		fixture.ClientManifestJSON,
		pubkeyHost,
		[]string{fixture.PeerIDHost},
		nil,
	)
	if err != nil {
		t.Fatalf("connect remote adapter: %v", err)
	}

	t.Cleanup(func() {
		adapter.Close()
		host.Close()
	})

	if got := adapter.State(); got != "Established" {
		t.Fatalf("adapter state: got %q want Established", got)
	}
	if got := adapter.SessionId(); got == nil || *got != fixture.SessionID {
		t.Fatalf("adapter session_id: got %v want %q", got, fixture.SessionID)
	}
	if got := host.SessionId(); got != fixture.SessionID {
		t.Fatalf("host session_id: got %q want %q", got, fixture.SessionID)
	}
	if got := adapter.RemotePeerId(); got == nil || *got != fixture.PeerIDHost {
		t.Fatalf("remote_peer_id: got %v want %q", got, fixture.PeerIDHost)
	}

	remoteManifest := adapter.RemoteManifest()
	if remoteManifest == nil {
		t.Fatal("remote_manifest is nil")
	}
	var manifest map[string]any
	if err := json.Unmarshal([]byte(*remoteManifest), &manifest); err != nil {
		t.Fatalf("parse remote_manifest: %v", err)
	}
	if got, _ := manifest["host_id"].(string); got != "test-host" {
		t.Fatalf("remote manifest host_id: got %q want test-host", got)
	}

	entryJSON := knowledgeEntryJSON(fixture.EntryID, fixture.EntryCanonicalName)
	putJSON, err := adapter.PutKnowledgeEntry(entryJSON, nil)
	if err != nil {
		t.Fatalf("putKnowledgeEntry: %v", err)
	}
	var putObject map[string]any
	if err := json.Unmarshal([]byte(putJSON), &putObject); err != nil {
		t.Fatalf("parse put response: %v", err)
	}
	if got, _ := putObject["entry_id"].(string); got != fixture.EntryID {
		t.Fatalf("put entry_id: got %q want %q", got, fixture.EntryID)
	}

	getJSON, err := adapter.GetKnowledgeEntry(fixture.EntryID)
	if err != nil {
		t.Fatalf("getKnowledgeEntry: %v", err)
	}
	var getObject map[string]any
	if err := json.Unmarshal([]byte(getJSON), &getObject); err != nil {
		t.Fatalf("parse get response: %v", err)
	}
	if got, _ := getObject["entry_id"].(string); got != fixture.EntryID {
		t.Fatalf("get entry_id: got %q want %q", got, fixture.EntryID)
	}
	if got, _ := getObject["canonical_name"].(string); got != fixture.EntryCanonicalName {
		t.Fatalf("get canonical_name: got %q want %q", got, fixture.EntryCanonicalName)
	}

	adapter.Close()
	if got := adapter.State(); got != "Closed" {
		t.Fatalf("adapter state after close: got %q want Closed", got)
	}
}
