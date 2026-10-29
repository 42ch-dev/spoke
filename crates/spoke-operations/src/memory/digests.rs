//! JCS / SHA-256 digest helpers (§6.7 `content_digest`, §6.10
//! `payload_digest`, §6.11 bundle `digest`, §6.13 `chunk_digest`):
//! RFC 8785 canonicalization → SHA-256 → 64 lowercase hex.

use crate::result::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::Value;
use sha2::{Digest, Sha256};

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

fn to_hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX_LOWER[(byte >> 4) as usize] as char);
        out.push(HEX_LOWER[(byte & 0x0f) as usize] as char);
    }
    out
}

/// RFC 8785 cannot represent non-finite floats; serde_jcs 0.2 serializes
/// them silently, so guard the contract ourselves before canonicalizing.
fn contains_non_finite(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64().is_some_and(|n| !n.is_finite()),
        Value::Array(items) => items.iter().any(contains_non_finite),
        Value::Object(entries) => entries.values().any(contains_non_finite),
        _ => false,
    }
}

/// SHA-256 over the RFC 8785 JCS canonicalization of `value`, as 64
/// lowercase hex — the shared digest primitive for `payload_digest`,
/// `content_digest`, `chunk_digest`, and bundle `digest`.
///
/// Rejects with `INVALID_INPUT` when the value cannot be canonicalized
/// (RFC 8785 has no representation for non-finite floats).
pub fn jcs_sha256_hex(value: &Value) -> SpokeResult<String> {
    if contains_non_finite(value) {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "JCS canonicalization failed: non-finite floats are not representable (RFC 8785)",
            None,
        );
    }
    match serde_jcs::to_vec(value) {
        Ok(canonical) => spoke_ok(to_hex_lower(&Sha256::digest(&canonical))),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("JCS canonicalization failed: {error}"),
            None,
        ),
    }
}

/// Digest over `value` with `excluded_field` removed — §6.13 `chunk_digest`
/// (chunk object minus `chunk_digest`) and §6.11 bundle `digest` (bundle
/// object minus `digest`).
pub fn jcs_sha256_hex_excluding(value: &Value, excluded_field: &str) -> SpokeResult<String> {
    let mut stripped = value.clone();
    if let Some(object) = stripped.as_object_mut() {
        object.remove(excluded_field);
    }
    jcs_sha256_hex(&stripped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpokeResult;
    use serde_json::json;

    fn digest_of(value: &serde_json::Value) -> String {
        match jcs_sha256_hex(value) {
            SpokeResult::Ok(digest) => digest,
            SpokeResult::Reject(reject) => panic!("expected digest, got {reject:?}"),
        }
    }

    /// Pinned vectors: JCS bytes follow the RFC 8785 rules by inspection
    /// (sorted keys, minimal numeric forms); SHA-256 pins were produced
    /// independently via `shasum -a 256` over those JCS bytes.
    #[test]
    fn jcs_sha256_known_vectors() {
        let first = json!({"b": 1, "a": [true, null]});
        assert_eq!(
            digest_of(&first),
            "51705a2c9eb3e7e410a58f696a770c3ac3885a0cf43eb7fc88f5e47c11d4d30d"
        );

        let nested = json!({
            "numbers": { "int": 42, "float": 1.5 },
            "literals": [null, true, false],
        });
        assert_eq!(
            digest_of(&nested),
            "fcd9991ad91f6c352701dd3cd819b0633b16e65757773973e7683318c11626f0"
        );
    }

    #[test]
    fn jcs_sha256_sorts_non_bmp_keys_by_utf16_code_units() {
        // RFC 8785 §3.2.3 sorts member names by UTF-16 code units. U+1F600
        // encodes as the surrogate pair D83D DE00, so it orders BEFORE the
        // BMP key U+FF61 — the opposite of Unicode-scalar order (which would
        // put U+FF61 first). The TS test pins the same canonical bytes and
        // digest.
        let value = json!({"😀": 1, "｡": 2});
        let canonical = serde_jcs::to_vec(&value).expect("canonical bytes");
        assert_eq!(canonical, "{\"😀\":1,\"｡\":2}".as_bytes());
        assert_eq!(
            digest_of(&value),
            "88954a1c73177a8dd8c59046f982693fbcfb908d08fbe5b06e82868052487feb"
        );
    }

    #[test]
    fn jcs_sha256_output_is_64_lowercase_hex() {
        let digest = digest_of(&json!({"a": 1}));
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn digest_excluding_field_matches_hand_canonical_form() {
        // §6.13 `chunk_digest`: digest over the chunk object minus
        // `chunk_digest`; §6.11 bundle `digest`: bundle minus `digest`.
        let chunk = json!({"chunk_digest": "deadbeef", "a": 1, "z": 2});
        let digest = match jcs_sha256_hex_excluding(&chunk, "chunk_digest") {
            SpokeResult::Ok(digest) => digest,
            SpokeResult::Reject(reject) => panic!("expected digest, got {reject:?}"),
        };
        assert_eq!(
            digest,
            // shasum -a 256 over '{"a":1,"z":2}'
            "99168216144c7fed5d4c54916cf98d9c66096280c04a499822a99b6658bd177a"
        );
    }
}
