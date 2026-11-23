//! RFC 8785 canonicalization with the `±2^53` endpoint re-spelling.
//!
//! `serde_jcs` refuses every integer beyond its `MAX_SAFE_INTEGER`
//! (`2^53 − 1`), but `2^53` is an exact IEEE-754 integer that the
//! TypeScript `canonicalize` reference emits as `9007199254740992`. The
//! signed envelope identity and the service digests must hash the same
//! bytes on both stacks, so an integer `±2^53` is re-spelled as its equal
//! `f64` before canonicalization.
//!
//! Every other value keeps its token: an integer JavaScript would round
//! to a neighbour has no signed identity to share, so it still refuses
//! (`2^53 + 1`) instead of being silently rounded.

use serde::Serialize;
use serde_json::Value;

/// Largest integer exactly representable as an f64 (`2^53`) — the
/// inclusive canonicalization bound C5 states (`[0, 2^53]`, no exponent).
pub(crate) const EXACT_INTEGER_BOUND: u64 = 1 << 53;

/// Re-spell an integer `±2^53` as its equal `f64`; every other value keeps
/// its token.
pub(crate) fn respell_endpoint(value: &Value) -> Option<Value> {
    let number = value.as_number()?;
    if number.as_u64() == Some(EXACT_INTEGER_BOUND) {
        Some(Value::from(EXACT_INTEGER_BOUND as f64))
    } else if number.as_i64() == Some(-(EXACT_INTEGER_BOUND as i64)) {
        Some(Value::from(-(EXACT_INTEGER_BOUND as f64)))
    } else {
        None
    }
}

/// Re-spell every integer `±2^53` in `value` (nested members and arrays
/// included). The caller must have admitted the endpoint only where the
/// contract allows it — otherwise an unrelated integer is widened.
pub(crate) fn respell_endpoints(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(respell_endpoints).collect()),
        Value::Object(members) => Value::Object(
            members
                .into_iter()
                .map(|(key, member)| (key, respell_endpoints(member)))
                .collect(),
        ),
        other => respell_endpoint(&other).unwrap_or(other),
    }
}

/// JCS-canonical UTF-8 bytes with the `±2^53` endpoint re-spelling:
/// `serde_json::to_value` (so typed signed objects and opaque payloads
/// share one path) → [`respell_endpoints`] → `serde_jcs::to_vec`.
pub(crate) fn canonical_bytes<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    serde_jcs::to_vec(&respell_endpoints(serde_json::to_value(value)?))
}
