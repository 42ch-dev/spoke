//! Shared golden hello vector loader — **test-only** (`#[cfg(test)]`).
//!
//! The golden hello vector (Ed25519 seed, derived pubkey + peer id, golden
//! nonce, host manifest, pinned RFC 8785 JCS bytes and libp2p-captured
//! signature) lives in `tests/fixtures/golden-hello.json` — the single
//! cross-language source of truth. This crate is the historical libp2p
//! capture authority, so the fixture sits under the crate and tests load it
//! with `include_str!` + `serde_json`.
//!
//! Pinned outputs are **transcribed from committed libp2p-captured
//! constants, never regenerated** by running the code under test. This
//! module is `#[cfg(test)]` and is never linked into non-test builds;
//! production modules must not import the artifact.

use spoke_schemas::connect::connect_hello::HostCapabilityManifest;

/// Raw parsed golden vector (field access is crate-test-facing).
#[derive(Debug, serde::Deserialize)]
pub(crate) struct GoldenHello {
    pub(crate) seed_hex: String,
    pub(crate) nonce: String,
    pub(crate) manifest: serde_json::Value,
    pub(crate) pubkey_hex: String,
    pub(crate) peer_id: String,
    pub(crate) jcs_hex: String,
    pub(crate) signature_b64u: String,
    pub(crate) manifest_json: String,
}

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/golden-hello.json"
));

static GOLDEN: std::sync::OnceLock<GoldenHello> = std::sync::OnceLock::new();

/// The parsed golden vector, parsed once per test process.
pub(crate) fn golden() -> &'static GoldenHello {
    GOLDEN.get_or_init(|| {
        serde_json::from_str(FIXTURE).expect("tests/fixtures/golden-hello.json parses")
    })
}

/// Golden Ed25519 seed (32 bytes, hex-decoded from the fixture).
pub(crate) fn golden_seed() -> [u8; 32] {
    decode_hex(&golden().seed_hex).expect("golden seed_hex is 64 hex chars")
}

/// Golden Ed25519 public key (32 bytes, hex-decoded from the fixture).
pub(crate) fn golden_pubkey() -> [u8; 32] {
    decode_hex(&golden().pubkey_hex).expect("golden pubkey_hex is 64 hex chars")
}

/// Golden host manifest — `authority` absent in the fixture (omitted, never
/// `null`), so `HostCapabilityManifest::authority` deserializes to `None`,
/// exactly matching the JCS-signed bytes in `jcs_hex`.
pub(crate) fn golden_manifest() -> HostCapabilityManifest {
    serde_json::from_value(golden().manifest.clone())
        .expect("golden manifest parses into HostCapabilityManifest")
}

fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}
