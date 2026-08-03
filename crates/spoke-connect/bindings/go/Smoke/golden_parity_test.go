package smoke_test

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"
)

// Golden hello vector — loaded from the shared cross-language SSOT
// (`crates/spoke-connect/tests/fixtures/golden-hello.json`). `go test` runs
// with the working directory set to this package, so the monorepo-relative
// path resolves through `bindings/go/Smoke → bindings → go → crate root`.
// The fixture carries the seed / nonce / manifest inputs AND the pinned
// output bytes (pubkey, peer id, JCS hex, signature) — asserted below, never
// recomputed and written back.
type goldenFixture struct {
	SeedHex       string         `json:"seed_hex"`
	Nonce         string         `json:"nonce"`
	Manifest      map[string]any `json:"manifest"`
	ManifestJSON  string         `json:"manifest_json"`
	PubkeyHex     string         `json:"pubkey_hex"`
	PeerID        string         `json:"peer_id"`
	JCSHex        string         `json:"jcs_hex"`
	SignatureB64u string         `json:"signature_b64u"`
}

func loadGoldenFixture(t *testing.T) *goldenFixture {
	t.Helper()
	path := filepath.Join("..", "..", "..", "tests", "fixtures", "golden-hello.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read golden SSOT: %v", err)
	}
	var fixture goldenFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parse golden SSOT: %v", err)
	}
	return &fixture
}

func decodeHex(t *testing.T, s string) []byte {
	t.Helper()
	raw, err := hex.DecodeString(s)
	if err != nil {
		t.Fatalf("decode hex %q: %v", s, err)
	}
	return raw
}

func TestGoldenDerivePeerID(t *testing.T) {
	fixture := loadGoldenFixture(t)
	goldenPubkey := decodeHex(t, fixture.PubkeyHex)

	got, err := spokeconnect.DerivePeerIdFromEd25519Pubkey(goldenPubkey)
	if err != nil {
		t.Fatalf("derive_peer_id: %v", err)
	}
	if got != fixture.PeerID {
		t.Fatalf("derive_peer_id: got %q, want %q", got, fixture.PeerID)
	}
}

func TestGoldenSignHelloSignature(t *testing.T) {
	fixture := loadGoldenFixture(t)
	goldenSeed := decodeHex(t, fixture.SeedHex)

	helloJSON, err := spokeconnect.SignHelloEd25519(goldenSeed, fixture.Nonce, fixture.ManifestJSON)
	if err != nil {
		t.Fatalf("sign_hello: %v", err)
	}

	var envelope map[string]any
	if err := json.Unmarshal([]byte(helloJSON), &envelope); err != nil {
		t.Fatalf("sign_hello JSON: %v", err)
	}

	if got, _ := envelope["peer_id"].(string); got != fixture.PeerID {
		t.Fatalf("sign_hello peer_id: got %q, want %q", got, fixture.PeerID)
	}
	if got, _ := envelope["signature"].(string); got != fixture.SignatureB64u {
		t.Fatalf("sign_hello signature: got %q, want %q", got, fixture.SignatureB64u)
	}
}

func TestGoldenVerifyHello(t *testing.T) {
	fixture := loadGoldenFixture(t)
	goldenSeed := decodeHex(t, fixture.SeedHex)
	goldenPubkey := decodeHex(t, fixture.PubkeyHex)

	helloJSON, err := spokeconnect.SignHelloEd25519(goldenSeed, fixture.Nonce, fixture.ManifestJSON)
	if err != nil {
		t.Fatalf("sign_hello: %v", err)
	}
	if err := spokeconnect.VerifyHelloEd25519(goldenPubkey, fixture.PeerID, helloJSON); err != nil {
		t.Fatalf("verify_hello: %v", err)
	}
}

func TestGoldenTamperedHelloRejected(t *testing.T) {
	fixture := loadGoldenFixture(t)
	goldenSeed := decodeHex(t, fixture.SeedHex)
	goldenPubkey := decodeHex(t, fixture.PubkeyHex)

	helloJSON, err := spokeconnect.SignHelloEd25519(goldenSeed, fixture.Nonce, fixture.ManifestJSON)
	if err != nil {
		t.Fatalf("sign_hello: %v", err)
	}
	tampered := strings.Replace(helloJSON, "data-store", "checker", 1)
	err = spokeconnect.VerifyHelloEd25519(goldenPubkey, fixture.PeerID, tampered)
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
