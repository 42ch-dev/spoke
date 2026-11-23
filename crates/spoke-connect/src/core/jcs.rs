//! RFC 8785 canonicalization with the revision endpoint re-spelling.
//!
//! `serde_jcs` refuses every integer beyond its `MAX_SAFE_INTEGER`
//! (`2^53 − 1`), but `2^53` is an exact IEEE-754 integer that the
//! TypeScript `canonicalize` reference emits as `9007199254740992`. The
//! signed envelope identity and the service digests must hash the same
//! bytes on both stacks, so an integer `±2^53` carried by a revision-typed
//! member is re-spelled as its equal `f64` before canonicalization.
//!
//! Every other member keeps its token: an integer JavaScript would round
//! to a neighbour has no signed identity to share, so it still refuses
//! (`2^53 + 1`) instead of being silently rounded, and a member outside the
//! revision vocabulary has no contract domain at the endpoint — it refuses
//! there too.

use serde::Serialize;
use serde_json::Value;

/// Largest integer exactly representable as an f64 (`2^53`) — the
/// inclusive canonicalization bound C5 states (`[0, 2^53]`, no exponent).
pub(crate) const EXACT_INTEGER_BOUND: u64 = 1 << 53;

/// The revision-typed member vocabulary (spoke-data-model §Revision
/// integers): the published six-member family plus the `spoke-memory`
/// optimistic-concurrency counters, which the schemas also bound at `2^53`.
///
/// shortcut: classification is by member name, not structural path; a
/// product-owned bag key spelled like a revision member takes the same
/// endpoint spelling. Upgrade to op-specific schema paths if a product bag
/// ever carries one of these names at the endpoint.
const REVISION_MEMBERS: [&str; 7] = [
    "revision",
    "deleted",
    "floor_revision",
    "keep",
    "expected_base_revision",
    "source_revision",
    "expected_revision",
];

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

/// Re-spell an integer `±2^53` only where a revision-typed member carries
/// it (nested members and arrays included). Every other integer at the
/// endpoint keeps its token, so `serde_jcs` refuses it.
pub(crate) fn respell_endpoints(value: Value) -> Value {
    fn walk(value: Value, revision_member: bool) -> Value {
        match value {
            Value::Number(_) if revision_member => respell_endpoint(&value).unwrap_or(value),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|item| walk(item, false)).collect())
            }
            Value::Object(members) => Value::Object(
                members
                    .into_iter()
                    .map(|(key, member)| {
                        let revision_member = REVISION_MEMBERS.contains(&key.as_str());
                        (key, walk(member, revision_member))
                    })
                    .collect(),
            ),
            other => other,
        }
    }
    walk(value, false)
}

/// JCS-canonical UTF-8 bytes with the `±2^53` revision endpoint re-spelling:
/// `serde_json::to_value` (so typed signed objects and opaque payloads
/// share one path) → [`respell_endpoints`] → `serde_jcs::to_vec`.
pub(crate) fn canonical_bytes<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    serde_jcs::to_vec(&respell_endpoints(serde_json::to_value(value)?))
}
