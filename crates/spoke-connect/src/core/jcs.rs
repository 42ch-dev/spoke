//! RFC 8785 canonicalization with the revision endpoint re-spelling.
//!
//! `serde_jcs` refuses every integer beyond its `MAX_SAFE_INTEGER`
//! (`2^53 − 1`), but `2^53` is an exact IEEE-754 integer that the
//! TypeScript `canonicalize` reference emits as `9007199254740992`. The
//! signed envelope identity and the service digests must hash the same
//! bytes on both stacks, so an integer `±2^53` carried by a revision-typed
//! member is re-spelled as its equal `f64` before canonicalization.
//!
//! The re-spelling follows the owning wire paths of the payload shape, never
//! member spellings: `Relation.metadata` is an open product bag, so
//! `metadata.revision` and `metadata.counter` keep their integer token
//! (and `serde_jcs` refuses both at the endpoint) while `relation.revision`
//! is a contract-owned revision member and canonicalizes. Every member
//! outside the tables keeps its token, so an integer JavaScript cannot
//! represent (`2^53 + 1`) still refuses instead of being rounded into the
//! signed identity.

use serde::Serialize;
use serde_json::Value;

/// Largest integer exactly representable as an f64 (`2^53`) — the
/// inclusive canonicalization bound C5 states (`[0, 2^53]`, no exponent).
pub(crate) const EXACT_INTEGER_BOUND: u64 = 1 << 53;

/// One root-relative wire path (object keys; arrays are transparent).
pub(crate) type RevisionPath = &'static [&'static str];

/// Request paths that own a revision integer, per op, derived from
/// `schemas/ops/*-request.schema.json` and the `port.*` payload shapes.
const REQUEST_REVISION_PATHS: &[(&str, &[RevisionPath])] = &[
    ("history.revisions.get", &[&["payload", "revision"]]),
    ("history.revisions.prune", &[&["payload", "keep"]]),
    ("knowledge.adopt", &[&["payload", "source_revision"]]),
    (
        "memory.candidates.dismiss",
        &[&["payload", "expected_revision"]],
    ),
    (
        "memory.candidates.review",
        &[&["payload", "expected_revision"]],
    ),
    ("memory.grants.revoke", &[&["payload", "expected_revision"]]),
    (
        "memory.imports.stage",
        &[
            &["payload", "chunk", "candidates", "revision"],
            &["payload", "chunk", "grants", "revision"],
            &["payload", "chunk", "profiles", "revision"],
            &[
                "payload",
                "chunk",
                "records",
                "import_ref",
                "source_revision",
            ],
            &["payload", "chunk", "records", "revision"],
            &["payload", "chunk", "reflections", "revision"],
            &["payload", "chunk", "subjects", "revision"],
        ],
    ),
    (
        "memory.profile.update",
        &[&["payload", "expected_revision"]],
    ),
    (
        "memory.records.delete",
        &[&["payload", "expected_revision"]],
    ),
    (
        "memory.records.promote",
        &[&["payload", "expected_revision"]],
    ),
    (
        "memory.records.update",
        &[&["payload", "expected_revision"]],
    ),
    ("memory.spaces.freeze", &[&["payload", "expected_revision"]]),
    (
        "memory.spaces.unfreeze",
        &[&["payload", "expected_revision"]],
    ),
    (
        "memory.subjects.fence",
        &[&["payload", "expected_revision"]],
    ),
    (
        "port.knowledge.put",
        &[
            &["payload", "entry", "revision"],
            &["payload", "entry", "adoption_anchor", "source_revision"],
            &["payload", "expected_base_revision"],
        ],
    ),
    (
        "port.relation.put",
        &[
            &["payload", "relation", "revision"],
            &["payload", "expected_base_revision"],
        ],
    ),
    (
        "promote",
        &[
            &["payload", "candidate", "revision"],
            &["payload", "candidate", "adoption_anchor", "source_revision"],
        ],
    ),
    ("relate", &[&["payload", "relation", "revision"]]),
    (
        "upsert",
        &[
            &["payload", "knowledge_entries", "revision"],
            &[
                "payload",
                "knowledge_entries",
                "adoption_anchor",
                "source_revision",
            ],
        ],
    ),
];

/// Response paths that own a revision integer — the union over
/// `schemas/ops/*-response.schema.json` plus the bare entity payloads of the
/// `port.*` reads and the adoption refusal vocabulary
/// (`error.details.adopted_revision`, and the served error branch
/// `KnowledgeAdoptResponse` carries inside `payload`). A response envelope
/// does not carry the op, so the direction-wide union is the exact set.
const RESPONSE_REVISION_PATHS: &[RevisionPath] = &[
    &["error", "details", "adopted_revision"],
    &["payload", "adoption_anchor", "source_revision"],
    &["payload", "candidate", "revision"],
    &[
        "payload",
        "candidates",
        "adoption_anchor",
        "source_revision",
    ],
    &["payload", "candidates", "revision"],
    &["payload", "chunk", "candidates", "revision"],
    &["payload", "chunk", "grants", "revision"],
    &["payload", "chunk", "profiles", "revision"],
    &[
        "payload",
        "chunk",
        "records",
        "import_ref",
        "source_revision",
    ],
    &["payload", "chunk", "records", "revision"],
    &["payload", "chunk", "reflections", "revision"],
    &["payload", "chunk", "subjects", "revision"],
    &["payload", "deleted"],
    &["payload", "entry", "adoption_anchor", "source_revision"],
    &["payload", "entry", "revision"],
    &["payload", "error", "details", "adopted_revision"],
    &["payload", "floor_revision"],
    &["payload", "grants", "revision"],
    &[
        "payload",
        "knowledge_entries",
        "adoption_anchor",
        "source_revision",
    ],
    &["payload", "knowledge_entries", "revision"],
    &[
        "payload",
        "knowledge_entry",
        "adoption_anchor",
        "source_revision",
    ],
    &["payload", "knowledge_entry", "revision"],
    &["payload", "profiles", "revision"],
    &["payload", "receipt", "result", "revision"],
    &["payload", "record", "import_ref", "source_revision"],
    &["payload", "record", "revision"],
    &["payload", "records", "import_ref", "source_revision"],
    &["payload", "records", "revision"],
    &["payload", "reflections", "revision"],
    &["payload", "relation", "revision"],
    &["payload", "result", "revision"],
    &["payload", "revision"],
    &["payload", "snapshot", "revision"],
    &[
        "payload",
        "snapshot",
        "snapshot",
        "adoption_anchor",
        "source_revision",
    ],
    &["payload", "snapshot", "snapshot", "revision"],
    &["payload", "spaces", "revision"],
    &["payload", "subject", "revision"],
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

/// The revision paths of one signed object: a request selects its op's
/// request paths, a response (neither carries `op`) takes the direction-wide
/// set, and a session envelope owns no revision member.
fn revision_paths(signed: &Value) -> &'static [RevisionPath] {
    if let Some(op) = signed.get("op").and_then(Value::as_str) {
        return REQUEST_REVISION_PATHS
            .iter()
            .find_map(|(name, paths)| (*name == op).then_some(*paths))
            .unwrap_or(&[]);
    }
    if signed.get("payload").is_some() || signed.get("error").is_some() {
        return RESPONSE_REVISION_PATHS;
    }
    &[]
}

/// Re-spell an integer `±2^53` only at the given root-relative paths
/// (nested members and array elements included). Every other integer keeps
/// its token, so `serde_jcs` refuses it at the endpoint.
pub(crate) fn respell_paths(value: Value, paths: &[RevisionPath]) -> Value {
    fn walk(value: Value, paths: &[RevisionPath], depth: usize, revision: bool) -> Value {
        match value {
            Value::Number(_) if revision => respell_endpoint(&value).unwrap_or(value),
            // An array element shares its array's path: positional indexes
            // are not wire members.
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| walk(item, paths, depth, false))
                    .collect(),
            ),
            Value::Object(members) => Value::Object(
                members
                    .into_iter()
                    .map(|(key, member)| {
                        let (terminal, continues) =
                            paths.iter().fold((false, false), |state, path| {
                                match path.get(depth) {
                                    Some(component) if *component == key.as_str() => {
                                        if path.len() == depth + 1 {
                                            (true, state.1)
                                        } else {
                                            (state.0, true)
                                        }
                                    }
                                    _ => state,
                                }
                            });
                        // No path continues through this key, so no deeper
                        // member can own a revision slot.
                        let member = if terminal || continues {
                            walk(member, paths, depth + 1, terminal)
                        } else {
                            member
                        };
                        (key, member)
                    })
                    .collect(),
            ),
            other => other,
        }
    }
    walk(value, paths, 0, false)
}

/// JCS-canonical UTF-8 bytes with the `±2^53` revision endpoint re-spelling
/// at the signed object's owning paths: `serde_json::to_value` (so typed
/// signed objects and opaque payloads share one path) → [`respell_paths`] →
/// `serde_jcs::to_vec`.
pub(crate) fn canonical_bytes<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    let signed = serde_json::to_value(value)?;
    let paths = revision_paths(&signed);
    serde_jcs::to_vec(&respell_paths(signed, paths))
}
