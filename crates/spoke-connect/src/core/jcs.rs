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
//!
//! Path matching is prefix-exact: each descent carries only the paths whose
//! components already matched, so an open object (`ErrorEnvelope.details`,
//! `Relation.metadata`) cannot borrow a component from a sibling path.
//!
//! Response owners are per op: a response envelope carries no `op`, so the
//! caller supplies the request op that produced it and only that op's
//! response set applies (own-keyed, empty fallback) — a
//! `knowledge.adopt` response never inherits `payload.deleted` from
//! `history.revisions.prune`. The adoption `details.adopted_revision`
//! exception additionally follows its owning discriminator: only a
//! `REVISION_CONFLICT` branch owns it, so a generic rejection on the same op
//! keeps the `2^53 − 1` bound.
//!
//! The same identity argument covers an integral float: the peer parses
//! `9007199254740992.0` into the same IEEE-754 number as the integer
//! spelling, so a non-owning value at `±2^53` refuses whatever its token,
//! and an owning path admits the endpoint in either spelling.

use serde::de::Error as _;
use serde::Serialize;
use serde_json::Value;

/// Largest integer exactly representable as an f64 (`2^53`) — the
/// inclusive canonicalization bound C5 states (`[0, 2^53]`, no exponent).
pub(crate) const EXACT_INTEGER_BOUND: u64 = 1 << 53;

/// `2^64` as an `f64`: the first magnitude past the integer tokens
/// `serde_json` keeps exact, mirroring the TypeScript endpoint guard.
const U64_RANGE_BOUND: f64 = 18_446_744_073_709_551_616.0;

/// `-2^63` as an `f64`: the low end of the integer token range.
const I64_RANGE_BOUND: f64 = -9_223_372_036_854_775_808.0;

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

/// Response paths that own a revision integer, per op — derived from
/// `schemas/ops/*-response.schema.json` plus the bare entity payloads of the
/// `port.*` reads and writes. A response envelope carries no `op`, so the
/// caller supplies the request op that produced it and only that op's
/// response set applies: no response inherits another op's exception
/// (own-keyed, empty fallback).
const RESPONSE_REVISION_PATHS: &[(&str, &[RevisionPath])] = &[
    (
        "extract",
        &[
            &["payload", "candidates", "adoption_anchor", "source_revision"],
            &["payload", "candidates", "revision"],
        ],
    ),
    (
        "history.revisions.get",
        &[
            &["payload", "snapshot", "revision"],
            &[
                "payload",
                "snapshot",
                "snapshot",
                "adoption_anchor",
                "source_revision",
            ],
            &["payload", "snapshot", "snapshot", "revision"],
        ],
    ),
    (
        "history.revisions.prune",
        &[&["payload", "deleted"], &["payload", "floor_revision"]],
    ),
    (
        "knowledge.adopt",
        &[
            &["payload", "entry", "adoption_anchor", "source_revision"],
            &["payload", "entry", "revision"],
        ],
    ),
    (
        "memory.candidates.get",
        &[&["payload", "candidate", "revision"]],
    ),
    (
        "memory.candidates.list",
        &[&["payload", "candidates", "revision"]],
    ),
    (
        "memory.exports.get",
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
    ("memory.grants.grant", &[&["payload", "result", "revision"]]),
    (
        "memory.grants.list",
        &[&["payload", "grants", "revision"]],
    ),
    (
        "memory.grants.revoke",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.profile.get",
        &[&["payload", "profiles", "revision"]],
    ),
    (
        "memory.profile.update",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.recall",
        &[
            &["payload", "records", "import_ref", "source_revision"],
            &["payload", "records", "revision"],
        ],
    ),
    (
        "memory.receipts.get",
        &[&["payload", "receipt", "result", "revision"]],
    ),
    (
        "memory.records.create",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.records.get",
        &[
            &["payload", "record", "import_ref", "source_revision"],
            &["payload", "record", "revision"],
        ],
    ),
    (
        "memory.records.list",
        &[
            &["payload", "records", "import_ref", "source_revision"],
            &["payload", "records", "revision"],
        ],
    ),
    (
        "memory.records.promote",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.records.update",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.reflections.list",
        &[&["payload", "reflections", "revision"]],
    ),
    (
        "memory.spaces.freeze",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.spaces.list",
        &[&["payload", "spaces", "revision"]],
    ),
    (
        "memory.spaces.unfreeze",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.subjects.ensure",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.subjects.fence",
        &[&["payload", "result", "revision"]],
    ),
    (
        "memory.subjects.get",
        &[&["payload", "subject", "revision"]],
    ),
    (
        "port.knowledge.get",
        &[
            &["payload", "adoption_anchor", "source_revision"],
            &["payload", "revision"],
        ],
    ),
    (
        "port.knowledge.put",
        &[
            &["payload", "adoption_anchor", "source_revision"],
            &["payload", "revision"],
        ],
    ),
    ("port.relation.get", &[&["payload", "revision"]]),
    ("port.relation.put", &[&["payload", "revision"]]),
    (
        "port.scope.list_knowledge_entries",
        &[
            &["payload", "adoption_anchor", "source_revision"],
            &["payload", "revision"],
        ],
    ),
    (
        "promote",
        &[
            &[
                "payload",
                "knowledge_entry",
                "adoption_anchor",
                "source_revision",
            ],
            &["payload", "knowledge_entry", "revision"],
        ],
    ),
    ("relate", &[&["payload", "relation", "revision"]]),
    (
        "upsert",
        &[
            &[
                "payload",
                "knowledge_entries",
                "adoption_anchor",
                "source_revision",
            ],
            &["payload", "knowledge_entries", "revision"],
        ],
    ),
];

/// The adoption refusal vocabulary belongs to `REVISION_CONFLICT` alone
/// (`.agents/specs/protocol/spoke-knowledge-adoption.md` §Refusal
/// vocabulary): `details.adopted_revision` is that code's exact detail, so a
/// generic rejection on the same op owns no revision slot. The two owning
/// response branches are the transport error envelope and the served error
/// branch `KnowledgeAdoptResponse` carries inside `payload`.
const ADOPTION_CONFLICT_CODE: &str = "REVISION_CONFLICT";
const ADOPTION_TRANSPORT_CONFLICT_PATH: RevisionPath =
    &["error", "details", "adopted_revision"];
const ADOPTION_SERVED_CONFLICT_PATH: RevisionPath =
    &["payload", "error", "details", "adopted_revision"];

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
/// request paths, a response takes only the supplying request op's response
/// paths plus the adoption `REVISION_CONFLICT` detail when that branch's
/// discriminator matches, and a session envelope owns no revision member.
fn revision_paths(
    signed: &Value,
    response_op: Option<&str>,
) -> (&'static [RevisionPath], Vec<RevisionPath>) {
    if let Some(op) = signed.get("op").and_then(Value::as_str) {
        return (
            REQUEST_REVISION_PATHS
                .iter()
                .find_map(|(name, paths)| (*name == op).then_some(*paths))
                .unwrap_or(&[]),
            Vec::new(),
        );
    }
    if signed.get("payload").is_some() || signed.get("error").is_some() {
        // A response carries one variant: a transport error at the root or a
        // served `payload.error` owns no success path, so a malformed payload
        // carrying both `entry` and `error` cannot borrow the success set.
        let error_variant = signed.get("error").is_some()
            || signed
                .get("payload")
                .is_some_and(|payload| payload.get("error").is_some());
        let paths = if error_variant {
            &[][..]
        } else {
            response_op
                .and_then(|op| {
                    RESPONSE_REVISION_PATHS
                        .iter()
                        .find_map(|(name, paths)| (*name == op).then_some(*paths))
                })
                .unwrap_or(&[])
        };
        let mut exceptions = Vec::new();
        if response_op == Some("knowledge.adopt") {
            if signed.pointer("/error/code").and_then(Value::as_str)
                == Some(ADOPTION_CONFLICT_CODE)
            {
                exceptions.push(ADOPTION_TRANSPORT_CONFLICT_PATH);
            }
            if signed.pointer("/payload/error/code").and_then(Value::as_str)
                == Some(ADOPTION_CONFLICT_CODE)
            {
                exceptions.push(ADOPTION_SERVED_CONFLICT_PATH);
            }
        }
        return (paths, exceptions);
    }
    (&[], Vec::new())
}

/// Refuse an integral float whose parsed value the peer would classify as an
/// out-of-domain integer: only the `±2^53` endpoint of an owning revision
/// path has a shared IEEE-754 identity, so signing any other such value
/// would emit bytes the peer refuses to verify.
fn float_domain_error(number: &serde_json::Number, revision: bool) -> Option<serde_json::Error> {
    // An integer token keeps its own treatment: exactly `±2^53` re-spells on
    // an owning path and `serde_jcs` refuses the rest past `2^53 − 1`.
    if number.is_u64() || number.is_i64() {
        return None;
    }
    let value = number.as_f64()?;
    let bound = EXACT_INTEGER_BOUND as f64;
    let out_of_domain = (value >= bound && value < U64_RANGE_BOUND)
        || (value <= -bound && value >= I64_RANGE_BOUND);
    if !out_of_domain || (revision && value.abs() == bound) {
        return None;
    }
    Some(serde_json::Error::custom(format!(
        "number {value} is outside the revision member domain of the signed object"
    )))
}

/// Re-spell an integer `±2^53` only at the given root-relative paths
/// (nested members and array elements included), refusing an integral float
/// outside an owning revision path. Every other integer keeps its token, so
/// `serde_jcs` refuses it at the endpoint.
#[cfg(any(test, feature = "remote-adapter"))]
pub(crate) fn respell_paths(
    value: Value,
    paths: &[RevisionPath],
) -> Result<Value, serde_json::Error> {
    respell_paths_with(value, paths, &[])
}

/// [`respell_paths`] over one direction-wide set plus the op-scoped paths a
/// response owns; the extra paths only apply at the root of `value`.
fn respell_paths_with(
    value: Value,
    paths: &[RevisionPath],
    extra: &[RevisionPath],
) -> Result<Value, serde_json::Error> {
    fn walk(
        value: Value,
        paths: &[RevisionPath],
        extra: &[RevisionPath],
        depth: usize,
        revision: bool,
    ) -> Result<Value, serde_json::Error> {
        match value {
            Value::Number(number) => {
                if let Some(error) = float_domain_error(&number, revision) {
                    return Err(error);
                }
                let respelled = revision
                    .then(|| respell_endpoint(&Value::Number(number.clone())))
                    .flatten();
                Ok(respelled.unwrap_or(Value::Number(number)))
            }
            // An array element shares its array's path: positional indexes
            // are not wire members.
            Value::Array(items) => items
                .into_iter()
                .map(|item| walk(item, paths, extra, depth, false))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::Object(members) => {
                let mut walked = Vec::with_capacity(members.len());
                for (key, member) in members {
                    // Carry only the paths whose prefix already matched, so a
                    // sibling path never supplies this member's component.
                    let mut continuations: Vec<RevisionPath> = Vec::new();
                    let mut terminal = false;
                    for path in paths.iter().chain(extra.iter()) {
                        if !matches!(path.get(depth), Some(component) if *component == key.as_str())
                        {
                            continue;
                        }
                        if path.len() == depth + 1 {
                            terminal = true;
                        } else {
                            continuations.push(*path);
                        }
                    }
                    // A member no path owns keeps no path state, but its own
                    // subtree still faces the endpoint refusal.
                    let member = if terminal {
                        walk(member, &[], &[], depth + 1, true)?
                    } else if continuations.is_empty() {
                        walk(member, &[], &[], 0, false)?
                    } else {
                        walk(member, &continuations, &[], depth + 1, false)?
                    };
                    walked.push((key, member));
                }
                Ok(Value::Object(walked.into_iter().collect()))
            }
            other => Ok(other),
        }
    }
    walk(value, paths, extra, 0, false)
}

/// JCS-canonical UTF-8 bytes with the `±2^53` revision endpoint re-spelling
/// at the signed object's owning paths: `serde_json::to_value` (so typed
/// signed objects and opaque payloads share one path) → [`respell_paths`] →
/// `serde_jcs::to_vec`. A request or session envelope carries its own owner
/// set; a response needs the request op that produced it
/// ([`canonical_bytes_with_op`]). Production signing always names the op
/// (requests carry theirs inside the signed object), so the no-op form is
/// test-facing.
#[cfg(test)]
pub(crate) fn canonical_bytes<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    canonical_bytes_with_op(value, None)
}

/// [`canonical_bytes`] with the request op that produced a response envelope
/// (ignored when the signed object carries its own `op`).
pub(crate) fn canonical_bytes_with_op<T>(
    value: &T,
    response_op: Option<&str>,
) -> Result<Vec<u8>, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    let signed = serde_json::to_value(value)?;
    let (paths, extra) = revision_paths(&signed, response_op);
    serde_jcs::to_vec(&respell_paths_with(signed, paths, &extra)?)
}
