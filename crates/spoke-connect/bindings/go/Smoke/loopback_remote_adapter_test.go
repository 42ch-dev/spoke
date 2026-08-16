//go:build smokehost

package smoke_test

import (
	"encoding/json"
	"testing"

	sc "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go/generated/spoke_connect"
)

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
