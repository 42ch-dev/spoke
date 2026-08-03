package smoke_test

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"

	spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"
)

var goldenSeed = []byte{
	0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
	0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
	0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
	0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
}

var goldenPubkey = []byte{
	0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9,
	0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b, 0xa7,
	0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7,
	0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04, 0x96, 0x64,
}

const (
	goldenPeerID       = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"
	goldenSignature    = "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg"
	goldenManifestJSON = `{"capabilities":["spoke-baseline"],"extensions":{},"host_id":"golden-host","namespaces":[],"roles":["data-store"],"schema_version":1}`
)

func goldenNonce() string {
	return strings.Join([]string{"golden-nonce", "000000000001"}, "-")
}

func TestGoldenDerivePeerID(t *testing.T) {
	got, err := spokeconnect.DerivePeerIdFromEd25519Pubkey(goldenPubkey)
	if err != nil {
		t.Fatalf("derive_peer_id: %v", err)
	}
	if got != goldenPeerID {
		t.Fatalf("derive_peer_id: got %q, want %q", got, goldenPeerID)
	}
}

func TestGoldenSignHelloSignature(t *testing.T) {
	helloJSON, err := spokeconnect.SignHelloEd25519(goldenSeed, goldenNonce(), goldenManifestJSON)
	if err != nil {
		t.Fatalf("sign_hello: %v", err)
	}

	var envelope map[string]any
	if err := json.Unmarshal([]byte(helloJSON), &envelope); err != nil {
		t.Fatalf("sign_hello JSON: %v", err)
	}

	if got, _ := envelope["peer_id"].(string); got != goldenPeerID {
		t.Fatalf("sign_hello peer_id: got %q, want %q", got, goldenPeerID)
	}
	if got, _ := envelope["signature"].(string); got != goldenSignature {
		t.Fatalf("sign_hello signature: got %q, want %q", got, goldenSignature)
	}
}

func TestGoldenVerifyHello(t *testing.T) {
	helloJSON, err := spokeconnect.SignHelloEd25519(goldenSeed, goldenNonce(), goldenManifestJSON)
	if err != nil {
		t.Fatalf("sign_hello: %v", err)
	}
	if err := spokeconnect.VerifyHelloEd25519(goldenPubkey, goldenPeerID, helloJSON); err != nil {
		t.Fatalf("verify_hello: %v", err)
	}
}

func TestGoldenTamperedHelloRejected(t *testing.T) {
	helloJSON, err := spokeconnect.SignHelloEd25519(goldenSeed, goldenNonce(), goldenManifestJSON)
	if err != nil {
		t.Fatalf("sign_hello: %v", err)
	}
	tampered := strings.Replace(helloJSON, "data-store", "checker", 1)
	err = spokeconnect.VerifyHelloEd25519(goldenPubkey, goldenPeerID, tampered)
	if err == nil {
		t.Fatal("tampered hello was accepted")
	}
	if !errors.Is(err, spokeconnect.ErrCoreErrorInvalidHelloSignature) {
		t.Fatalf("tampered hello: got %v, want InvalidHelloSignature", err)
	}
}

func TestGoldenProtocolVersion(t *testing.T) {
	if got := spokeconnect.ProtocolVersion(); got != 1 {
		t.Fatalf("protocol_version: got %d, want 1", got)
	}
}
