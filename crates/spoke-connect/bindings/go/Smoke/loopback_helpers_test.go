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

// Shared loopback harness helpers — used by the smokehost-tagged
// RemoteAdapterFFI loopback test and the tool-faces loopback test (which runs
// against the committed production binding + native, no smoke host needed).

type loopbackFixture struct {
	SeedClientHex      string `json:"seed_client_hex"`
	SeedHostHex        string `json:"seed_host_hex"`
	PubkeyHostHex      string `json:"pubkey_host_hex"`
	PubkeyClientHex    string `json:"pubkey_client_hex"`
	PeerIDHost         string `json:"peer_id_host"`
	PeerIDClient       string `json:"peer_id_client"`
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
