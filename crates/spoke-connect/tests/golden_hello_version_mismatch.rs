//! Cross-language golden vector: the golden identity advertising
//! `protocol_version` 2 while the core `PROTOCOL_VERSION` is 1 is a
//! mixed-version hello — both `verify_hello_ed25519` (Rust) and
//! `verifyHelloEd25519` (TS) gate the version FIRST, before signature
//! verification, so the wire hello MUST reject with the dedicated
//! `CoreError::ProtocolVersionMismatch` / `code: "protocol_version_mismatch"`
//! regardless of its (object-stale, never-consulted) pinned signature.
//!
//! The fixture is the SSOT under `crates/spoke-connect/tests/fixtures/`; the
//! TS package carries a byte-identical registered copy
//! (`golden-vector-sync.mjs`) and its
//! `tests/golden-hello-version-mismatch.test.ts` asserts the same outcome on
//! the same bytes — the cross-language parity proof for the shipped
//! version-first gate.

use spoke_connect::core::{verify_hello_ed25519, CoreError};

const FIXTURE: &str = include_str!("fixtures/golden-hello-version-mismatch.json");

/// Decode the fixture's pinned lowercase hex into bytes.
fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("fixture hex is well-formed"))
        .collect()
}

#[test]
fn mixed_version_golden_hello_rejects_with_protocol_version_mismatch() {
    let fixture: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("golden fixture parses as JSON");
    let peer_id = fixture["peer_id"].as_str().expect("fixture peer_id");
    let pubkey: [u8; 32] = decode_hex(fixture["pubkey_hex"].as_str().expect("fixture pubkey_hex"))
        .try_into()
        .expect("pubkey_hex is 32 bytes");
    let hello: spoke_schemas::connect::ConnectHello =
        serde_json::from_value(fixture["hello"].clone()).expect("wire hello parses as ConnectHello");

    // The fixture's `protocol_version` must actually be a mismatch — the
    // golden vector is only meaningful while the core stays at version 1.
    assert_ne!(
        hello.protocol_version.get(),
        spoke_connect::core::PROTOCOL_VERSION,
        "fixture must advertise a non-core protocol_version"
    );

    let error = verify_hello_ed25519(&pubkey, peer_id, &hello, None)
        .expect_err("mixed-version hello must reject");
    assert!(
        matches!(
            error,
            CoreError::ProtocolVersionMismatch { ref reason }
                if reason.contains("unsupported protocol_version 2 (expected 1)")
        ),
        "unexpected error: {error:?}"
    );
}

#[test]
fn fixture_ssot_is_a_schema_conformant_wire_hello() {
    // The SSOT must stay a parseable `ConnectHello` — deserialization above
    // already proves it. This test pins the version-gate premise: the
    // dedicated kind fires on the version alone, before any signature /
    // identity check, so a signature that would not verify for the mutated
    // object is irrelevant (you do not waste crypto on a wrong-version peer).
    let fixture: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("golden fixture parses as JSON");
    let hello: spoke_schemas::connect::ConnectHello =
        serde_json::from_value(fixture["hello"].clone()).expect("wire hello parses");
    assert_eq!(hello.protocol_version.get(), 2);
    assert_eq!(hello.peer_id.as_str(), fixture["peer_id"].as_str().unwrap());
    assert!(hello.nonce.len() >= 16);
}
