//! `connect_responder` — productized connect responder (frozen contract
//! `tool-contracts.md` §6, ported from the demo-server recipe
//! `examples/connect-demo/server/src/host/connect-host.ts`).
//!
//! PUBLIC surface: the [`connect_responder`] factory + the responder
//! object's [`ConnectResponder::register_tool_handler`],
//! [`ConnectResponder::invoke_tool`] (the reverse face), read-only session
//! info ([`ConnectResponder::state`], `session_id`, `remote_peer_id`,
//! `remote_manifest`), and [`ConnectResponder::close`]. Consumers never
//! touch envelope-auth / sequence internals.
//!
//! The responder owns the server end of a message-oriented [`Transport`]:
//!
//! - **handshake**: allowlist fail-closed check FIRST → hello verify
//!   (`peer_id` binding via the preconfigured `peer_keys` pubkey) → nonce
//!   replay record → dial-bound responder hello (5-field signed object
//!   carrying the initiator's nonce as `peer_nonce`) → signed
//!   `ConnectSession` snapshot. Empty-intersection fallback preserved
//!   verbatim from the demo: the wire snapshot requires ≥1 negotiated
//!   capability, so a degenerate empty-intersection dial emits
//!   `["spoke-baseline"]` — the dialer derives its own set from the hellos,
//!   so the fallback has no authorization impact (documented carried-over
//!   behavior).
//! - **serve**: per inbound invoke, a serialized gate (peek → verify →
//!   advance; the async verify cannot interleave with a concurrent invoke)
//!   followed by a concurrent dispatch phase: `tools.*` ops run the
//!   registered tool handler (or deny `op_unsupported`), `port.*` ops run
//!   the D4 catalogue against the injected async
//!   [`RemoteServePorts`] (absent `ports` still answers the dispatch-deny
//!   branch), the core `extract` op runs the optional injected
//!   [`RemoteExtractService`](crate::remote::RemoteExtractService)
//!   (a host that declares `ke-extraction` without providing the service
//!   takes the same dispatch-deny branch), `memory.*` ops run the optional
//!   injected [`RemoteMemoryService`](crate::remote::RemoteMemoryService)
//!   under the same probe rule — a known memory op that was not negotiated
//!   is denied `capability_missing` at the gate, before any probe — and
//!   unknown ops are denied. A Scope-bearing `port.*` request additionally
//!   passes the supplementary `ke-ownership` gate and Scope validation
//!   before the provider is probed.
//!   Every served `port.*` provider call and the whole `extract` /
//!   `memory.*` service call are bounded by the responder's local serve-wait
//!   budget (`invoke_timeout_ms`, default 5000): expiry answers exactly one signed
//!   `INTERNAL_ERROR` with `details.kind = "timeout"` and leaves the session
//!   Established (bounded wait, not forced host-work termination). The bound
//!   starts after the payload validation, covers the knowledge face's
//!   per-peer admission (run on the blocking pool — a blocking resolver
//!   cannot be cancelled, so the bound, not a forced stop, is what the
//!   caller is guaranteed), and a close aborts the in-flight serve
//!   dispatches so no serve wait or its timer outlives the session. A
//!   failed envelope-auth verify
//!   produces no handler side effect and no session-state mutation
//!   (auth-before-advance, spec §Verify rules). An unparseable inbound
//!   frame closes the connection (carried over from the demo).
//! - **invoke_tool**: the reverse face — outbound counter, request signing,
//!   send-tail wire-order serialization, response correlation +
//!   envelope-auth verify, per-waiter timeout, and the deferred-send
//!   poison-close mirror (a waiter that settles while its send is still
//!   queued means the allocated outbound sequence never hit the wire —
//!   close the session, same semantics as the adapter's `invoke_op`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::FutureExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use spoke_operations::{
    assert_adoption_anchor_preserved, from_error_envelope, parse_tool_capability_id, spoke_ok,
    spoke_reject, to_error_envelope, BaselinePorts, ScopedKnowledgePort, SpokeReject,
    SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_response::ConnectInvokeResponse;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::{
    ComputeRequest, ExtractRequest, ExtractResponse, Finding, HostCapabilityManifest,
    KnowledgeEntry, ProjectRequest, Relation, Scope,
};

use crate::cognition::{unsupported_cognition_requirement, WIRE_INVALID_INPUT};
use crate::core::{
    authenticate_invoke_request, authenticate_invoke_response_with_op, authenticate_session,
    check_response_correlation, cognition_requirement_present, derive_peer_id_from_ed25519_pubkey,
    is_allowlisted, required_capability, sign_hello_ed25519, verify_hello_ed25519,
    verify_invoke_request_auth, verify_invoke_response_auth_with_op, Correlation,
    EnvelopeAuthError, InboundSequence, InvokeRequestSignInput, InvokeResponseSignInput,
    NonceStore, OutboundSequence, SessionSignInput, CAPABILITY_KE_OWNERSHIP,
    CAPABILITY_SPOKE_COGNITION, CAPABILITY_SPOKE_KNOWLEDGE_SCOPED,
};
use crate::hello::generate_nonce;
use crate::remote::remote_adapter::{
    connect_manifest, internal_error, is_connect_invoke_request, map_error_envelope,
    wire_response_correlation, RemoteError as ResponderError,
    RemoteErrorKind as ResponderErrorKind, RemoteIdentity, ToolHandler,
};
use crate::remote::serve_ports::{
    exact_revision_u64, schema_versions_in_domain, RemoteServePorts,
};
use crate::remote::transport::Transport;
use crate::remote::{
    baseline_scope_shape, optional_scope_op, port_dispatch_allowed,
    requires_ownership_capability, validate_scope_declaration,
};
use crate::runtime::generate_request_id;

/// Default bounded-wait deadline for each reverse-invoke waiter AND the
/// default local serve-wait budget for served provider calls, ms (parity
/// with TS `DEFAULT_INVOKE_TIMEOUT_MS`).
const DEFAULT_INVOKE_TIMEOUT_MS: u64 = 5000;

/// Session id prefix — deterministic per dialing peer (opaque, not
/// schema-enforced; parity with TS `SESSION_ID_PREFIX`).
const SESSION_ID_PREFIX: &str = "connect-responder-session-";

/// Session-lifecycle labels (frozen contract §4 labels; identical to
/// [`crate::remote::RemoteAdapterState`] — one state vocabulary).
pub type ConnectResponderState = crate::remote::RemoteAdapterState;

/// The agreed capability subset of the two signed hellos: the plain
/// intersection in local-manifest order (mirror of the TS
/// `negotiatedCapabilities`). One family, no aliasing — a legacy flag carries
/// no authority here, so `spoke-cognition` appears only when both sides
/// declare it.
fn negotiated_capabilities(local: &[String], remote: &[String]) -> Vec<String> {
    local
        .iter()
        .filter(|cap| remote.iter().any(|advertised| advertised == *cap))
        .cloned()
        .collect()
}

/// Optional-family op → required face probe (D4 catalogue). Serving order
/// is gate → probe → serve/deny: after the capability gate passes, the
/// injected ports face is structurally probed via
/// [`RemoteServePorts::as_computable`] / [`RemoteServePorts::as_fork_timeline`];
/// a `None` probe (returned as the missing method name) means the host
/// declared the family but does not provide it (host misconfiguration) —
/// answer the same dispatch-deny branch as absent ports. Baseline ops are
/// not probed: the `BaselinePorts` supertrait guarantees their methods.
fn missing_optional_serve_face(
    op: &str,
    ports: &(dyn RemoteServePorts + Send + Sync),
) -> Option<&'static str> {
    match op {
        "port.computable.project" if ports.as_computable().is_none() => Some("project"),
        "port.computable.compute" if ports.as_computable().is_none() => Some("compute"),
        "port.fork.list_timeline_events" if ports.as_fork_timeline().is_none() => {
            Some("list_fork_timeline_events")
        }
        _ => None,
    }
}

/// Safety net for a baseline arm reached with a `None` baseline: the
/// responder probes the owning face per op before the bound, so this only
/// fires for a caller that bypassed that probe.
fn missing_baseline_face() -> SpokeReject {
    SpokeReject {
        code: SpokeRejectCode::CapabilityPortMissing,
        message: "no baseline face configured".to_owned(),
        details: None,
    }
}

/// Deserialize one opaque payload field (defensive — the dispatch gate
/// already ran; a malformed payload answers `INVALID_INPUT`).
fn payload_field<T: DeserializeOwned>(payload: &Value, field: &str, op: &str) -> SpokeResult<T> {
    let Some(value) = payload.get(field) else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("invalid {op} payload: missing {field}"),
            None,
        );
    };
    // The generated `SchemaVersion` carriers are unbounded `NonZeroU64`;
    // every `schema_version` member must also satisfy the published
    // `[1, 2^53-1]` domain before the typed decode.
    if !schema_versions_in_domain(value) {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!(
                "invalid {op} payload: {field} carries a schema_version outside [1, 2^53-1]"
            ),
            None,
        );
    }
    match serde_json::from_value(value.clone()) {
        Ok(value) => spoke_ok(value),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("invalid {op} payload: {error}"),
            None,
        ),
    }
}

/// Safety net for a `SCOPE_BEARING_OPS` arm reached without the `Scope` the
/// responder gate decodes: the gate rejects a missing or malformed declared
/// Scope before dispatch, so this only fires for a caller that bypassed it.
fn missing_declared_scope(op: &str) -> SpokeReject {
    SpokeReject {
        code: SpokeRejectCode::InvalidInput,
        message: format!("invalid {op} payload: missing scope"),
        details: None,
    }
}

/// One refusal message for every knowledge-scope deny (unbound peer,
/// rejecting admission, mismatched list scope): the caller learns only that
/// the scope is unauthorized.
const KNOWLEDGE_SCOPE_NOT_AUTHORIZED: &str = "knowledge scope not authorized";

/// Signed serve-wait expiry (D7/D14): the existing `INTERNAL_ERROR`
/// application-reject map with the existing `details.kind = "timeout"`
/// vocabulary — the same projection the reverse-invoke waiter timeout uses.
/// The boundary is a bounded wait only: it neither terminates host work nor
/// touches the session (no counter rollback, no capability mutation, no
/// forced close), so the caller can keep using the session.
fn serve_timeout_reject(budget: Duration) -> SpokeReject {
    let mut details = Map::new();
    details.insert("kind".into(), Value::String("timeout".into()));
    SpokeReject {
        code: SpokeRejectCode::InternalError,
        message: format!(
            "serve wait exceeded the local budget of {}ms",
            budget.as_millis()
        ),
        details: Some(details),
    }
}

/// §14.2 memory-boundary deny details: every dispatch-layer denial is
/// `terminal` — retrying the same request unchanged fails again.
fn memory_terminal_details() -> Map<String, Value> {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("terminal".into()));
    details
}

/// §14.2/§14.3: the serve budget expired AFTER the memory service was
/// invoked, so the transport has no evidence that nothing committed — the
/// outcome is unknown. The response says so instead of claiming a clean
/// failure; provider work is neither cancelled nor rolled back by this end.
fn memory_uncertain_reject(budget: Duration) -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("timeout".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: format!(
            "memory serve wait exceeded the local budget of {}ms; the outcome is unknown — poll the receipt or replay the same idempotency key",
            budget.as_millis()
        ),
        details: Some(details),
    }
}

/// §14.2/§14.3, knowledge face: the serve budget expired AFTER the store call
/// was invoked, so the transport has no evidence that nothing committed — a
/// create-style put (no `expected_base_revision` fence) may have applied. The
/// response says the outcome is unknown instead of claiming a clean failure;
/// the store work is neither cancelled nor rolled back by this end. Reads keep
/// the plain serve timeout (no store effect to be uncertain about).
fn knowledge_uncertain_reject(budget: Duration) -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("timeout".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: format!(
            "knowledge serve wait exceeded the local budget of {}ms; the outcome is unknown — re-read the entry to observe the committed revision before replaying the same request",
            budget.as_millis()
        ),
        details: Some(details),
    }
}

/// §14.2/§14.3: the detached memory invoke panicked (or otherwise dropped
/// its result channel) AFTER admission, so the transport has no evidence
/// that nothing committed — a clean `INTERNAL_ERROR` would assert no-effect.
/// The response says the outcome is unknown instead.
fn memory_panic_uncertain_reject() -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("panic".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: "memory service failed after invocation; the outcome is unknown — poll the receipt or replay the same idempotency key"
            .to_string(),
        details: Some(details),
    }
}

/// One decoded knowledge op: the catalogue arm's validated arguments,
/// produced before the serve bound starts. The three knowledge ops are
/// served through the knowledge face, never through the baseline provider.
enum KnowledgeCall {
    GetKnowledgeEntry(String),
    PutKnowledgeEntry(KnowledgeEntry, Option<u64>),
    ListKnowledgeEntries,
}

/// The knowledge face a host provides: the request-scoped face probed first
/// (A1-D5), or the peer-only `0.18.1` face it falls back to. Both serve the
/// same store; only the admission signature differs.
enum KnowledgeServingFace {
    Scoped(Arc<crate::remote::RemoteScopedKnowledgeFace>),
    PeerOnly(Arc<crate::remote::RemoteKnowledgeFace>),
}

impl KnowledgeServingFace {
    fn store(&self) -> Arc<dyn ScopedKnowledgePort + Send + Sync> {
        match self {
            Self::Scoped(face) => Arc::clone(&face.store),
            Self::PeerOnly(face) => Arc::clone(&face.store),
        }
    }
}

/// One decoded `port.*` dispatch: the catalogue arm's validated arguments,
/// produced by [`decode_port_call`] before the serve bound starts (B1 — a
/// malformed payload keeps its `INVALID_INPUT` reject even at a zero budget).
enum PortCall<'a> {
    GetRelation(String),
    PutRelation(Relation, Option<u64>),
    ListTimelineEvents(&'a Scope),
    PutFindings(Vec<Finding>),
    ListRules(Vec<String>),
    ListPeerManifests,
    Project(ProjectRequest),
    Compute(ComputeRequest),
    ForkTimelineEvents(&'a Scope),
}

/// The three remote knowledge ops, served only through
/// [`RemoteServePorts::as_knowledge`](crate::remote::RemoteServePorts::as_knowledge).
fn is_knowledge_op(op: &str) -> bool {
    matches!(
        op,
        "port.knowledge.get" | "port.knowledge.put" | "port.scope.list_knowledge_entries"
    )
}

/// Decode and validate one knowledge op payload. Get and put accept an
/// optional declared scope (validated by the responder gate, applied after
/// admission); the list op's declared scope is required and is the one the
/// gate already decoded and validated.
fn decode_knowledge_call(
    op: &str,
    payload: &Value,
    declared_scope: Option<&Scope>,
) -> SpokeResult<KnowledgeCall> {
    match op {
        // TS twin (`validateKnowledgeOpPayload`) rejects a non-string or empty
        // `entry_id` with INVALID_INPUT before admission; the store never sees
        // an empty id.
        "port.knowledge.get" => match payload_field::<Value>(payload, "entry_id", op) {
            SpokeResult::Ok(Value::String(id)) if !id.is_empty() => {
                spoke_ok(KnowledgeCall::GetKnowledgeEntry(id))
            }
            _ => spoke_reject(
                SpokeRejectCode::InvalidInput,
                "invalid port.knowledge.get payload",
                None,
            ),
        },
        "port.knowledge.put" => {
            // Scoped-put wire-wide precedence (contract D1 / qc3 F-016):
            // request validation answers `INVALID_INPUT` first —
            // regardless of the stored state. An explicit-null
            // `destination_entry_id` is malformed, and the typed
            // `Option` decoder would collapse a present-null member to
            // `None`, so both null checks run on the RAW payload before
            // decode (TS parity: `validateKnowledgeOpPayload` /
            // `isValidKnowledgeEntryDeep`).
            if payload
                .get("destination_entry_id")
                .is_some_and(Value::is_null)
            {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    "invalid port.knowledge.put payload: destination_entry_id must be a string or absent, not null",
                    None,
                );
            }
            if let Some(members) = payload.get("entry").and_then(Value::as_object) {
                // Every schema-optional member whose declared kind is not
                // nullable: a present `null` is malformed, not absent —
                // the `Option` decode must not collapse it (TS parity:
                // `matchesOptionalFields` / `isValidKnowledgeEntryDeep`).
                // (`modules` is an `Option<ModuleMap>` too.)
                const NULLABLE_OPT_NONE: &[&str] = &[
                    "source_anchor",
                    "adoption_anchor",
                    "revision",
                    "created_at",
                    "updated_at",
                    "modules",
                    "owner",
                    "disclosure",
                ];
                if let Some(null_member) = NULLABLE_OPT_NONE
                    .iter()
                    .find(|name| members.get(**name).is_some_and(Value::is_null))
                {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        format!("invalid port.knowledge.put payload: entry.{null_member} must not be null"),
                        None,
                    );
                }
                // F-018 residual: the same `Option`-null collapse hides
                // inside the nested declared objects — every declared
                // member of `body` / `source_anchor` / `source_anchor.span`
                // / `body.attributes[]` is non-nullable on the wire, so an
                // explicit null anywhere under them is malformed (none of
                // their members carries a nullable type; extension bags
                // nested inside keep opaque `null` values legal).
                for (parent, keys) in [
                    ("body", &["summary", "tags", "attributes", "state", "computable"][..]),
                    (
                        "source_anchor",
                        &[
                            "schema_version",
                            "source_id",
                            "span",
                            "label",
                            "mime_type",
                            "extensions",
                        ][..],
                    ),
                    (
                        "adoption_anchor",
                        &[
                            "schema_version",
                            "source_scope_id",
                            "source_entry_id",
                            "source_revision",
                            "extensions",
                        ][..],
                    ),
                ] {
                    if let Some(nested) = members.get(parent).and_then(Value::as_object) {
                        if let Some(null_member) = keys
                            .iter()
                            .find(|name| nested.get(**name).is_some_and(Value::is_null))
                        {
                            return spoke_reject(
                                SpokeRejectCode::InvalidInput,
                                format!("invalid port.knowledge.put payload: entry.{parent}.{null_member} must not be null"),
                                None,
                            );
                        }
                    }
                }
                if let Some(span) = members
                    .get("source_anchor")
                    .and_then(Value::as_object)
                    .and_then(|anchor| anchor.get("span"))
                    .and_then(Value::as_object)
                {
                    if let Some(null_member) = ["start", "end"]
                        .iter()
                        .find(|name| span.get(**name).is_some_and(Value::is_null))
                    {
                        return spoke_reject(
                            SpokeRejectCode::InvalidInput,
                            format!("invalid port.knowledge.put payload: entry.source_anchor.span.{null_member} must not be null"),
                            None,
                        );
                    }
                }
                if let Some(attributes) = members
                    .get("body")
                    .and_then(Value::as_object)
                    .and_then(|body| body.get("attributes"))
                    .and_then(Value::as_array)
                {
                    for attribute in attributes {
                        if let Some(attribute) = attribute.as_object() {
                            if let Some(null_member) =
                                ["trait_type", "value", "display_type", "max_value"]
                                    .iter()
                                    .find(|name| {
                                        attribute.get(**name).is_some_and(Value::is_null)
                                    })
                            {
                                return spoke_reject(
                                    SpokeRejectCode::InvalidInput,
                                    format!("invalid port.knowledge.put payload: entry.body.attributes[].{null_member} must not be null"),
                                    None,
                                );
                            }
                        }
                    }
                }
            }

            // Contract E1 — revision-typed members live in the
            // interoperable integer domain `[0, 2^53]` on the wire: the
            // typed `u64` decoders would accept beyond that, so the
            // bound runs on the RAW payload (TS parity:
            // `isAdoptExactNonNegativeInteger` inside
            // `isValidKnowledgeEntryDeep` / `validateKnowledgeOpPayload`).
            // The bound is compared on the exact integer representation
            // (qc3-05): an exact `9007199254740993` refuses instead of
            // rounding into `2^53`, while an integral-float spelling is
            // admitted by its parsed value.
            fn adopt_domain(value: Option<&Value>) -> bool {
                match value {
                    None | Some(Value::Null) => true,
                    Some(value) => exact_revision_u64(value).is_some(),
                }
            }
            if let Some(members) = payload.get("entry").and_then(Value::as_object) {
                if !adopt_domain(members.get("revision")) {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        "invalid port.knowledge.put payload: entry.revision must be an integer within [0, 2^53]",
                        None,
                    );
                }
                if !adopt_domain(
                    members
                        .get("adoption_anchor")
                        .and_then(Value::as_object)
                        .and_then(|anchor| anchor.get("source_revision")),
                ) {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        "invalid port.knowledge.put payload: entry.adoption_anchor.source_revision must be an integer within [0, 2^53]",
                        None,
                    );
                }
            }
            if !adopt_domain(payload.get("expected_base_revision")) {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    "invalid port.knowledge.put payload: expected_base_revision must be an integer within [0, 2^53] or null",
                    None,
                );
            }
            let entry = match payload_field::<KnowledgeEntry>(payload, "entry", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            // The raw `Option<u64>` binding refuses an integral-float
            // spelling; the member decodes through its exact integer
            // representation like `relation.put` does (qc3-05).
            let expected = match payload.get("expected_base_revision") {
                Some(Value::Null) => None,
                Some(value) => match exact_revision_u64(value) {
                    Some(revision) => Some(revision),
                    None => {
                        return spoke_reject(
                            SpokeRejectCode::InvalidInput,
                            "invalid port.knowledge.put payload: expected_base_revision must be an integer within [0, 2^53] or null",
                            None,
                        )
                    }
                },
                None => {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        "invalid port.knowledge.put payload: missing expected_base_revision",
                        None,
                    )
                }
            };
            spoke_ok(KnowledgeCall::PutKnowledgeEntry(entry, expected))
        }
        "port.scope.list_knowledge_entries" => match declared_scope {
            Some(_) => spoke_ok(KnowledgeCall::ListKnowledgeEntries),
            None => SpokeResult::Reject(missing_declared_scope(op)),
        },
        _ => SpokeResult::Reject(SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: format!("unimplemented knowledge op {op}"),
            details: None,
        }),
    }
}

/// Scope equality between the requested and the admitted scope: the same
/// `scope_id` AND the same `viewpoint` (both absent or equal). A mismatch —
/// including an admitted viewpoint the request omits — refuses.
fn scope_matches_admission(requested: &Scope, admitted: &Scope) -> bool {
    requested.scope_id == admitted.scope_id && requested.viewpoint == admitted.viewpoint
}

/// Adopt one admitted scalar constraint: the admitted value when the request
/// omits or repeats it. An outer `None` means the request named a different
/// value — a widening, not a narrowing.
fn adopted_scalar<T: Clone + PartialEq>(
    admitted: &Option<T>,
    requested: &Option<T>,
) -> Option<Option<T>> {
    match (admitted, requested) {
        (Some(admitted), Some(requested)) if admitted != requested => None,
        (Some(admitted), _) => Some(Some(admitted.clone())),
        (None, requested) => Some(requested.clone()),
    }
}

/// Admitted ∩ requested ids: the request only narrows, and an empty
/// intersection stays an empty filter (no results), never the admitted set.
fn intersected_ids(
    admitted: &Option<Vec<String>>,
    requested: &Option<Vec<String>>,
) -> Option<Vec<String>> {
    match (admitted, requested) {
        (Some(admitted), Some(requested)) => Some(
            requested
                .iter()
                .filter(|id| admitted.contains(id))
                .cloned()
                .collect(),
        ),
        (Some(admitted), None) => Some(admitted.clone()),
        (None, requested) => requested.clone(),
    }
}

/// The scope passed to the store for a list: the admitted `scope_id` /
/// `viewpoint` are authoritative (equal to the request — the caller checked)
/// and every other admitted constraint bounds the request. `None` refuses: the
/// request named a scalar value the admission does not grant, and honouring it
/// would widen the query. Id lists intersect; admitted `extensions` namespaces
/// win on conflict; the request's cognition selection is untouched.
fn effective_scope(requested: &Scope, admitted: &Scope) -> Option<Scope> {
    let entry_types = match (&admitted.entry_types, &requested.entry_types) {
        // The request narrows the admitted set: any value outside it would
        // widen the query, so only a subset is served (as the request's set).
        (Some(admitted), Some(requested))
            if !requested
                .iter()
                .all(|entry_type| admitted.contains(entry_type)) =>
        {
            return None;
        }
        (admitted, requested) => requested.clone().or_else(|| admitted.clone()),
    };
    let source_id = adopted_scalar(&admitted.source_id, &requested.source_id)?;
    let timeline_scale = adopted_scalar(&admitted.timeline_scale, &requested.timeline_scale)?;
    let fork_id = adopted_scalar(&admitted.fork_id, &requested.fork_id)?;
    let mut scope = requested.clone();
    scope.scope_id = admitted.scope_id.clone();
    scope.viewpoint = admitted.viewpoint.clone();
    scope.entry_ids = intersected_ids(&admitted.entry_ids, &requested.entry_ids);
    scope.timeline_event_ids =
        intersected_ids(&admitted.timeline_event_ids, &requested.timeline_event_ids);
    scope.entry_types = entry_types;
    scope.source_id = source_id;
    scope.timeline_scale = timeline_scale;
    scope.fork_id = fork_id;
    scope.extensions = match (&requested.extensions, &admitted.extensions) {
        (None, admitted_extensions) => admitted_extensions.clone(),
        (Some(requested_extensions), None) => Some(requested_extensions.clone()),
        (Some(requested_extensions), Some(admitted_extensions)) => {
            let mut merged = requested_extensions.clone();
            for (namespace, value) in admitted_extensions.iter() {
                merged.0.insert(namespace.clone(), value.clone());
            }
            Some(merged)
        }
    };
    Some(scope)
}

/// Serve one decoded [`KnowledgeCall`] against the served scope. Runs
/// inside the serve bound; admission and the declared-scope intersection ran
/// before it, so a refusal never reaches the store and `list_scope` is
/// present exactly when the call lists.
async fn call_knowledge_op(
    call: KnowledgeCall,
    store: &(dyn ScopedKnowledgePort + Send + Sync),
    served_scope: &Scope,
    list_scope: Option<&Scope>,
) -> SpokeResult<Value> {
    match call {
        KnowledgeCall::GetKnowledgeEntry(entry_id) => map_result(
            store
                .get_scoped_knowledge_entry(served_scope, &entry_id)
                .await,
        ),
        KnowledgeCall::PutKnowledgeEntry(entry, expected_base_revision) => {
            // The scoped put path serving `port.knowledge.put` owns the
            // same anchor-immutability obligation as the upsert/promote
            // orchestrations (spoke-knowledge-adoption contract §Anchor
            // immutability): a stored `adoption_anchor` is preserved
            // byte-identically or the write refuses
            // `ADOPTION_ANCHOR_IMMUTABLE` with `details.entry_id` —
            // before the put, never after.
            let stored = match store
                .get_scoped_knowledge_entry(served_scope, &entry.entry_id)
                .await
            {
                SpokeResult::Ok(stored) => Some(stored),
                // A missing destination is a create — no anchor to
                // preserve. `KNOWLEDGE_ENTRY_NOT_FOUND` is the scoped
                // store's established entry-specific absence code; a
                // generic `NOT_FOUND` means the same.
                SpokeResult::Reject(reject)
                    if matches!(
                        reject.code,
                        SpokeRejectCode::NotFound | SpokeRejectCode::KnowledgeEntryNotFound
                    ) =>
                {
                    None
                }
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            // Caller OCC: the wire-supplied `expected_base_revision` is
            // the CAS fence, evaluated against the SAME pre-read
            // snapshot the anchor check inspects. `None` is create-only
            // — it refuses an occupied slot; `Some(base)` must equal the
            // stored revision — a stale base or an update on an absent
            // entry refuses before any write. The refusal shapes mirror
            // the baseline `put_knowledge_entry` OCC vocabulary.
            match (expected_base_revision, stored.as_ref()) {
                (None, Some(_)) => {
                    let mut details = Map::new();
                    details.insert("entry_id".into(), Value::String(entry.entry_id.clone()));
                    return spoke_reject(
                        SpokeRejectCode::RevisionConflict,
                        format!("Entry already exists: {}", entry.entry_id),
                        Some(details),
                    );
                }
                (Some(expected), existing) => {
                    let current = existing.and_then(|stored| stored.revision).unwrap_or(0);
                    let absent = existing.is_none();
                    if absent || current != expected {
                        let mut details = Map::new();
                        details.insert("entry_id".into(), Value::String(entry.entry_id.clone()));
                        details.insert("expectedBaseRevision".into(), json!(expected));
                        if !absent {
                            details.insert("storeRevision".into(), json!(current));
                        }
                        return spoke_reject(
                            SpokeRejectCode::StoredRevisionStale,
                            if absent {
                                format!("KnowledgeEntry not found for update: {}", entry.entry_id)
                            } else {
                                format!(
                                    "Store revision {current} does not match expected base {expected}"
                                )
                            },
                            Some(details),
                        );
                    }
                }
                (None, None) => {}
            }
            if let SpokeResult::Reject(reject) =
                assert_adoption_anchor_preserved(&entry, stored.as_ref())
            {
                return SpokeResult::Reject(reject);
            }
            // §Anchor immutability's CAS binding: the write carries the
            // caller base, which now equals the inspected snapshot —
            // a verified `Some` is the stored revision itself and a
            // `None` fence stays a create-only CAS against absence.
            map_result(
                store
                    .put_scoped_knowledge_entry(served_scope, entry, expected_base_revision)
                    .await,
            )
        }
        KnowledgeCall::ListKnowledgeEntries => {
            let scope = list_scope.expect("a list call carries its effective scope");
            map_result(store.list_scoped_knowledge_entries(scope).await)
        }
    }
}

/// Decode and validate a baseline `port.*` payload per the D4 catalogue —
/// the validation the serve bound must start AFTER (B1). The dispatch gate
/// (capability check), the optional-face probe (gate → probe → serve/deny)
/// and the payload-dependent gates have already run when this is called;
/// unknown ops reject `CAPABILITY_PORT_MISSING` as a safety net for host
/// misconfiguration (the gate denies them first). The three knowledge ops
/// are decoded by [`decode_knowledge_call`] instead.
///
/// `declared_scope` is the declared Scope the responder gate already decoded
/// and validated for the two remaining Scope-bearing ops — those arms carry
/// that value instead of decoding the same JSON again.
fn decode_port_call<'a>(
    op: &str,
    payload: &Value,
    declared_scope: Option<&'a Scope>,
) -> SpokeResult<PortCall<'a>> {
    let field = |name: &str| payload_field::<Value>(payload, name, op);
    match op {
        "port.relation.get" => match field("relation_id") {
            SpokeResult::Ok(Value::String(id)) => spoke_ok(PortCall::GetRelation(id)),
            _ => spoke_reject(
                SpokeRejectCode::InvalidInput,
                "invalid port.relation.get payload",
                None,
            ),
        },
        "port.relation.put" => {
            let relation = match payload_field::<Relation>(payload, "relation", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            // Contract E1: revision-typed members live in the `[0, 2^53]`
            // domain; the generated entity carries an unbounded `u64`, so
            // the bound runs here before the provider (qc2 Important 1 /
            // qc3-05) — an exact `9007199254740994` must refuse, never be
            // flushed to the port.
            if relation
                .revision
                .is_some_and(|revision| revision > crate::core::jcs::EXACT_INTEGER_BOUND)
            {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    "invalid port.relation.put payload: relation.revision must be an integer within [0, 2^53]",
                    None,
                );
            }
            // Contract E1: `expected_base_revision` is revision-typed
            // (`[0, 2^53]`) or null. The revision is decoded from the raw
            // member through its exact integer representation, so an
            // integral float spelling (`1.0`) names the same base instead
            // of failing the typed `u64` decode.
            let expected = match payload.get("expected_base_revision") {
                Some(Value::Null) => None,
                Some(value) => match exact_revision_u64(value) {
                    Some(revision) => Some(revision),
                    None => {
                        return spoke_reject(
                            SpokeRejectCode::InvalidInput,
                            "invalid port.relation.put payload: expected_base_revision must be an integer within [0, 2^53] or null",
                            None,
                        )
                    }
                },
                None => {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        "invalid port.relation.put payload: missing expected_base_revision",
                        None,
                    )
                }
            };
            spoke_ok(PortCall::PutRelation(relation, expected))
        }
        "port.scope.list_timeline_events" => match declared_scope {
            Some(scope) => spoke_ok(PortCall::ListTimelineEvents(scope)),
            None => SpokeResult::Reject(missing_declared_scope(op)),
        },
        "port.finding.put" => match payload_field::<Vec<Finding>>(payload, "findings", op) {
            SpokeResult::Ok(findings) => spoke_ok(PortCall::PutFindings(findings)),
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        },
        "port.rule.list" => match payload_field::<Vec<String>>(payload, "rule_refs", op) {
            SpokeResult::Ok(rule_refs) => spoke_ok(PortCall::ListRules(rule_refs)),
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        },
        "port.host.list_peer_manifests" => spoke_ok(PortCall::ListPeerManifests),
        "port.computable.project" => {
            match serde_json::from_value::<ProjectRequest>(payload.clone()) {
                Ok(request) => spoke_ok(PortCall::Project(request)),
                Err(error) => spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("invalid port.computable.project payload: {error}"),
                    None,
                ),
            }
        }
        "port.computable.compute" => {
            match serde_json::from_value::<ComputeRequest>(payload.clone()) {
                Ok(request) => spoke_ok(PortCall::Compute(request)),
                Err(error) => spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("invalid port.computable.compute payload: {error}"),
                    None,
                ),
            }
        }
        "port.fork.list_timeline_events" => match declared_scope {
            Some(scope) => spoke_ok(PortCall::ForkTimelineEvents(scope)),
            None => SpokeResult::Reject(missing_declared_scope(op)),
        },
        _ => SpokeResult::Reject(SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: format!("unimplemented port op {op}"),
            details: None,
        }),
    }
}

/// Dispatch one decoded [`PortCall`] to the owning injected face — the
/// provider work the serve bound covers, because the bound starts here, after
/// validation and probing (B1). The optional families own their ops and never
/// look at the baseline face (B5); a `None` baseline on a baseline arm is the
/// unreachable safety net for the responder's per-op probe.
async fn call_port_op(
    call: PortCall<'_>,
    baseline: Option<&(dyn BaselinePorts + Send + Sync)>,
    ports: &(dyn RemoteServePorts + Send + Sync),
) -> SpokeResult<Value> {
    match call {
        PortCall::GetRelation(relation_id) => match baseline {
            Some(baseline) => map_result(baseline.get_relation(&relation_id).await),
            None => SpokeResult::Reject(missing_baseline_face()),
        },
        PortCall::PutRelation(relation, expected) => match baseline {
            Some(baseline) => map_result(baseline.put_relation(relation, expected).await),
            None => SpokeResult::Reject(missing_baseline_face()),
        },
        PortCall::ListTimelineEvents(scope) => match baseline {
            Some(baseline) => map_result(baseline.list_timeline_events(scope).await),
            None => SpokeResult::Reject(missing_baseline_face()),
        },
        PortCall::PutFindings(findings) => match baseline {
            Some(baseline) => map_result(baseline.put_findings(findings).await),
            None => SpokeResult::Reject(missing_baseline_face()),
        },
        PortCall::ListRules(rule_refs) => match baseline {
            Some(baseline) => map_result(baseline.list_rules(&rule_refs).await),
            None => SpokeResult::Reject(missing_baseline_face()),
        },
        PortCall::ListPeerManifests => match baseline {
            Some(baseline) => map_result(baseline.list_peer_host_capability_manifests().await),
            None => SpokeResult::Reject(missing_baseline_face()),
        },
        PortCall::Project(request) => match ports.as_computable() {
            Some(computable) => map_result(computable.project(request).await),
            None => SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::CapabilityPortMissing,
                message: "port op port.computable.project requires optional port method project"
                    .to_owned(),
                details: None,
            }),
        },
        PortCall::Compute(request) => match ports.as_computable() {
            Some(computable) => map_result(computable.compute(request).await),
            None => SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::CapabilityPortMissing,
                message: "port op port.computable.compute requires optional port method compute"
                    .to_owned(),
                details: None,
            }),
        },
        PortCall::ForkTimelineEvents(scope) => match ports.as_fork_timeline() {
            Some(fork) => map_result(fork.list_fork_timeline_events(scope).await),
            None => SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::CapabilityPortMissing,
                message:
                    "port op port.fork.list_timeline_events requires optional port method list_fork_timeline_events"
                        .to_owned(),
                details: None,
            }),
        },
    }
}

/// Map an adapter `SpokeResult<T>` to the opaque success `Value` (or
/// preserve the reject).
fn map_result<T: Serialize>(result: SpokeResult<T>) -> SpokeResult<Value> {
    match result {
        SpokeResult::Ok(value) => match serde_json::to_value(&value) {
            Ok(payload) => spoke_ok(payload),
            Err(error) => spoke_reject(
                SpokeRejectCode::InternalError,
                format!("port response serialize failed: {error}"),
                None,
            ),
        },
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

/// §14.2/§14.3, history face: the serve budget expired AFTER the history
/// service was invoked, so the transport has no evidence that nothing
/// committed — a prune may have applied. The response says the outcome is
/// unknown instead of claiming a clean failure; provider work is neither
/// cancelled nor rolled back by this end, and the settled prune is still
/// recorded by the service dedup so a same-key replay recovers it
/// (spoke-history contract §9.1).
fn history_uncertain_reject(budget: Duration) -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("timeout".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: format!(
            "history serve wait exceeded the local budget of {}ms; the outcome is unknown — replay the same idempotency key",
            budget.as_millis()
        ),
        details: Some(details),
    }
}

/// §14.2/§14.3: the detached history invoke panicked (or otherwise
/// dropped its result channel) AFTER invocation, so the transport has no
/// evidence that nothing committed — a clean `INTERNAL_ERROR` would
/// assert no-effect. The response says the outcome is unknown instead
/// (same containment as the memory panic path).
fn history_panic_uncertain_reject() -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("panic".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: "history service failed after invocation; the outcome is unknown — replay the same idempotency key"
            .to_string(),
        details: Some(details),
    }
}

/// §14.2/§14.3, adoption face: the serve budget expired AFTER the
/// adoption service was invoked, so the transport has no evidence that
/// nothing committed — an adoption may have written its destination copy.
/// The response says the outcome is unknown instead of claiming a clean
/// failure; provider work is neither cancelled nor rolled back by this
/// end, and the settled adoption is still recorded by the service dedup
/// so a same-key replay recovers it (same rule as `history.*`).
fn adoption_uncertain_reject(budget: Duration) -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("timeout".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: format!(
            "adoption serve wait exceeded the local budget of {}ms; the outcome is unknown — replay the same idempotency key",
            budget.as_millis()
        ),
        details: Some(details),
    }
}

/// §14.2/§14.3: the detached adoption invoke panicked (or otherwise
/// dropped its result channel) AFTER invocation, so the transport has no
/// evidence that nothing committed — a clean `INTERNAL_ERROR` would
/// assert no-effect. The response says the outcome is unknown instead
/// (same containment as the memory/history panic paths).
fn adoption_panic_uncertain_reject() -> SpokeReject {
    let mut details = Map::new();
    details.insert("retry_class".into(), Value::String("uncertain".into()));
    details.insert("kind".into(), Value::String("panic".into()));
    SpokeReject {
        code: SpokeRejectCode::OutcomeUncertain,
        message: "adoption service failed after invocation; the outcome is unknown — replay the same idempotency key"
            .to_string(),
        details: Some(details),
    }
}

/// A parked reverse invoke: correlation material + the waiter channel + the
/// timeout task (mirror of the adapter's `PendingInvoke`).
struct PendingReverseInvoke {
    correlation: Correlation,
    /// The request op: a response envelope carries no `op`, so the waiter
    /// supplies it to select the op-scoped response revision paths.
    op: String,
    tx: tokio::sync::mpsc::Sender<Result<ConnectInvokeResponse, ResponderError>>,
    timeout_task: tokio::task::JoinHandle<()>,
}

/// Gate-phase outcome for an inbound invoke (see
/// [`ConnectResponder::run_gate`]).
enum ServeGateResult {
    /// Stray request (no established session) — ignored, no response.
    Stray,
    /// Gate rejection: answer the error branch with these wire fields.
    Denied {
        code: String,
        message: String,
        details: Option<Map<String, Value>>,
    },
    /// Gate passed — dispatch may run.
    Ok,
}

/// Established-session state, per peer.
struct ResponderSession {
    session_id: String,
    /// The verified dialer's hello Ed25519 public key (set at handshake) —
    /// every inbound post-hello envelope is verified against it.
    client_pubkey: [u8; 32],
    /// The session's negotiated capabilities (intersection of the local and
    /// dialer hello manifests) — the dispatch gate for inbound invokes.
    negotiated_capabilities: Vec<String>,
    inbound: Mutex<InboundSequence>,
    outbound: Mutex<OutboundSequence>,
}

/// Connect responder over one established session (frozen contract §6).
/// Construct via [`connect_responder`] — the factory alone yields an
/// un-established responder whose handshake runs in the background; the
/// dialer's hello is the synchronization point.
pub struct ConnectResponder {
    transport: Arc<dyn Transport>,
    /// This responder's raw Ed25519 hello-identity seed.
    local_secret: [u8; 32],
    /// Host manifest advertised in the responder's signed hello.
    manifest: HostCapabilityManifest,
    /// Trusted dialer peer ids (fail-closed allowlist).
    allowlist: Vec<String>,
    /// Preconfigured dialer Ed25519 public keys by peer_id (responder-owned
    /// trust config). A dialing peer on `allowlist` without a preconfigured
    /// key fails the handshake (fail-closed).
    peer_keys: HashMap<String, [u8; 32]>,
    /// Local async [`RemoteServePorts`] served on the remote side via the
    /// D4 catalogue. Absent `ports` still answers `port.*` invokes with the
    /// dispatch-deny branch (documented behavior).
    ports: Option<Arc<dyn RemoteServePorts + Send + Sync>>,
    /// Bounded-wait deadline for each reverse-invoke waiter AND for every
    /// served provider call (`port.*` dispatch + the whole `extract`
    /// service), ms — the local serve-wait budget of D14 Serving.
    invoke_timeout: Duration,
    /// Dialer hello nonce single-use record (handshake replay protection).
    nonce_store: Mutex<NonceStore>,
    state: Mutex<ConnectResponderState>,
    session: Mutex<Option<ResponderSession>>,
    remote_manifest: Mutex<Option<HostCapabilityManifest>>,
    /// Tool-handler registry for `tools.*` invokes (frozen contract §6):
    /// `register_tool_handler` fills it; the serving path looks it up by
    /// exact capability id. The local manifest's `tools[]` (carried through
    /// hello) is the discovery source — this registry MUST NOT mutate the
    /// manifest; a registry/manifest mismatch is surfaced at invoke time:
    /// a manifest-declared tool with no registered handler is denied
    /// fail-closed (op_unsupported → CAPABILITY_PORT_MISSING);
    /// `validate_manifest_tools` checks manifest-internal consistency
    /// only.
    tool_handlers: Mutex<HashMap<String, ToolHandler>>,
    pending: Arc<Mutex<HashMap<String, PendingReverseInvoke>>>,
    serve_loop_running: AtomicBool,
    /// In-flight serve dispatch tasks (one per gated invoke), tracked so
    /// `close_session` aborts them: neither a serve wait nor its local
    /// budget timer outlives the session it was dispatched for. The serve
    /// path registers a dispatch under this same lock, so a dispatch is
    /// either visible to the close's drain or never started at all.
    serve_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// Outbound send serialization tail. Sequences are allocated
    /// synchronously in call order, but the send of invoke N must not start
    /// until the send of invoke N−1 finished (the dialer's strict inbound
    /// gate rejects out-of-order requests). Mirror of the TS `#sendTail`
    /// promise chain; like the adapter's lock-first `invoke_op`, the tail
    /// lock is acquired BEFORE the outbound allocation so the wire order
    /// matches the allocation order. The waiter is registered before the
    /// tail (its timeout clock starts at call time), so a timeout that fires
    /// while a send is queued behind the tail is observable as a settled
    /// waiter (the deferred-send poison-close mirror, frozen §6).
    send_tail: tokio::sync::Mutex<()>,
}

impl ConnectResponder {
    fn new(options: ConnectResponderOptions) -> Self {
        Self {
            transport: options.transport,
            local_secret: options.identity.seed,
            manifest: options.manifest,
            allowlist: options.allowlist,
            peer_keys: options.peer_keys,
            ports: options.ports,
            invoke_timeout: Duration::from_millis(
                options
                    .invoke_timeout_ms
                    .unwrap_or(DEFAULT_INVOKE_TIMEOUT_MS),
            ),
            nonce_store: Mutex::new(NonceStore::new()),
            state: Mutex::new(ConnectResponderState::Disconnected),
            session: Mutex::new(None),
            remote_manifest: Mutex::new(None),
            tool_handlers: Mutex::new(HashMap::new()),
            pending: Arc::new(Mutex::new(HashMap::new())),
            serve_loop_running: AtomicBool::new(false),
            serve_tasks: Mutex::new(Vec::new()),
            send_tail: tokio::sync::Mutex::new(()),
        }
    }

    /// Read-only session state (frozen contract §6 labels).
    #[must_use]
    pub fn state(&self) -> ConnectResponderState {
        *self.state.lock().expect("state lock")
    }

    /// The assigned session id (`None` before establishment; the Rust
    /// adapter convention — TS returns `""`).
    #[must_use]
    pub fn session_id(&self) -> Option<String> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    /// The verified dialing peer id (`None` before establishment).
    #[must_use]
    pub fn remote_peer_id(&self) -> Option<String> {
        // The session's `initiator_peer_id` — stored as the session binding
        // (the responder is the responder role; the peer id is the dialer's).
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.client_pubkey)
            .map(|pubkey| derive_peer_id_from_ed25519_pubkey(&pubkey))
    }

    /// The dialing peer's `HostCapabilityManifest`, from the authenticated
    /// hello `host` (discovery after auth). `None` before establishment
    /// (the Rust adapter convention; TS throws).
    #[must_use]
    pub fn remote_manifest(&self) -> Option<HostCapabilityManifest> {
        self.remote_manifest.lock().expect("manifest lock").clone()
    }

    /// Release the session and transport. Idempotent; pending reverse
    /// invokes fail with `INTERNAL_ERROR` `details.kind = "session_closed"`.
    pub fn close(&self) {
        self.close_session("local shutdown");
    }

    // ── Tool serving + reverse invoke (frozen contract §6) ────────────────

    /// Register a handler for a `tools.<ns>.<tool_id>` capability served on
    /// this responder. Grammar-validated: a non-`tools.` id returns the
    /// library grammar reject (`INVALID_INPUT` with `details.capability_id`,
    /// the `parse_tool_capability_id` result unchanged) and registers
    /// nothing; `SpokeResult::Ok(())` is answered only after the insertion.
    /// This responder face follows the library result convention; the
    /// dialer-side `RemoteAdapter` face keeps its programmer-error panic.
    /// Duplicate registration for the same id OVERWRITES the previous handler
    /// (last-wins, documented). The registry does NOT mutate the local
    /// manifest — descriptor truth for discovery stays in the manifest's
    /// `tools[]` (sent through hello).
    pub fn register_tool_handler(
        &self,
        capability_id: &str,
        handler: ToolHandler,
    ) -> SpokeResult<()> {
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
        self.tool_handlers
            .lock()
            .expect("tool handlers lock")
            .insert(capability_id.to_owned(), handler);
        SpokeResult::Ok(())
    }

    /// Reverse tool-invoke face (frozen contract §6): issue a
    /// `ConnectInvokeRequest` with `op = capability_id` toward the dialer
    /// and resolve with the tool's `result` (extracted from the success
    /// `payload = { result: <opaque JSON> }`). Deny answers map via the
    /// existing D7 row (`op_unsupported` / `capability_missing` →
    /// `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved).
    pub async fn invoke_tool(&self, capability_id: &str, arguments: Value) -> SpokeResult<Value> {
        // Fail fast on a non-tool capability id (the op string IS the
        // capability string; a non-`tools.` id is a programming error).
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
        // Tool invoke payload shape: `{ "arguments": <opaque JSON> }` (§4).
        let response = match self
            .invoke_op(capability_id, json!({ "arguments": arguments }))
            .await
        {
            Ok(response) => response,
            Err(error) => return internal_error(error.kind.as_str(), error.message),
        };
        match response {
            ConnectInvokeResponse::Variant1 { error, .. } => {
                SpokeResult::Reject(map_error_envelope(&error))
            }
            ConnectInvokeResponse::Variant0 { payload, .. } => {
                // Tool success-payload gate (frozen §4): success is
                // `payload = { "result": <opaque JSON> }`; a success payload
                // without a `result` key rejects with `INTERNAL_ERROR`
                // `details.kind = "transport"`.
                match payload.get("result") {
                    Some(result) => spoke_ok(result.clone()),
                    None => internal_error(
                        "transport",
                        format!(
                            "response payload decode failed: payload does not match the {capability_id} success shape"
                        ),
                    ),
                }
            }
        }
    }

    // ── Session lifecycle (hard-private — only `connect_responder` starts) ──

    /// All failure paths that make the session unusable: mark `Closed`, fail
    /// every pending reverse-invoke waiter with `session_closed`, abort every
    /// in-flight serve dispatch (so neither the serve wait nor its local
    /// budget timer outlives the session), and release the transport
    /// (fire-and-forget).
    fn close_session(&self, reason: &str) {
        {
            let mut state = self.state.lock().expect("state lock");
            if *state == ConnectResponderState::Closed {
                return;
            }
            *state = ConnectResponderState::Closed;
        }

        let entries: Vec<PendingReverseInvoke> = self
            .pending
            .lock()
            .expect("pending lock")
            .drain()
            .map(|(_request_id, entry)| entry)
            .collect();
        for entry in entries {
            entry.timeout_task.abort();
            let _ = entry.tx.try_send(Err(ResponderError::new(
                ResponderErrorKind::SessionClosed,
                format!("connect session closed: {reason}"),
            )));
        }

        let serve_tasks: Vec<tokio::task::JoinHandle<()>> = self
            .serve_tasks
            .lock()
            .expect("serve task lock")
            .drain(..)
            .collect();
        for task in serve_tasks {
            task.abort();
        }

        let transport = Arc::clone(&self.transport);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = transport.close().await;
            });
        }
    }

    // ── Handshake (allowlist-first → hello verify → nonce → dial-bound
    // ── responder hello → signed session snapshot) ─────────────────────────

    async fn handshake(&self) -> Result<(), String> {
        let bytes = self
            .transport
            .recv()
            .await
            .map_err(|error| error.to_string())?;
        let hello: ConnectHello = serde_json::from_slice(&bytes)
            .map_err(|_| "expected ConnectHello from client".to_string())?;
        // Allowlist fail-closed check FIRST: an untrusted peer is rejected
        // before any signature work.
        let client_peer_id = hello.peer_id.as_str().to_string();
        if !is_allowlisted(&self.allowlist, &client_peer_id) {
            return Err(format!("peer {client_peer_id} not on allowlist"));
        }
        let client_pubkey = self
            .peer_keys
            .get(&client_peer_id)
            .copied()
            .ok_or_else(|| {
                format!("no preconfigured public key for allowlisted peer {client_peer_id}")
            })?;
        let derived_client_peer_id = derive_peer_id_from_ed25519_pubkey(&client_pubkey);
        verify_hello_ed25519(&client_pubkey, &derived_client_peer_id, &hello, None)
            .map_err(|error| format!("client hello verification failed: {error}"))?;
        if !self
            .nonce_store
            .lock()
            .expect("nonce lock")
            .check_and_record(&client_peer_id, hello.nonce.as_str())
        {
            return Err("nonce replay".to_string());
        }

        let responder_peer_id = derive_peer_id_from_ed25519_pubkey(
            &ed25519_dalek::SigningKey::from_bytes(&self.local_secret)
                .verifying_key()
                .to_bytes(),
        );
        let remote_manifest: HostCapabilityManifest = {
            let wire: ConnectHostCapabilityManifest = hello.host;
            serde_json::from_value(serde_json::to_value(&wire).expect("hello host serializes"))
                .map_err(|error| format!("dialer hello host decode failed: {error}"))?
        };
        let negotiated: Vec<String> =
            negotiated_capabilities(&self.manifest.capabilities, &remote_manifest.capabilities);

        *self.session.lock().expect("session lock") = Some(ResponderSession {
            session_id: format!("{SESSION_ID_PREFIX}{derived_client_peer_id}"),
            client_pubkey,
            negotiated_capabilities: negotiated.clone(),
            inbound: Mutex::new(InboundSequence::new()),
            outbound: Mutex::new(OutboundSequence::new()),
        });
        *self.remote_manifest.lock().expect("manifest lock") = Some(remote_manifest);

        // Answer with our signed hello (responder role — dial binding): the
        // hello signs the 5-field object incl. `peer_nonce` = the
        // initiator's nonce, so a captured responder hello cannot be
        // replayed into a fresh dial. Then the signed session snapshot.
        let nonce = generate_nonce().map_err(|error| error.to_string())?;
        let hello = sign_hello_ed25519(
            &self.local_secret,
            &nonce,
            &connect_manifest(&self.manifest),
            Some(hello.nonce.as_str()),
        )
        .map_err(|error| format!("responder hello sign failed: {error}"))?;
        let bytes = serde_json::to_vec(&hello).map_err(|error| error.to_string())?;
        self.transport
            .send(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        // The session snapshot is signed with the responder identity
        // (`spoke-connect-session-jcs-v1`) — the dialer verifies it against
        // this responder's hello public key. The wire snapshot requires ≥1
        // negotiated capability; the dialer derives its own set from the
        // hellos, so the fallback only covers a degenerate
        // empty-intersection dial (carried-over demo behavior, no
        // authorization impact).
        let snapshot = authenticate_session(
            &self.local_secret,
            &SessionSignInput {
                session_id: format!("{SESSION_ID_PREFIX}{derived_client_peer_id}"),
                initiator_peer_id: derived_client_peer_id,
                responder_peer_id,
                opened_at: chrono::Utc::now(),
                negotiated_capabilities: if negotiated.is_empty() {
                    vec!["spoke-baseline".to_owned()]
                } else {
                    negotiated
                },
                initial_sequence: 0,
            },
            HashMap::new(),
        )
        .map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
        self.transport
            .send(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    // ── Serve loop (request-first classification) ─────────────────────────

    async fn run(self: &Arc<Self>) {
        match self.handshake().await {
            Ok(()) => {}
            Err(error) => {
                // Handshake failure (or responder misconfiguration): fail the
                // peer's pending recv like a connection drop.
                self.close_session(&format!("handshake failed: {error}"));
                return;
            }
        }
        *self.state.lock().expect("state lock") = ConnectResponderState::Established;
        self.run_serve_loop();
    }

    fn run_serve_loop(self: &Arc<Self>) {
        if self.serve_loop_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let responder = Arc::clone(self);
        tokio::spawn(async move {
            responder.serve_loop().await;
            responder.serve_loop_running.store(false, Ordering::SeqCst);
        });
    }

    async fn serve_loop(self: &Arc<Self>) {
        while *self.state.lock().expect("state lock") == ConnectResponderState::Established {
            let bytes = match self.transport.recv().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    // Transport closed — the session is unusable; fail pending
                    // waiters.
                    self.close_session("transport loss");
                    return;
                }
            };
            let doc: Value = match serde_json::from_slice(&bytes) {
                Ok(doc) => doc,
                Err(error) => {
                    // Unparseable inbound: closes the connection per the
                    // carried-over demo semantics — a bare return would leave
                    // the socket open while the responder stops recv'ing, and
                    // the dialer's established session would hang on its next
                    // invoke.
                    self.close_session(&format!("unparseable inbound message: {error}"));
                    return;
                }
            };

            // Classify request shape FIRST (normative `spoke-connect.md`
            // §Request / response classification): an inbound envelope
            // carrying `op` is a `ConnectInvokeRequest` — never a response,
            // even though a reverse request carries the same correlation
            // echo fields + payload as the response success branch.
            if is_connect_invoke_request(&doc) {
                // Gate serialization: await peek → verify → advance inline;
                // the loop reads the next envelope only after this gate
                // completes. Dispatch fires without blocking the loop.
                match self.serve_invoke(&doc).await {
                    Ok(()) => continue,
                    Err(error) => {
                        // Unexpected serving failure (local key
                        // misconfiguration / internal bug): fail closed like
                        // transport loss — a live session must not silently
                        // drop invokes.
                        self.close_session(&format!("invoke serving failed: {error}"));
                        return;
                    }
                }
            }

            if wire_response_correlation(&doc).is_some() {
                self.demux_response(&doc).await;
                continue;
            }

            // Post-handshake stray envelope (hello / session / unknown
            // shape): ignored.
        }
    }

    /// Serve one inbound invoke per the canonical order (frozen §4):
    /// classify (caller) → stray → sequence peek → envelope-auth verify →
    /// advance → gate → handler → signed response. The caller (serve loop)
    /// awaits this method inline so peek → verify → advance are serialized;
    /// dispatch fires without blocking the loop.
    async fn serve_invoke(self: &Arc<Self>, doc: &Value) -> Result<(), ResponderError> {
        match self.run_gate(doc).await? {
            ServeGateResult::Stray => Ok(()),
            ServeGateResult::Denied {
                code,
                message,
                details,
            } => {
                self.send_reverse_error_envelope(doc, &code, &message, details.as_ref())
                    .await?;
                Ok(())
            }
            ServeGateResult::Ok => {
                let doc = doc.clone();
                let responder = Arc::clone(self);
                // Registration is atomic with `close_session`: the closed
                // check, the spawn and the insert all run under the lock that
                // close drains, so a close concurrent with this dispatch
                // either finds the handle in the list and aborts it, or is
                // seen here and the dispatch never starts. A dispatch
                // registered into an already-closed responder would instead
                // arm a serve wait (and its local budget timer) that no close
                // aborts.
                let mut tasks = self.serve_tasks.lock().expect("serve task lock");
                if self.state() == ConnectResponderState::Closed {
                    return Ok(());
                }
                // Finished handles are pruned on the way through, so the list
                // holds in-flight dispatches only.
                tasks.retain(|handle| !handle.is_finished());
                tasks.push(tokio::spawn(async move {
                    responder.dispatch_invoke(doc).await;
                }));
                Ok(())
            }
        }
    }

    /// Gate phase — sequence peek (non-mutating) → envelope-auth verify →
    /// advance. Fail-closed (auth-before-advance, spec §Verify rules): a
    /// forged/tampered/stripped signature answers `auth_failed` and leaves
    /// the inbound counter unchanged. Returns [`ServeGateResult::Stray`] for
    /// stray requests (ignored), a rejection spec for gate failures, or
    /// [`ServeGateResult::Ok`] when dispatch may run.
    async fn run_gate(&self, doc: &Value) -> Result<ServeGateResult, ResponderError> {
        // Extract session material under the guard; never hold a MutexGuard
        // across an await.
        let (session_id, client_pubkey) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(ServeGateResult::Stray); // stray — no established session
            };
            (session.session_id.clone(), session.client_pubkey)
        };
        let sequence = match doc.get("sequence") {
            // A present-but-non-numeric `sequence` is a malformed wire
            // request: answer the deny branch (parity with the TS gate,
            // whose strict `InboundSequence.peek` throws on a non-number →
            // `invalid_sequence`) instead of silently ignoring it as `Stray`
            // (deny observability parity — a silent ignore makes the sender
            // wait out its timeout for no answer).
            Some(value) => match value.as_i64() {
                Some(sequence) => sequence,
                None => {
                    return Ok(ServeGateResult::Denied {
                        code: "invalid_sequence".to_owned(),
                        message: format!("inbound sequence {value} is not the next expected"),
                        details: None,
                    });
                }
            },
            None => return Ok(ServeGateResult::Stray),
        };
        // Stray check: a `session_id` bound to a DIFFERENT live session
        // would be ignored — this responder owns one session, so verify owns
        // the session-binding assert (mirroring the adapter).
        // 1. Sequence peek — non-mutating (auth-before-advance, frozen §4).
        let peek_ok = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(ServeGateResult::Stray);
            };
            let inbound = session.inbound.lock().expect("inbound lock");
            inbound.peek(sequence).is_ok()
        };
        if !peek_ok {
            return Ok(ServeGateResult::Denied {
                code: "invalid_sequence".to_owned(),
                message: format!("inbound sequence {sequence} is not the next expected"),
                details: None,
            });
        }
        // 2. Envelope-auth verify (contract §7 — auth-before-advance): the
        //    request signature is verified over the wire form against the
        //    dialer's hello Ed25519 public key BEFORE the inbound counter
        //    advances.
        if let Err(error) = verify_invoke_request_auth(&client_pubkey, doc, &session_id) {
            return match error.kind() {
                Some(kind) => Ok(ServeGateResult::Denied {
                    code: EnvelopeAuthError::CODE.to_owned(),
                    message: error.to_string(),
                    details: Some(Map::from_iter([(
                        "kind".into(),
                        Value::String(kind.as_str().to_owned()),
                    )])),
                }),
                // Wrong-length key is responder misconfiguration — fail loudly.
                None => Err(ResponderError::new(
                    ResponderErrorKind::EnvelopeAuthInvalid,
                    error.to_string(),
                )),
            };
        }
        // 3. Advance the inbound counter only after envelope-auth verify
        //    passed. The serialized gate makes the advance race-free.
        {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(ServeGateResult::Stray);
            };
            let advance_result = session
                .inbound
                .lock()
                .expect("inbound lock")
                .advance(sequence);
            advance_result.map_err(|error| {
                ResponderError::new(
                    ResponderErrorKind::Transport,
                    format!("inbound sequence advance failed: {error}"),
                )
            })?;
        }
        Ok(ServeGateResult::Ok)
    }

    /// Dispatch phase — runs after the serialized gate; may interleave with
    /// other invokes. Fully contained: a throwing/panicking handler answers
    /// the error branch and never crashes the loop.
    async fn dispatch_invoke(self: &Arc<Self>, mut doc: Value) {
        if let Err(error) = self.try_dispatch_invoke(&mut doc).await {
            // Unexpected serving failure (e.g. non-JSON-serializable handler
            // result): answer the error branch so the invoker never hangs,
            // and never crash the loop.
            let _ = self
                .send_reverse_error_envelope(
                    &doc,
                    SpokeRejectCode::InternalError.as_str(),
                    &format!("invoke failed: {error}"),
                    None,
                )
                .await;
        }
    }

    /// Contained dispatch body (see [`ConnectResponder::dispatch_invoke`]).
    async fn try_dispatch_invoke(&self, doc: &mut Value) -> Result<(), ResponderError> {
        let negotiated = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(()); // stray — belt-and-braces; the gate checked
            };
            session.negotiated_capabilities.clone()
        };
        let (Some(op), Some(_session_id), Some(_sequence), Some(_request_id)) = (
            doc.get("op").and_then(Value::as_str),
            doc.get("session_id").and_then(Value::as_str),
            doc.get("sequence").and_then(Value::as_i64),
            doc.get("request_id").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        // Dispatch gate — `dispatch_allowed`-level logic (frozen §3):
        // `tools.*` ops require the op string itself; baseline `port.*`
        // ops require `spoke-baseline`; optional families route via the
        // same product map to `l2-computable` / `l5-fork`. All evaluate
        // against `negotiated_capabilities` (never a raw requirements-map
        // composition, which would deny the self-describing tools family).
        if !self.gate_allows(op, &negotiated) {
            // A `memory.*`, `history.*` or `knowledge.*` op with a
            // core-table row is a capability gap: the gate evaluates
            // negotiated capabilities only, so a known op that was not
            // negotiated answers `capability_missing` (protocol §4.1 /
            // spoke-history / spoke-knowledge-adoption contracts). An op
            // with no row at all stays `op_unsupported`.
            let core_gated_deny = op.starts_with("memory.")
                || op.starts_with("history.")
                || op.starts_with("knowledge.");
            let code = if core_gated_deny && required_capability(op).is_some() {
                "capability_missing"
            } else {
                "op_unsupported"
            };
            // §14.2: every memory-boundary denial — a known op missing its
            // capability and an op with no row at all — is classified
            // `terminal`, so the wire code choice never drops the metadata.
            // History denials carry no extra details.
            let memory_details = op.starts_with("memory.").then(memory_terminal_details);
            self.send_reverse_error_envelope(
                doc,
                code,
                &format!("op {op} is not authorized by this session"),
                memory_details.as_ref(),
            )
            .await?;
            return Ok(());
        }
        if op.starts_with("tools.") {
            self.dispatch_tool_invoke(doc).await?;
            return Ok(());
        }
        // `memory.*` ops are served by the optional injected
        // [`RemoteMemoryService`](crate::remote::RemoteMemoryService) face.
        if op.starts_with("memory.") {
            self.dispatch_memory_invoke(doc, &negotiated).await?;
            return Ok(());
        }
        // `history.*` ops are served by the optional injected
        // `RemoteHistoryService` face (spoke-history contract): the same
        // gate → probe → serve/deny order as `memory.*`.
        if op.starts_with("history.") {
            // The service owns the request Value: extract the payload
            // member instead of cloning the whole request tree. The
            // invoke echo never reads `payload` back.
            let payload = doc
                .get_mut("payload")
                .map(Value::take)
                .unwrap_or_else(|| json!({}));
            self.dispatch_history_invoke(doc, payload, &negotiated)
                .await?;
            return Ok(());
        }
        // `knowledge.*` ops are served by the optional injected
        // `RemoteAdoptionService` face (spoke-knowledge-adoption
        // contract): the same gate → probe → serve/deny order as
        // `history.*`.
        if op.starts_with("knowledge.") {
            let payload = doc
                .get_mut("payload")
                .map(Value::take)
                .unwrap_or_else(|| json!({}));
            self.dispatch_adoption_invoke(doc, payload).await?;
            return Ok(());
        }
        // `extract` is a core op served by an optional injected service
        // (F1/F3) — it is not a `port.*` method and carries no port-map row.
        if op == "extract" {
            self.dispatch_extract_invoke(doc).await?;
            return Ok(());
        }
        self.dispatch_port_invoke(doc, &negotiated).await?;
        Ok(())
    }

    /// Dispatch gate: the named combined gate (core table incl. `tools.*`,
    /// then the product `port.*` map).
    fn gate_allows(&self, op: &str, negotiated: &[String]) -> bool {
        port_dispatch_allowed(op, negotiated)
    }

    /// Serve a `tools.*` invoke through the registered handler (or deny).
    async fn dispatch_tool_invoke(&self, doc: &Value) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            return Ok(());
        };
        // Handler or deny — fail-closed serving (frozen deny matrix): a gate
        // pass with no registered handler answers `op_unsupported`.
        let handler = self
            .tool_handlers
            .lock()
            .expect("tool handlers lock")
            .get(op)
            .cloned();
        let Some(handler) = handler else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no handler registered for {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // The request payload carries the tool arguments as
        // `{ "arguments": <opaque JSON> }` (frozen §4). A non-object
        // arguments field is a malformed provider request — serve `{}`
        // (the structural argument gate is caller-side).
        let arguments = doc
            .get("payload")
            .and_then(|payload| payload.get("arguments"))
            .filter(|arguments| arguments.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        // A panicking handler answers the error branch (catch_unwind
        // containment — the panic is caught at the future-poll boundary so
        // the serve loop never crashes), mirroring the adapter.
        let result = match std::panic::AssertUnwindSafe(handler(arguments))
            .catch_unwind()
            .await
        {
            Ok(result) => result,
            Err(payload) => {
                let message = crate::remote::remote_adapter::panic_payload_message(payload);
                SpokeResult::<Value>::Reject(SpokeReject {
                    code: SpokeRejectCode::InternalError,
                    message,
                    details: None,
                })
            }
        };
        self.send_tool_result(doc, result).await
    }

    /// Send the signed response for a served tool invoke (success
    /// `{ result }` branch, or the error branch via `to_error_envelope`).
    async fn send_tool_result(
        &self,
        doc: &Value,
        result: SpokeResult<Value>,
    ) -> Result<(), ResponderError> {
        let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(sequence) = doc.get("sequence").and_then(Value::as_i64) else {
            return Ok(());
        };
        let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let secret = self.local_secret;
        match result {
            SpokeResult::Ok(value) => {
                // Success branch: `payload = { "result": <opaque JSON> }`
                // (frozen §4).
                let signed = authenticate_invoke_response_with_op(
                    &secret,
                    &InvokeResponseSignInput::Success {
                        session_id: session_id.to_owned(),
                        sequence,
                        request_id: request_id.to_owned(),
                        payload: json!({ "result": value }),
                    },
                    HashMap::new(),
                    doc.get("op").and_then(Value::as_str),
                )
                .map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("response sign failed: {error}"),
                    )
                })?;
                self.send_reverse_response(
                    &serde_json::to_value(signed).expect("signed response serializes"),
                );
                Ok(())
            }
            SpokeResult::Reject(reject) => {
                let error: spoke_schemas::connect::connect_invoke_response::ErrorEnvelope =
                    serde_json::from_value(
                        serde_json::to_value(to_error_envelope(&reject))
                            .expect("ops error envelope serializes"),
                    )
                    .expect("field-identical error envelope converts");
                let signed = authenticate_invoke_response_with_op(
                    &secret,
                    &InvokeResponseSignInput::Error {
                        session_id: session_id.to_owned(),
                        sequence,
                        request_id: request_id.to_owned(),
                        error,
                    },
                    HashMap::new(),
                    doc.get("op").and_then(Value::as_str),
                )
                .map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("response sign failed: {error}"),
                    )
                })?;
                self.send_reverse_response(
                    &serde_json::to_value(signed).expect("signed response serializes"),
                );
                Ok(())
            }
        }
    }

    /// Serve a core `extract` invoke through the injected
    /// [`RemoteExtractService`](crate::remote::RemoteExtractService)
    /// (F1/F3). The static capability gate already passed in
    /// [`ConnectResponder::try_dispatch_invoke`]; the rest of the order is
    /// probe → decode/validate → call the service once → signed response.
    ///
    /// The service owns the host-local extraction port, the loaded input
    /// value and the extractor — neither is a transport parameter, and this
    /// end never encodes one. An injected service that answers with the
    /// `ExtractResponse` error branch is normalized through the existing
    /// error-envelope map to the same application reject path.
    async fn dispatch_extract_invoke(&self, doc: &Value) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            return Ok(());
        };
        // Provider probe (gate → probe → serve/deny): an absent ports face
        // and a ports face without the `extract` service are the same
        // documented dispatch-deny branch — the host declared the capability
        // without providing the service.
        let Some(service) = self.ports.as_ref().and_then(|ports| ports.as_extract()) else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no extract service configured for op {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // Decode/validate before the service runs: the invoke payload IS the
        // `ExtractRequest` (F1), not a wrapper object.
        let payload = doc.get("payload").cloned().unwrap_or_else(|| json!({}));
        let request = match serde_json::from_value::<ExtractRequest>(payload) {
            Ok(request) => request,
            Err(error) => {
                self.send_reverse_error_envelope(
                    doc,
                    SpokeRejectCode::InvalidInput.as_str(),
                    &format!("invalid {op} payload: {error}"),
                    None,
                )
                .await?;
                return Ok(());
            }
        };
        // B1/B2 serve bound: the complete extraction service (loader +
        // extractor) shares ONE local budget — the wait starts at provider
        // invocation, not per stage. A zero budget answers the signed
        // timeout without starting the service; expiry drops the timed-out
        // future (no late response is possible).
        let bounded = if self.invoke_timeout.is_zero() {
            None
        } else {
            tokio::time::timeout(self.invoke_timeout, service.extract(request))
                .await
                .ok()
        };
        // A close during the bounded wait released the session: this dispatch
        // answers nothing afterwards — no late success, no late second error.
        if self.state() == ConnectResponderState::Closed {
            return Ok(());
        }
        let Some(result) = bounded else {
            let reject = serve_timeout_reject(self.invoke_timeout);
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
            return Ok(());
        };
        let response = match result {
            SpokeResult::Ok(response) => response,
            SpokeResult::Reject(reject) => {
                self.send_reverse_error_envelope(
                    doc,
                    reject.code.as_str(),
                    &reject.message,
                    reject.details.as_ref(),
                )
                .await?;
                return Ok(());
            }
        };
        if let ExtractResponse::Variant1 { error, .. } = &response {
            let reject = serde_json::to_value(error)
                .ok()
                .and_then(|value| {
                    serde_json::from_value::<spoke_schemas::ErrorEnvelope>(value).ok()
                })
                .map_or_else(
                    || SpokeReject {
                        code: SpokeRejectCode::InternalError,
                        message: format!("{op} service returned an unmappable error branch"),
                        details: None,
                    },
                    |error| from_error_envelope(&error),
                );
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
            return Ok(());
        }
        // Success payload is the existing `ExtractResponse` success branch
        // itself (F1) — no `{ result }` wrapper.
        match serde_json::to_value(&response) {
            Ok(payload) => {
                self.send_ok_response(doc, payload).await?;
            }
            Err(error) => {
                self.send_reverse_error_envelope(
                    doc,
                    SpokeRejectCode::InternalError.as_str(),
                    &format!("{op} response serialize failed: {error}"),
                    None,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Serve a `memory.*` invoke through the injected
    /// [`RemoteMemoryService`](crate::remote::RemoteMemoryService) (protocol
    /// §13). The static capability gate already passed in
    /// [`ConnectResponder::try_dispatch_invoke`] — a known memory op that was
    /// not negotiated never reaches here (`capability_missing`). The rest of
    /// the order is probe → call the service once → signed response; an
    /// absent face is the documented dispatch-deny branch (the host declared
    /// `spoke-memory` without providing the service).
    ///
    /// The service owns the provider port, the admission context (the
    /// authenticated peer → `Principal`) and the per-op wire mapping; this
    /// end forwards the op string and its payload object unchanged.
    async fn dispatch_memory_invoke(
        &self,
        doc: &Value,
        negotiated: &[String],
    ) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            // Defensive: the dispatch gate routes only string ops here, so a
            // non-string op means a caller bypassed it. Fail closed with the
            // terminal memory-boundary reject instead of silently swallowing
            // the invoke (which would leave the sender waiting).
            let details = memory_terminal_details();
            self.send_reverse_error_envelope(
                doc,
                SpokeRejectCode::InvalidInput.as_str(),
                "invalid memory invoke: op must be a string",
                Some(&details),
            )
            .await?;
            return Ok(());
        };
        // Payload-dependent cognition gate (consumer contract §2.1/§2.2) on
        // the raw payload: runs before the service probe and any provider
        // call, so a denied requirement has no memory side effect.
        let empty_payload = json!({});
        let payload = doc.get("payload").unwrap_or(&empty_payload);
        if !self
            .serve_cognition_gate(doc, payload, negotiated, op)
            .await?
        {
            return Ok(());
        }
        let Some(service) = self.ports.as_ref().and_then(|ports| ports.as_memory()) else {
            let details = memory_terminal_details();
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no memory service configured for op {op}"),
                Some(&details),
            )
            .await?;
            return Ok(());
        };
        // The admitted principal derives from the authenticated session peer
        // (MEM-R003), never from the payload.
        let Some(peer_id) = self.remote_peer_id() else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no authenticated peer for op {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        let payload = doc.get("payload").cloned().unwrap_or_else(|| json!({}));
        // Same serve bound as `extract`: the wait starts at provider
        // invocation; a zero budget answers the signed timeout without
        // starting the service.
        // A zero budget rejects before the service is invoked (the baseline
        // serve-timeout rejection stands). A non-zero budget that expires
        // AFTER invocation means the mutation may have committed: that is
        // `OUTCOME_UNCERTAIN`, not a clean internal error. The admitted
        // invoke is detached from the response deadline (spawned, matching
        // the TS path's uncancellable promise): expiry answers the budget,
        // it never cancels the provider task, and polling then observes the
        // real completion.
        let zero_budget = self.invoke_timeout.is_zero();
        let bounded = if zero_budget {
            None
        } else {
            let service = Arc::clone(&service);
            let peer_id = peer_id.clone();
            let op = op.to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _ = tx.send(service.invoke(&peer_id, &op, payload).await);
            });
            match tokio::time::timeout(self.invoke_timeout, rx).await {
                Ok(Ok(result)) => Some(result),
                // The detached task sends exactly once before dropping the
                // sender; a receive failure means it panicked AFTER
                // admission, so — like a budget expiry — the transport
                // cannot assert no-effect (§14.3): answer uncertain, never
                // a clean internal error.
                Ok(Err(_)) => Some(SpokeResult::Reject(memory_panic_uncertain_reject())),
                Err(_) => None,
            }
        };
        if self.state() == ConnectResponderState::Closed {
            return Ok(());
        }
        let Some(result) = bounded else {
            let reject = if zero_budget {
                serve_timeout_reject(self.invoke_timeout)
            } else {
                memory_uncertain_reject(self.invoke_timeout)
            };
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
            return Ok(());
        };
        match result {
            SpokeResult::Ok(value) => self.send_ok_response(doc, value).await?,
            SpokeResult::Reject(reject) => {
                self.send_reverse_error_envelope(
                    doc,
                    reject.code.as_str(),
                    &reject.message,
                    reject.details.as_ref(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Serve a `history.*` invoke through the injected
    /// [`RemoteHistoryService`](crate::remote::RemoteHistoryService) face
    /// (spoke-history contract §Connect operations). The static capability
    /// gate already passed in
    /// [`ConnectResponder::try_dispatch_invoke`] — a known history op that
    /// was not negotiated never reaches here (`capability_missing`). The
    /// serving order mirrors the `memory.*` branch: cognition gate →
    /// `as_history` probe → bounded service invoke → response envelope.
    /// The service owns the closed request decode, peer admission,
    /// effective-scope resolution and prune dedup; an absent face is the
    /// documented dispatch-deny branch (the host declared `spoke-history`
    /// without providing the service).
    ///
    /// The serve bound matches the `memory.*` ops: a zero budget answers
    /// the plain timeout without invoking the service at all, while
    /// expiry AFTER invocation means the admitted mutation may have
    /// committed — that is `OUTCOME_UNCERTAIN`/`uncertain`
    /// (§14.2/§14.3), recoverable through the service's recorded-outcome
    /// dedup, never a clean internal error or a silent re-execution. The
    /// service task keeps running past the deadline so a settling prune
    /// still records its outcome.
    async fn dispatch_history_invoke(
        &self,
        doc: &Value,
        payload: Value,
        negotiated: &[String],
    ) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            // Defensive: the dispatch gate routes only string ops here — a
            // non-string op means a caller bypassed it. Fail closed instead
            // of silently swallowing the invoke.
            self.send_reverse_error_envelope(
                doc,
                SpokeRejectCode::InvalidInput.as_str(),
                "invalid history invoke: op must be a string",
                None,
            )
            .await?;
            return Ok(());
        };
        // Payload-dependent cognition gate on the extracted payload: the
        // history request `scope` carries the `Scope.cognition` selection
        // slot, so a declared requirement needs `spoke-cognition`
        // negotiated before the probe and any provider call.
        if !self
            .serve_cognition_gate(doc, &payload, negotiated, op)
            .await?
        {
            return Ok(());
        }
        let Some(service) = self.ports.as_ref().and_then(|ports| ports.as_history()) else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no history service configured for op {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // The admitted principal/scope derive from the authenticated
        // session peer, never from the payload (MEM-R003; contract
        // §Authorization).
        let Some(peer_id) = self.remote_peer_id() else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no authenticated peer for op {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // Same serve bound as `memory.*`: a zero budget rejects before the
        // service is invoked (the baseline serve-timeout rejection
        // stands). A non-zero budget that expires AFTER invocation means
        // the mutation may have committed: that is `OUTCOME_UNCERTAIN`,
        // not a clean internal error. The admitted invoke is detached
        // from the response deadline (spawned, matching the TS path's
        // uncancellable promise): expiry answers the budget, it never
        // cancels the provider task, and the dedup store still records
        // the settled outcome for a same-key replay to recover.
        let zero_budget = self.invoke_timeout.is_zero();
        let bounded = if zero_budget {
            None
        } else {
            let service = Arc::clone(&service);
            let peer_id = peer_id.clone();
            let op = op.to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _ = tx.send(service.invoke(&peer_id, &op, payload).await);
            });
            match tokio::time::timeout(self.invoke_timeout, rx).await {
                Ok(Ok(result)) => Some(result),
                // The detached task sends exactly once before dropping the
                // sender; a receive failure means it panicked AFTER
                // admission, so — like a budget expiry — the transport
                // cannot assert no-effect (§14.3): answer uncertain,
                // never a clean internal error.
                Ok(Err(_)) => Some(SpokeResult::Reject(history_panic_uncertain_reject())),
                Err(_) => None,
            }
        };
        if self.state() == ConnectResponderState::Closed {
            return Ok(());
        }
        let Some(result) = bounded else {
            let reject = if zero_budget {
                serve_timeout_reject(self.invoke_timeout)
            } else {
                history_uncertain_reject(self.invoke_timeout)
            };
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
            return Ok(());
        };
        match result {
            SpokeResult::Ok(value) => self.send_ok_response(doc, value).await?,
            SpokeResult::Reject(reject) => {
                self.send_reverse_error_envelope(
                    doc,
                    reject.code.as_str(),
                    &reject.message,
                    reject.details.as_ref(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Serve a `knowledge.*` invoke through the injected
    /// [`RemoteAdoptionService`](crate::remote::RemoteAdoptionService)
    /// face (spoke-knowledge-adoption contract §Connect operation). The
    /// static capability gate already passed in
    /// [`ConnectResponder::try_dispatch_invoke`] — `knowledge.adopt`
    /// requires `spoke-knowledge-adoption` AND `spoke-knowledge`, and a
    /// known adoption op that was not negotiated never reaches here
    /// (`capability_missing`). The serving order mirrors the `history.*`
    /// branch: `as_adoption` probe → bounded service invoke → response
    /// envelope. The service owns the closed request decode, peer
    /// admission, effective-scope resolution and idempotency dedup; an
    /// absent face is the documented dispatch-deny branch (the host
    /// declared `spoke-knowledge-adoption` without providing the
    /// service). The request carries scope ids, never a declared `Scope`
    /// — there is no cognition selection slot, so no cognition gate.
    ///
    /// The serve bound matches the `history.*` ops: a zero budget answers
    /// the plain timeout without invoking the service at all, while
    /// expiry AFTER invocation means the admitted mutation may have
    /// committed — that is `OUTCOME_UNCERTAIN`/`uncertain`, recoverable
    /// through the service's recorded-outcome dedup, never a clean
    /// internal error or a silent re-execution. The service task keeps
    /// running past the deadline so a settling adoption still records its
    /// outcome.
    async fn dispatch_adoption_invoke(
        &self,
        doc: &Value,
        payload: Value,
    ) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            // Defensive: the dispatch gate routes only string ops here — a
            // non-string op means a caller bypassed it. Fail closed instead
            // of silently swallowing the invoke.
            self.send_reverse_error_envelope(
                doc,
                SpokeRejectCode::InvalidInput.as_str(),
                "invalid adoption invoke: op must be a string",
                None,
            )
            .await?;
            return Ok(());
        };
        let Some(service) = self.ports.as_ref().and_then(|ports| ports.as_adoption()) else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no adoption service configured for op {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // The admitted principal/scope derive from the authenticated
        // session peer, never from the payload (MEM-R003; contract
        // §Admission and owner-private absence).
        let Some(peer_id) = self.remote_peer_id() else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no authenticated peer for op {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // Same serve bound as `history.*`: a zero budget rejects before the
        // service is invoked. A non-zero budget that expires AFTER
        // invocation means the mutation may have committed: that is
        // `OUTCOME_UNCERTAIN`, not a clean internal error. The admitted
        // invoke is detached from the response deadline (spawned): expiry
        // answers the budget, it never cancels the provider task, and the
        // dedup store still records the settled outcome for a same-key
        // replay to recover.
        let zero_budget = self.invoke_timeout.is_zero();
        let bounded = if zero_budget {
            None
        } else {
            let service = Arc::clone(&service);
            let peer_id = peer_id.clone();
            let op = op.to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _ = tx.send(service.invoke(&peer_id, &op, payload).await);
            });
            match tokio::time::timeout(self.invoke_timeout, rx).await {
                Ok(Ok(result)) => Some(result),
                // The detached task sends exactly once before dropping the
                // sender; a receive failure means it panicked AFTER
                // admission, so — like a budget expiry — the transport
                // cannot assert no-effect: answer uncertain, never a clean
                // internal error.
                Ok(Err(_)) => Some(SpokeResult::Reject(adoption_panic_uncertain_reject())),
                Err(_) => None,
            }
        };
        if self.state() == ConnectResponderState::Closed {
            return Ok(());
        }
        let Some(result) = bounded else {
            let reject = if zero_budget {
                serve_timeout_reject(self.invoke_timeout)
            } else {
                adoption_uncertain_reject(self.invoke_timeout)
            };
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
            return Ok(());
        };
        match result {
            SpokeResult::Ok(value) => self.send_ok_response(doc, value).await?,
            SpokeResult::Reject(mut reject) => {
                // qc1 F-031 — the responder never trusts an arbitrary
                // `RemoteAdoptionService` implementation: a Reject
                // receipt's revision-typed detail gets the same
                // `[0, 2^53]` guard as the served branches, so an
                // out-of-domain `details.adopted_revision` answers a
                // provider-fault `INTERNAL_ERROR` instead of travelling
                // on the wire.
                if let Some(fault) =
                    crate::remote::serve_ports::normalize_adopt_reject_details(&mut reject)
                {
                    reject = fault;
                }
                self.send_reverse_error_envelope(
                    doc,
                    reject.code.as_str(),
                    &reject.message,
                    reject.details.as_ref(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Payload-dependent cognition gate shared by the two cognition-carrying
    /// serving branches (`port.*` and `memory.*`), parallel to the F2
    /// ownership conjunct (consumer contract §2.1/§2.2).
    ///
    /// `Ok(true)` means the request may be served; `Ok(false)` means the deny
    /// envelope was already sent and the caller must stop — no probe, no
    /// provider call, no provider side effect.
    async fn serve_cognition_gate(
        &self,
        doc: &Value,
        payload: &Value,
        negotiated: &[String],
        op: &str,
    ) -> Result<bool, ResponderError> {
        if !cognition_requirement_present(op, payload) {
            return Ok(true);
        }
        if !negotiated
            .iter()
            .any(|cap| cap == CAPABILITY_SPOKE_COGNITION)
        {
            // Family absence is the normative consumer contract's
            // `capability_missing` branch (§2.1); the ownership conjunct keeps
            // `op_unsupported`. §D7 maps both codes to one dialer class.
            self.send_reverse_error_envelope(
                doc,
                "capability_missing",
                &format!(
                    "op {op} requires capability {CAPABILITY_SPOKE_COGNITION} for its declared cognition requirement"
                ),
                None,
            )
            .await?;
            return Ok(false);
        }
        if let Some((message, details)) = unsupported_cognition_requirement(op, payload) {
            self.send_reverse_error_envelope(doc, WIRE_INVALID_INPUT, message, Some(&details))
                .await?;
            return Ok(false);
        }
        Ok(true)
    }

    /// Serve one of the three knowledge ops through the knowledge face.
    ///
    /// Gate order: decode → face probe → admission (exactly once, keyed by
    /// the authenticated peer, with the declared scope selector when there is
    /// one) → declared scope equality/intersection → store call. A list
    /// always declares a scope; get/put may (A2). The request-scoped face is
    /// probed before the peer-only face
    /// (A1-D5). Admission and the store share ONE deadline; `Ok(None)` means
    /// a refusal envelope was already sent and the store was never called — a
    /// refusal discloses nothing, not even whether the entry exists.
    async fn serve_knowledge_op(
        &self,
        doc: &Value,
        op: &str,
        payload: &Value,
        declared_scope: Option<&Scope>,
        ports: &(dyn RemoteServePorts + Send + Sync),
    ) -> Result<Option<SpokeResult<Value>>, ResponderError> {
        let call = match decode_knowledge_call(op, payload, declared_scope) {
            SpokeResult::Ok(call) => call,
            SpokeResult::Reject(reject) => {
                self.send_reverse_error_envelope(
                    doc,
                    reject.code.as_str(),
                    &reject.message,
                    reject.details.as_ref(),
                )
                .await?;
                return Ok(None);
            }
        };
        // Gate → probe → serve/deny: a declared `spoke-knowledge` without the
        // serving face is the same dispatch-deny branch as an absent
        // baseline face, never a baseline fallback (D7 wire code). The
        // request-scoped face wins, and the peer-only probe runs only when it
        // is absent — a host that supplies the scoped face never sees the
        // fallback probe (it may panic or do work); a host that adopts
        // nothing keeps the peer-only face and its `0.18.1` answer.
        let face = match ports.as_scoped_knowledge() {
            Some(scoped) => KnowledgeServingFace::Scoped(scoped),
            None => match ports.as_knowledge() {
                Some(peer_only) => KnowledgeServingFace::PeerOnly(peer_only),
                None => {
                    self.send_reverse_error_envelope(
                        doc,
                        "op_unsupported",
                        &format!("no knowledge face configured for port op {op}"),
                        None,
                    )
                    .await?;
                    return Ok(None);
                }
            },
        };
        let store = face.store();
        // The admitted scope derives from the authenticated session peer,
        // never from the payload. One refusal shape for an unbound peer and
        // for a rejecting admission: no existence detail.
        let Some(peer_id) = self.remote_peer_id() else {
            self.deny_knowledge_scope(doc).await?;
            return Ok(None);
        };
        // Admission is provider work too, so the same serve budget bounds it
        // (B1). It cannot be interrupted — it runs on the blocking pool — so
        // the responder's wait, not a forced stop, is the guarantee; a zero
        // budget therefore answers the signed timeout without admitting.
        if self.invoke_timeout.is_zero() {
            self.deny_knowledge_serve_timeout(doc).await?;
            return Ok(None);
        }
        // One deadline for the whole op: admission and the store call share the
        // local budget, so a slow admission cannot buy a second full timeout.
        let deadline = std::time::Instant::now() + self.invoke_timeout;
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        let admission = match &face {
            KnowledgeServingFace::Scoped(face) => {
                // The selector the request declares: the list op names one
                // and get/put may (A2). Only the scoped closure captures it,
                // so the peer-only path never copies it.
                let selector = declared_scope.map(|scope| scope.scope_id.clone());
                let admission = Arc::clone(&face.admission);
                tokio::time::timeout(
                    wait,
                    tokio::task::spawn_blocking(move || admission(&peer_id, selector.as_deref())),
                )
                .await
            }
            KnowledgeServingFace::PeerOnly(face) => {
                let admission = Arc::clone(&face.admission);
                tokio::time::timeout(
                    wait,
                    tokio::task::spawn_blocking(move || admission(&peer_id)),
                )
                .await
            }
        };
        let admitted = match admission {
            Ok(Ok(SpokeResult::Ok(scope))) => scope,
            Ok(Ok(SpokeResult::Reject(_))) => {
                self.deny_knowledge_scope(doc).await?;
                return Ok(None);
            }
            // A panicking admission is contained, exactly like a foreign
            // resolver panic: nothing reached the store, so this is a clean
            // internal error, not an uncertain outcome. The join payload
            // carries the callback's panic text; it stays host-side and never
            // reaches the wire.
            Ok(Err(_join)) => {
                self.send_reverse_error_envelope(
                    doc,
                    SpokeRejectCode::InternalError.as_str(),
                    "knowledge admission failed",
                    None,
                )
                .await?;
                return Ok(None);
            }
            Err(_) => {
                self.deny_knowledge_serve_timeout(doc).await?;
                return Ok(None);
            }
        };
        // A declared scope (always for the list op, optionally for get/put)
        // must equal the admitted one, and its effective form is the admitted
        // authority intersected with the request (A2-D4 — the same two
        // checks). Both refusals land before the store call so a request
        // cannot widen the query.
        let served_scope = match declared_scope {
            Some(requested) => {
                if !scope_matches_admission(requested, &admitted) {
                    self.deny_knowledge_scope(doc).await?;
                    return Ok(None);
                }
                match effective_scope(requested, &admitted) {
                    Some(effective) => Some(effective),
                    None => {
                        self.deny_knowledge_scope(doc).await?;
                        return Ok(None);
                    }
                }
            }
            None => None,
        };
        // An absent scope keeps the admitted one (the `0.18.1` get/put path).
        let store_scope = served_scope.as_ref().unwrap_or(&admitted);
        // The store call runs under what is left of the same deadline. A budget
        // already spent on admission answers the signed timeout without
        // reaching the store, so a caller that timed out cannot still mutate.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(Some(SpokeResult::Reject(serve_timeout_reject(
                self.invoke_timeout,
            ))));
        }
        // Only a put mutates: an expiry after the store was invoked may have
        // applied it, so that expiry reports an unknown outcome. Reads have no
        // store effect to be uncertain about and keep the plain timeout.
        let mutates = matches!(&call, KnowledgeCall::PutKnowledgeEntry(..));
        let result = match tokio::time::timeout(
            remaining,
            call_knowledge_op(call, store.as_ref(), store_scope, served_scope.as_ref()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) if mutates => {
                SpokeResult::Reject(knowledge_uncertain_reject(self.invoke_timeout))
            }
            Err(_) => SpokeResult::Reject(serve_timeout_reject(self.invoke_timeout)),
        };
        Ok(Some(result))
    }

    /// Answer the one knowledge-scope refusal shape: no existence detail,
    /// whatever the admission said.
    async fn deny_knowledge_scope(&self, doc: &Value) -> Result<(), ResponderError> {
        self.send_reverse_error_envelope(
            doc,
            SpokeRejectCode::NotAuthorized.as_str(),
            KNOWLEDGE_SCOPE_NOT_AUTHORIZED,
            None,
        )
        .await
    }

    /// Answer the signed serve-budget expiry for a knowledge op.
    async fn deny_knowledge_serve_timeout(&self, doc: &Value) -> Result<(), ResponderError> {
        let reject = serve_timeout_reject(self.invoke_timeout);
        self.send_reverse_error_envelope(
            doc,
            reject.code.as_str(),
            &reject.message,
            reject.details.as_ref(),
        )
        .await
    }

    /// Serve a `port.*` invoke through the D4 catalogue (or dispatch-deny).
    ///
    /// `negotiated` is the session's negotiated capability set (the caller
    /// already read it for the static gate) — the supplementary F2 ownership
    /// conjunct is evaluated against the same set.
    async fn dispatch_port_invoke(
        &self,
        doc: &Value,
        negotiated: &[String],
    ) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            return Ok(());
        };
        // Borrowed, not cloned: every reader below takes `&Value`, and the
        // absent-payload case reads as an empty object exactly as the previous
        // `json!({})` fallback did.
        let empty_payload = json!({});
        let payload = doc.get("payload").unwrap_or(&empty_payload);
        // F2 supplementary product-boundary gate (frozen): the declared Scope
        // validation and the conditional ownership requirement keep the
        // responder's original order — shape first, then ownership — and both
        // still run after the static capability gate and before the provider
        // is probed or called. The dialer never pre-gates, so this is the
        // first place the payload-dependent rules are applied.
        //
        // A payload that declares the new optional cognition subtree can fail
        // the typed decode on that subtree alone (the closed grammar is
        // `deny_unknown_fields`). Such a failure is deferred to the raw §2.2
        // diagnosis below, where it is reported with its own requirement path;
        // every baseline shape failure still answers first.
        let mut deferred_scope_reject = None;
        let declared_scope = match validate_scope_declaration(op, payload) {
            Ok(scope) => scope,
            Err(reject) => {
                if cognition_requirement_present(op, payload)
                    && baseline_scope_shape(op, payload).is_ok()
                {
                    deferred_scope_reject = Some(reject);
                    None
                } else {
                    self.send_reverse_error_envelope(
                        doc,
                        reject.code.as_str(),
                        &reject.message,
                        reject.details.as_ref(),
                    )
                    .await?;
                    return Ok(());
                }
            }
        };
        // A2-D3: the marker is a conjunct on the declared scope, checked
        // after its shape (a malformed member keeps `INVALID_INPUT`) and
        // before the ownership/cognition conjuncts. A session that did not
        // negotiate it must never be served a scope it would silently drop.
        if declared_scope.is_some()
            && optional_scope_op(op)
            && !negotiated
                .iter()
                .any(|cap| cap == CAPABILITY_SPOKE_KNOWLEDGE_SCOPED)
        {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!(
                    "op {op} requires capability {CAPABILITY_SPOKE_KNOWLEDGE_SCOPED} for its declared scope"
                ),
                None,
            )
            .await?;
            return Ok(());
        }
        if requires_ownership_capability(op, payload)
            && !negotiated.iter().any(|cap| cap == CAPABILITY_KE_OWNERSHIP)
        {
            // The frozen deny matrix fixes the ownership conjunct to
            // `op_unsupported` (spoke-connect §Op dispatch gate), unlike the
            // cognition family's `capability_missing`.
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!(
                    "op {op} requires capability {CAPABILITY_KE_OWNERSHIP} for its declared viewpoint"
                ),
                None,
            )
            .await?;
            return Ok(());
        }
        // Payload-dependent cognition gate (consumer contract §2.1/§2.2) on
        // the raw payload: after the original shape/ownership denials and
        // before the provider is probed or called.
        if !self
            .serve_cognition_gate(doc, payload, negotiated, op)
            .await?
        {
            return Ok(());
        }
        if let Some(reject) = deferred_scope_reject {
            // The negotiated family serves the declared requirement, so the
            // typed decode failure the baseline shape could not explain
            // stands on its own.
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
            return Ok(());
        }
        let ports = match self.ports.as_ref() {
            Some(ports) => Arc::clone(ports),
            None => {
                // Absent `ports` (documented): the capability gate passes but
                // there is no ports face to serve — answer the
                // dispatch-deny branch.
                self.send_reverse_error_envelope(
                    doc,
                    "op_unsupported",
                    &format!("no ports face configured for port op {op}"),
                    None,
                )
                .await?;
                return Ok(());
            }
        };
        let result = if is_knowledge_op(op) {
            match self
                .serve_knowledge_op(doc, op, payload, declared_scope.as_ref(), ports.as_ref())
                .await?
            {
                Some(result) => result,
                None => return Ok(()),
            }
        } else {
            // Optional-family probe (gate → probe → serve/deny): the
            // capability gate passed, so a missing optional face means the
            // host declared a family it does not provide (misconfiguration)
            // — answer the same dispatch-deny branch as absent ports. The
            // probe precedes the payload decode, exactly as before.
            if let Some(missing) = missing_optional_serve_face(op, ports.as_ref()) {
                self.send_reverse_error_envelope(
                    doc,
                    "op_unsupported",
                    &format!(
                        "port op {op} requires optional port method {missing}, which the configured ports do not implement"
                    ),
                    None,
                )
                .await?;
                return Ok(());
            }
            // B1 serve bound: the payload decode + validation above ran
            // BEFORE the bound, so a malformed payload keeps its existing
            // `INVALID_INPUT` reject even at a zero budget and the bound
            // covers provider dispatch only. A zero budget answers the signed
            // timeout without ever constructing the provider future (zero
            // provider calls); expiry drops the timed-out provider future — a
            // bounded wait, never forced termination or rollback.
            let call = match decode_port_call(op, payload, declared_scope.as_ref()) {
                SpokeResult::Ok(call) => call,
                SpokeResult::Reject(reject) => {
                    self.send_reverse_error_envelope(
                        doc,
                        reject.code.as_str(),
                        &reject.message,
                        reject.details.as_ref(),
                    )
                    .await?;
                    return Ok(());
                }
            };
            // B5: each op is served by the face that owns it, so the baseline
            // probe applies only to the baseline ops — an optional-only host
            // (computable / fork timeline) is not refused for lacking a
            // baseline face.
            let baseline = match &call {
                PortCall::Project(_) | PortCall::Compute(_) | PortCall::ForkTimelineEvents(_) => {
                    None
                }
                _ => match ports.as_baseline() {
                    Some(baseline) => Some(baseline),
                    None => {
                        self.send_reverse_error_envelope(
                            doc,
                            "op_unsupported",
                            &format!("no baseline face configured for port op {op}"),
                            None,
                        )
                        .await?;
                        return Ok(());
                    }
                },
            };
            if self.invoke_timeout.is_zero() {
                SpokeResult::Reject(serve_timeout_reject(self.invoke_timeout))
            } else {
                match tokio::time::timeout(
                    self.invoke_timeout,
                    call_port_op(call, baseline, ports.as_ref()),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => SpokeResult::Reject(serve_timeout_reject(self.invoke_timeout)),
                }
            }
        };
        // A close during the bounded wait released the session: this dispatch
        // answers nothing afterwards — no late success, no late second error.
        if self.state() == ConnectResponderState::Closed {
            return Ok(());
        }
        if let SpokeResult::Ok(value) = result {
            // Success payload carries the raw success value `T` (D4), NOT the
            // `{ result }` tool shape — the dialer's `invoke_mapped`
            // validates it.
            self.send_ok_response(doc, value).await?;
        } else {
            let reject = match result {
                SpokeResult::Reject(reject) => reject,
                SpokeResult::Ok(_) => unreachable!("matched above"),
            };
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
        }
        Ok(())
    }

    // ── response helpers ─────────────────────────────────────────────────────

    /// Send one signed response envelope, fire-and-forget.
    fn send_reverse_response(&self, doc: &Value) {
        let envelope = match serde_json::to_vec(doc) {
            Ok(envelope) => envelope,
            Err(_) => return, // non-JSON-serializable response — drop
        };
        let transport = Arc::clone(&self.transport);
        tokio::spawn(async move {
            let _ = transport.send(&envelope).await;
        });
    }

    /// Sign + send an error-branch response to an inbound invoke. The echo
    /// fields come from the wire document (the request discriminator
    /// guarantees their presence), so reject paths that run before typed
    /// deserialization (sequence gate, envelope-auth verify) can still
    /// answer the sender.
    async fn send_reverse_error_envelope(
        &self,
        doc: &Value,
        code: &str,
        message: &str,
        details: Option<&Map<String, Value>>,
    ) -> Result<(), ResponderError> {
        let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
            return Ok(()); // stray — no echo to answer
        };
        // A present-but-non-numeric `sequence` is denied at the gate's
        // sequence-extraction step (parity with the TS gate, which echoes
        // the raw value verbatim); the typed response envelope carries an
        // i64 `sequence`, so echo a wire-impossible sentinel (-1 is below
        // the wire floor of 0) — the deny is still observable and
        // request_id-correlatable, and a well-formed sender never produces
        // a non-numeric sequence.
        let sequence = match doc.get("sequence") {
            Some(value) => value.as_i64().unwrap_or(-1),
            None => return Ok(()), // stray — no echo to answer
        };
        let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let mut error = Map::new();
        error.insert("code".into(), Value::String(code.to_owned()));
        error.insert("message".into(), Value::String(message.to_owned()));
        if let Some(details) = details {
            error.insert("details".into(), Value::Object(details.clone()));
        }
        error.insert("extensions".into(), json!({}));
        let signed = authenticate_invoke_response_with_op(
            &self.local_secret,
            &InvokeResponseSignInput::Error {
                session_id: session_id.to_owned(),
                sequence,
                request_id: request_id.to_owned(),
                error: serde_json::from_value(Value::Object(error)).map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("error envelope build failed: {error}"),
                    )
                })?,
            },
            HashMap::new(),
            doc.get("op").and_then(Value::as_str),
        )
        .map_err(|error| {
            ResponderError::new(
                ResponderErrorKind::Transport,
                format!("response sign failed: {error}"),
            )
        })?;
        self.send_reverse_response(
            &serde_json::to_value(signed).expect("signed response serializes"),
        );
        Ok(())
    }

    /// Sign + send a success-branch response (raw port value, D4 shape).
    async fn send_ok_response(&self, doc: &Value, payload: Value) -> Result<(), ResponderError> {
        let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(sequence) = doc.get("sequence").and_then(Value::as_i64) else {
            return Ok(());
        };
        let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let payload = if payload.is_null() {
            json!({})
        } else {
            payload
        };
        let signed = authenticate_invoke_response_with_op(
            &self.local_secret,
            &InvokeResponseSignInput::Success {
                session_id: session_id.to_owned(),
                sequence,
                request_id: request_id.to_owned(),
                payload,
            },
            HashMap::new(),
            doc.get("op").and_then(Value::as_str),
        )
        .map_err(|error| {
            ResponderError::new(
                ResponderErrorKind::Transport,
                format!("response sign failed: {error}"),
            )
        })?;
        self.send_reverse_response(
            &serde_json::to_value(signed).expect("signed response serializes"),
        );
        Ok(())
    }

    // ── Reverse-invoke response demux (request_id → waiter) ───────────────

    /// Demux a response envelope to its pending reverse-invoke waiter.
    /// Correlation echo check first (non-mutating wire-position validation),
    /// then envelope-auth verify against the dialer's hello public key. A
    /// forged/tampered response fails closed — only this waiter, never a
    /// session-state mutation.
    async fn demux_response(&self, doc: &Value) {
        let Some(correlation) = wire_response_correlation(doc) else {
            return;
        };
        let request_id = correlation.request_id.clone();
        let entry = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(&request_id);
        let Some(entry) = entry else {
            return; // unknown/duplicate response — dropped
        };
        entry.timeout_task.abort();
        let (client_pubkey, session_id) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                // The session vanished mid-loop (close raced this response):
                // fail every pending waiter and transition to `Closed`.
                drop(session_guard);
                self.close_session("session state missing in serve loop");
                return;
            };
            (session.client_pubkey, session.session_id.clone())
        };
        // Correlation echo check first, then envelope-auth verify (contract
        // §7), then typed deserialization (the signer signed exactly the
        // wire branch, so it cannot fail).
        let outcome = check_response_correlation(&entry.correlation, &correlation)
            .map_err(|_| {
                ResponderError::new(
                    ResponderErrorKind::CorrelationMismatch,
                    "response echo fields do not match the request",
                )
            })
            .and_then(|()| {
                verify_invoke_response_auth_with_op(
                    &client_pubkey,
                    doc,
                    &session_id,
                    Some(&entry.op),
                )
                .map_err(|error| {
                    match error.kind() {
                        Some(kind) => ResponderError::new(
                            match kind {
                                crate::core::EnvelopeAuthErrorKind::Missing => {
                                    ResponderErrorKind::EnvelopeAuthMissing
                                }
                                crate::core::EnvelopeAuthErrorKind::Invalid => {
                                    ResponderErrorKind::EnvelopeAuthInvalid
                                }
                                crate::core::EnvelopeAuthErrorKind::SessionUnbound => {
                                    ResponderErrorKind::EnvelopeAuthSessionUnbound
                                }
                            },
                            error.to_string(),
                        ),
                        None => ResponderError::new(
                            ResponderErrorKind::EnvelopeAuthInvalid,
                            error.to_string(),
                        ),
                    }
                })
            })
            .and_then(|()| {
                serde_json::from_value::<ConnectInvokeResponse>(doc.clone()).map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("response decode failed: {error}"),
                    )
                })
            });
        let _ = entry.tx.try_send(outcome);
    }

    // ── Reverse invoke path (outbound counter → sign → send tail → waiter) ──

    /// Send one reverse invoke and resolve with its correlated response
    /// envelope. Errors with [`ResponderError`] on timeout / transport
    /// failure / session close / correlation mismatch / sequence exhaustion.
    ///
    /// Outbound wire-order serialization (contract §5.3 / §10): the send
    /// tail lock is acquired BEFORE the outbound sequence allocation
    /// (mirroring the adapter's lock-first `invoke_op`), so concurrent
    /// reverse invokes reach the wire in exactly the order they allocated.
    /// The waiter is registered BEFORE the tail (its timeout clock starts at
    /// call time), so a timeout that fires while a send is queued behind the
    /// tail is observable as a settled waiter (the deferred-send
    /// poison-close mirror): a waiter that settled while its send was still
    /// queued means the allocated outbound sequence never hit the wire —
    /// close the session (the peer's inbound gate would be stuck at that
    /// sequence) instead of leaving a silently poisoned session or
    /// transmitting late (a duplicate dispatch on the peer).
    async fn invoke_op(
        &self,
        op: &str,
        payload: Value,
    ) -> Result<ConnectInvokeResponse, ResponderError> {
        let state = *self.state.lock().expect("state lock");
        if state != ConnectResponderState::Established {
            return Err(ResponderError::new(
                ResponderErrorKind::SessionClosed,
                format!("connect session is not established (state {state})"),
            ));
        }
        // Waiter registration FIRST (call-time clock, BEFORE the send tail):
        // the request_id + waiter channel + timeout task + a provisional
        // pending entry are created synchronously, so a reverse invoke that
        // queues behind the tail still runs its timeout — a waiter that
        // settles while its send is queued is observable (the deferred-send
        // poison-close mirror; TS `#invokeOp` starts its timer before the
        // send-chain tail wait). The entry's correlation is completed with
        // the allocated sequence once the tail is held below; no response
        // can arrive before the send, so the provisional entry is never
        // demuxed.
        let session_id = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Err(ResponderError::new(
                    ResponderErrorKind::SessionClosed,
                    "connect session is not established",
                ));
            };
            session.session_id.clone()
        };
        let request_id = generate_request_id().map_err(|error| {
            ResponderError::new(ResponderErrorKind::Transport, error.to_string())
        })?;
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let timeout_task = {
            let pending = Arc::clone(&self.pending);
            let tx_timeout = tx.clone();
            let timeout = self.invoke_timeout;
            let request_id = request_id.clone();
            let op_owned = op.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                let fired = pending
                    .lock()
                    .expect("pending lock")
                    .remove(&request_id)
                    .is_some();
                if fired {
                    let _ = tx_timeout.try_send(Err(ResponderError::new(
                        ResponderErrorKind::Timeout,
                        format!(
                            "invoke {op_owned} ({request_id}) timed out after {}ms",
                            timeout.as_millis()
                        ),
                    )));
                }
            })
        };
        self.pending.lock().expect("pending lock").insert(
            request_id.clone(),
            PendingReverseInvoke {
                correlation: Correlation {
                    session_id: session_id.clone(),
                    sequence: -1, // provisional — completed with the allocated sequence below
                    request_id: request_id.clone(),
                },
                tx: tx.clone(),
                op: op.to_owned(),
                timeout_task,
            },
        );

        // Send-tail serialization (wire-order fix): the tail is acquired
        // BEFORE the outbound allocation, mirroring the adapter's lock-first
        // `invoke_op` — concurrent reverse invokes reach the wire in exactly
        // the order they allocated (contract §5.3 / §10; the dialer's strict
        // inbound gate rejects an out-of-order request).
        let _send_guard = self.send_tail.lock().await;

        // Outbound sequence allocation (synchronous, under the tail).
        let sequence = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                self.drop_pending(&request_id);
                return Err(ResponderError::new(
                    ResponderErrorKind::SessionClosed,
                    "connect session is not established",
                ));
            };
            let allocate_result = session.outbound.lock().expect("outbound lock").allocate();
            match allocate_result {
                Ok(sequence) => sequence,
                Err(_) => {
                    drop(session_guard);
                    self.drop_pending(&request_id);
                    // Outbound sequence exhaustion: the session is unusable
                    // (no wrap); close it and fail this invoke with
                    // `sequence_exhausted`.
                    self.close_session("outbound sequence exhausted");
                    return Err(ResponderError::new(
                        ResponderErrorKind::SequenceExhausted,
                        "outbound sequence space exhausted — reopen session",
                    ));
                }
            }
        };
        // The wire request to sign (`spoke-connect-invoke-request-jcs-v1`,
        // envelope-auth contract §2/§3): the signed object covers
        // `{session_id, sequence, request_id, op, payload}`.
        let signed = authenticate_invoke_request(
            &self.local_secret,
            &InvokeRequestSignInput {
                session_id,
                sequence: sequence as i64,
                request_id: request_id.clone(),
                op: op.to_owned(),
                payload,
                auth: None,
            },
            HashMap::new(),
        )
        .map_err(|error| {
            self.drop_pending(&request_id);
            ResponderError::new(
                ResponderErrorKind::Transport,
                format!("envelope auth signing failed: {error}"),
            )
        })?;

        // Complete the waiter's correlation with the allocated sequence (the
        // entry is only demuxed after the send below).
        {
            let mut pending = self.pending.lock().expect("pending lock");
            if let Some(entry) = pending.get_mut(&request_id) {
                entry.correlation.sequence = sequence as i64;
            }
        }

        // Poison check (deferred-send poison-close mirror): a waiter that
        // settled while its send was queued behind the tail means the
        // allocated outbound sequence never hit the wire — close the session
        // and settle with the error the waiter already observed (the
        // timeout/close message is queued on the channel).
        if !self
            .pending
            .lock()
            .expect("pending lock")
            .contains_key(&request_id)
        {
            self.close_session(
                "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
            );
            return match rx.recv().await {
                Some(result) => result,
                None => Err(ResponderError::new(
                    ResponderErrorKind::SessionClosed,
                    "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                )),
            };
        }

        // Encode + send. A synchronous encode failure or an async send
        // failure settles this invoke now — no dead entry waits out the
        // timeout.
        let envelope = match serde_json::to_vec(&signed) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.fail_pending_send(&request_id, &format!("invoke encode failed: {error}"));
                return Err(ResponderError::new(
                    ResponderErrorKind::Transport,
                    format!("invoke encode failed: {error}"),
                ));
            }
        };
        if let Err(error) = self.transport.send(&envelope).await {
            self.fail_pending_send(&request_id, &format!("invoke send failed: {error}"));
            return Err(ResponderError::new(
                ResponderErrorKind::Transport,
                format!("invoke send failed: {error}"),
            ));
        }
        // Wire order is settled — release the send tail so later-allocated
        // invokes can transmit while this one awaits its correlated response.
        drop(_send_guard);

        match rx.recv().await {
            Some(result) => result,
            None => Err(ResponderError::new(
                ResponderErrorKind::Transport,
                "invoke channel closed unexpectedly",
            )),
        }
    }

    /// Cleanup for a send/encode failure: remove the pending entry, abort its
    /// timeout task, and settle the waiter with a `transport` error.
    fn fail_pending_send(&self, request_id: &str, message: &str) {
        if let Some(entry) = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(request_id)
        {
            entry.timeout_task.abort();
            let _ = entry.tx.try_send(Err(ResponderError::new(
                ResponderErrorKind::Transport,
                message.to_owned(),
            )));
        }
    }

    /// Remove a registered waiter WITHOUT settling (aborting its timeout
    /// task). Used by the pre-send failure paths (allocation / sign) after
    /// the provisional registration — the invoke never reached the wire, so
    /// the caller observes the error directly and the waiter channel is
    /// dropped with this function's caller.
    fn drop_pending(&self, request_id: &str) {
        if let Some(entry) = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(request_id)
        {
            entry.timeout_task.abort();
        }
    }
}

/// Options for [`connect_responder`] (frozen contract §6; mirror of TS
/// `ConnectResponderOptions`).
#[derive(Clone)]
pub struct ConnectResponderOptions {
    /// Message-oriented transport (consumer-provided; loopback ships in-repo).
    pub transport: Arc<dyn Transport>,
    /// This responder's raw Ed25519 seed (hello + envelope-auth identity).
    pub identity: RemoteIdentity,
    /// Host manifest advertised in the responder's signed hello.
    pub manifest: HostCapabilityManifest,
    /// Trusted dialer peer ids (fail-closed allowlist).
    pub allowlist: Vec<String>,
    /// Preconfigured dialer Ed25519 public keys by peer_id. Key distribution
    /// is transport-adapter-owned per the spec; the responder knows its
    /// trusted identities statically. A dialing peer on `allowlist` without
    /// a preconfigured key fails the handshake (fail-closed).
    pub peer_keys: HashMap<String, [u8; 32]>,
    /// Local async [`RemoteServePorts`] served on the remote side via the
    /// D4 catalogue. Absent `ports` still answers `port.*` invokes with the
    /// dispatch-deny branch (documented behavior). Baseline-only providers
    /// migrate via `RemoteServePortsComposite::new(baseline, None, None)`.
    pub ports: Option<Arc<dyn RemoteServePorts + Send + Sync>>,
    /// Bounded-wait deadline for each reverse-invoke waiter AND the local
    /// serve-wait budget bounding every served `port.*` provider call and
    /// the whole `extract` service call, ms (default
    /// [`DEFAULT_INVOKE_TIMEOUT_MS`]). One value, two uses — a host setting
    /// zero for immediate serve-timeout semantics also collapses the
    /// reverse-invoke waiter deadline: zero answers the serve timeout with
    /// zero provider calls AND expires every reverse-invoke waiter
    /// immediately, so it is not a serve-only switch. The value is local: it
    /// does not inherit the dialer's budget and does not promise the reply
    /// beats the dialer's timeout.
    pub invoke_timeout_ms: Option<u64>,
}

/// Start a connect responder over `transport`: performs the signed hello
/// exchange + session snapshot in the background, then serves invokes. The
/// returned responder is in `Handshaking` until the dialer's hello arrives;
/// a handshake rejection closes the transport so the dial fails fast.
///
/// Divergence note (parity table): the TS factory validates
/// `identity.seed.length === 32` and throws; the Rust `RemoteIdentity` seed
/// is a `[u8; 32]` (type-enforced), so the factory has no error path and
/// returns the responder directly — handshake failures surface via the
/// `state` transition to `Closed` exactly like TS.
pub async fn connect_responder(options: ConnectResponderOptions) -> Arc<ConnectResponder> {
    let responder = Arc::new(ConnectResponder::new(options));
    *responder.state.lock().expect("state lock") = ConnectResponderState::Handshaking;
    let runner = Arc::clone(&responder);
    tokio::spawn(async move {
        runner.run().await;
    });
    responder
}

#[cfg(test)]
mod tests {

    // ── Scoped-put null-shape checks (F-016/F-018) ─────────────────────

    fn minimal_put_entry() -> Value {
        json!({
            "schema_version": 1,
            "entry_id": "ke_x",
            "entry_type": "note",
            "canonical_name": "x",
            "status": "confirmed",
            "revision": 1,
            "body": {},
            "extensions": {},
        })
    }

    fn decode_put(entry: Value) -> SpokeResult<KnowledgeCall> {
        decode_knowledge_call(
            "port.knowledge.put",
            &json!({ "entry": entry, "expected_base_revision": Value::Null }),
            None,
        )
    }

    /// qc3-05 — every revision slot of `port.knowledge.put` is compared on
    /// its exact integer representation: an integral-float spelling decodes
    /// by its parsed value, an exact integer past the endpoint refuses
    /// instead of rounding back into the domain.
    #[test]
    fn scoped_put_decodes_float_revisions_and_refuses_exact_overflow() {
        for (spelling, expected) in [
            (json!(3), 3_u64),
            (json!(3.0), 3_u64),
            (json!(9007199254740992_u64), 9007199254740992_u64),
            (json!(9007199254740992.0), 9007199254740992_u64),
        ] {
            let mut entry = minimal_put_entry();
            entry["revision"] = spelling.clone();
            let payload = json!({
                "entry": entry,
                "expected_base_revision": spelling.clone(),
            });
            let call = decode_knowledge_call("port.knowledge.put", &payload, None);
            let SpokeResult::Ok(KnowledgeCall::PutKnowledgeEntry(entry, base_revision)) = call
            else {
                panic!("revision={spelling} must decode")
            };
            assert_eq!(
                entry.revision,
                Some(expected),
                "entry.revision={spelling} must decode by its parsed value"
            );
            assert_eq!(
                base_revision,
                Some(expected),
                "expected_base_revision={spelling} must decode by its parsed value"
            );
        }

        let rejects = |entry: Value, base: Value| {
            matches!(
                decode_knowledge_call(
                    "port.knowledge.put",
                    &json!({ "entry": entry, "expected_base_revision": base }),
                    None,
                ),
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::InvalidInput,
                    ..
                })
            )
        };
        for refused in [
            json!(9007199254740993_u64),
            json!(9007199254740994_u64),
            json!(1.5),
            json!(-1),
        ] {
            let mut entry = minimal_put_entry();
            entry["revision"] = refused.clone();
            assert!(rejects(entry, Value::Null), "entry.revision={refused}");
            assert!(
                rejects(minimal_put_entry(), refused.clone()),
                "expected_base_revision={refused}"
            );
            let mut entry = minimal_put_entry();
            entry["adoption_anchor"] = json!({
                "schema_version": 1,
                "source_scope_id": "spc_local",
                "source_entry_id": "ke_src",
                "source_revision": refused.clone(),
                "extensions": {},
            });
            assert!(
                rejects(entry, Value::Null),
                "adoption_anchor.source_revision={refused}"
            );
        }
    }

    fn minimal_relation() -> Value {
        json!({
            "schema_version": 1,
            "relation_id": "rel_x",
            "relation_type": "related_to",
            "from_id": "ke_a",
            "to_id": "ke_b",
            "extensions": {},
        })
    }

    fn decode_relation_put(payload: Value) -> SpokeResult<PortCall<'static>> {
        decode_port_call("port.relation.put", &payload, None)
    }

    /// qc2 Important 1 / qc3-05 / qc3-07 — the relation put fence admits
    /// the `2^53` revision endpoint and an integral-float base spelling,
    /// leaves the open `metadata` object untouched, and refuses an exact
    /// revision beyond the domain before the provider.
    #[test]
    fn relation_put_admits_the_endpoint_and_refuses_oversized_revisions() {
        let mut relation = minimal_relation();
        relation["revision"] = json!(9_007_199_254_740_992_u64);
        relation["metadata"] = json!({ "schema_version": "product-v2" });
        for base in [Value::Null, json!(1), json!(1.0)] {
            let payload = json!({
                "relation": relation,
                "expected_base_revision": base,
            });
            let call = decode_relation_put(payload);
            let SpokeResult::Ok(PortCall::PutRelation(decoded, expected)) = call else {
                panic!("base={base}: an endpoint revision + opaque metadata must decode")
            };
            assert_eq!(decoded.revision, Some(9_007_199_254_740_992));
            assert_eq!(
                decoded.metadata.get("schema_version"),
                Some(&json!("product-v2")),
                "metadata is opaque"
            );
            assert_eq!(
                expected,
                if base.is_null() { None } else { Some(1) },
                "base={base}"
            );
        }

        for refused in [9_007_199_254_740_993_u64, 9_007_199_254_740_994_u64] {
            let mut relation = minimal_relation();
            relation["revision"] = json!(refused);
            let oversized_relation = decode_relation_put(json!({
                "relation": relation,
                "expected_base_revision": Value::Null,
            }));
            assert!(
                matches!(
                    oversized_relation,
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::InvalidInput,
                        ..
                    })
                ),
                "relation.revision={refused} must refuse before the provider"
            );
            let oversized_base = decode_relation_put(json!({
                "relation": minimal_relation(),
                "expected_base_revision": refused,
            }));
            assert!(
                matches!(
                    oversized_base,
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::InvalidInput,
                        ..
                    })
                ),
                "expected_base_revision={refused} must refuse before the provider"
            );
        }
    }

    /// C-03 — an integral-float `schema_version` names the same wire number
    /// the typed decoder and TypeScript read, so the raw scan admits it and
    /// the generated visitor decodes the parsed integer; the `2^53` endpoint
    /// is outside the SchemaVersion domain in either spelling.
    #[test]
    fn relation_put_reads_schema_version_by_its_parsed_value() {
        let mut relation = minimal_relation();
        relation["schema_version"] = json!(1.0);
        let call = decode_relation_put(json!({
            "relation": relation,
            "expected_base_revision": Value::Null,
        }));
        let SpokeResult::Ok(PortCall::PutRelation(decoded, _)) = call else {
            panic!("schema_version 1.0 must decode to the parsed 1")
        };
        assert_eq!(decoded.schema_version.get(), 1);

        for refused in [
            json!(9_007_199_254_740_992_u64),
            json!(9_007_199_254_740_992.0),
        ] {
            let mut relation = minimal_relation();
            relation["schema_version"] = refused.clone();
            assert!(
                matches!(
                    decode_relation_put(json!({
                        "relation": relation,
                        "expected_base_revision": Value::Null,
                    })),
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::InvalidInput,
                        ..
                    })
                ),
                "schema_version={refused} must refuse"
            );
        }
    }

    fn minimal_finding() -> Value {
        json!({
            "schema_version": 1,
            "finding_id": "fnd_x",
            "severity": "minor",
            "status": "open",
            "title": "title",
            "description": "description",
            "extensions": {},
        })
    }

    /// `Finding.text_position` is product data: a member spelled
    /// `schema_version` reaches the provider untouched, while the finding's
    /// own schema-owned `schema_version` stays bounded.
    #[test]
    fn finding_put_admits_opaque_text_position_and_bounds_schema_version() {
        let mut finding = minimal_finding();
        finding["text_position"] = json!({ "schema_version": "draft", "revision": "draft" });
        let call = decode_port_call("port.finding.put", &json!({ "findings": [finding] }), None);
        let SpokeResult::Ok(PortCall::PutFindings(findings)) = call else {
            panic!("an opaque text_position must reach the provider")
        };
        assert_eq!(
            findings[0].text_position.get("schema_version"),
            Some(&json!("draft")),
            "text_position is opaque"
        );

        let mut finding = minimal_finding();
        finding["schema_version"] = json!(9_007_199_254_740_992_u64);
        assert!(
            matches!(
                decode_port_call("port.finding.put", &json!({ "findings": [finding] }), None),
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::InvalidInput,
                    ..
                })
            ),
            "the finding's own schema_version is schema-owned"
        );
    }

    /// Every declared nested member is non-nullable: an explicit `null`
    /// inside `body`, `source_anchor`, `source_anchor.span` or
    /// `body.attributes[]` is malformed `INVALID_INPUT`, never the
    /// absent-member collapse (qc1 F-018 / qc3 F-016 residual — TS parity:
    /// `isValidKnowledgeEntryDeep` refuses them before the store).
    #[test]
    fn scoped_put_refuses_null_nested_entry_members() {
        let mut entry = minimal_put_entry();
        entry["body"]["summary"] = Value::Null;
        assert!(matches!(
            decode_put(entry),
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                ..
            })
        ));

        let mut entry = minimal_put_entry();
        entry["source_anchor"] = json!({
            "schema_version": 1,
            "source_id": "src_1",
            "span": Value::Null,
            "extensions": {},
        });
        assert!(matches!(
            decode_put(entry),
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                ..
            })
        ));

        let mut entry = minimal_put_entry();
        entry["source_anchor"] = json!({
            "schema_version": 1,
            "source_id": "src_1",
            "span": { "start": Value::Null, "end": 5 },
            "extensions": {},
        });
        assert!(matches!(
            decode_put(entry),
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                ..
            })
        ));

        let mut entry = minimal_put_entry();
        entry["body"]["attributes"] = json!([{
            "trait_type": "kind",
            "value": "v",
            "display_type": Value::Null,
        }]);
        assert!(matches!(
            decode_put(entry),
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                ..
            })
        ));

        // Nested extension-bag `null` stays legal — `extensions` values
        // are opaque, not declared members.
        let mut entry = minimal_put_entry();
        entry["extensions"]["ns"] = json!({ "k": Value::Null });
        assert!(matches!(decode_put(entry), SpokeResult::Ok(_)));
    }
    use super::*;
    use crate::remote::transport::loopback_transport_pair;

    fn test_manifest() -> HostCapabilityManifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": "test-host",
            "roles": ["data-store"],
            "capabilities": ["spoke-memory"],
            "namespaces": ["toy_world"],
            "extensions": {},
        }))
        .expect("valid manifest")
    }

    // ── Cognition capability / raw requirement vectors ────────────────────
    //
    // The vectors are the SAME file the TS suite consumes
    // (`packages/spoke-connect-ts/tests/fixtures/cognition-negotiation.json`),
    // so both languages agree on the host manifests, payloads and exact
    // rejection fields.

    use crate::cognition::COGNITION_FIXTURE;

    fn capability_list(host: &Value) -> Vec<String> {
        host.get("capabilities")
            .and_then(Value::as_array)
            .expect("host capabilities")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn cognition_negotiation_vectors_match_the_family_intersection() {
        let fixture: Value = serde_json::from_str(COGNITION_FIXTURE).expect("fixture json");
        let cases = fixture
            .get("cases")
            .and_then(Value::as_array)
            .expect("fixture cases");
        let mut negotiated_cases = 0;
        for case in cases {
            if case.get("kind").and_then(Value::as_str) != Some("negotiation") {
                continue;
            }
            negotiated_cases += 1;
            let local = capability_list(&json!({
                "capabilities": case.get("local_capabilities"),
            }));
            let remote = capability_list(&json!({
                "capabilities": case.get("remote_capabilities"),
            }));
            let negotiated = negotiated_capabilities(&local, &remote);
            assert_eq!(
                negotiated
                    .iter()
                    .any(|cap| cap == CAPABILITY_SPOKE_COGNITION),
                case.get("negotiated_includes_cognition")
                    .and_then(Value::as_bool)
                    .expect("negotiated_includes_cognition"),
                "{case}"
            );
        }
        assert!(negotiated_cases >= 5, "negotiation vectors present");
    }

    /// `gate_allows` (the serve gate) for the product-map knowledge ops: only
    /// an explicitly negotiated `spoke-knowledge` admits them — a baseline-only
    /// set does not — and a knowledge-only session is still refused a baseline
    /// op.
    #[tokio::test]
    async fn gate_allows_requires_knowledge_capability_for_the_knowledge_ops() {
        let pair = loopback_transport_pair();
        let responder = ConnectResponder::new(ConnectResponderOptions {
            transport: Arc::new(pair.server),
            identity: RemoteIdentity { seed: [0x21; 32] },
            manifest: test_manifest(),
            allowlist: Vec::new(),
            peer_keys: HashMap::new(),
            ports: None,
            invoke_timeout_ms: None,
        });
        let caps =
            |list: &[&str]| -> Vec<String> { list.iter().map(|cap| (*cap).to_owned()).collect() };
        for op in ["port.knowledge.get", "port.scope.list_knowledge_entries"] {
            assert!(
                responder.gate_allows(op, &caps(&["spoke-knowledge"])),
                "{op}"
            );
            assert!(
                responder.gate_allows(op, &caps(&["spoke-baseline", "spoke-knowledge"])),
                "{op}"
            );
            assert!(
                !responder.gate_allows(op, &caps(&["spoke-baseline"])),
                "{op}: spoke-baseline must not satisfy spoke-knowledge"
            );
            assert!(!responder.gate_allows(op, &caps(&["ke-ownership"])), "{op}");
            assert!(!responder.gate_allows(op, &caps(&[])), "{op}");
        }
        assert!(
            !responder.gate_allows(
                "port.scope.list_timeline_events",
                &caps(&["spoke-knowledge"])
            ),
            "a knowledge-only session must not serve a baseline op"
        );
        assert!(responder.gate_allows(
            "port.scope.list_timeline_events",
            &caps(&["spoke-baseline"])
        ));
        responder.close();
    }

    /// Original first-denial preserved: a malformed baseline Scope answers
    /// its shape failure BEFORE the conditional `ke-ownership` conjunct, and a
    /// cognition-carrying payload whose baseline shape is malformed keeps that
    /// shape denial too instead of borrowing the §2.2 verdict.
    #[tokio::test]
    async fn malformed_scope_keeps_its_shape_denial_before_ownership_and_cognition() {
        let pair = loopback_transport_pair();
        let responder = Arc::new(ConnectResponder::new(ConnectResponderOptions {
            transport: Arc::new(pair.server),
            identity: RemoteIdentity { seed: [0x33; 32] },
            manifest: test_manifest(),
            allowlist: Vec::new(),
            peer_keys: HashMap::new(),
            ports: None,
            invoke_timeout_ms: None,
        }));
        let negotiated = vec![
            crate::core::CAPABILITY_SPOKE_BASELINE.to_owned(),
            CAPABILITY_SPOKE_COGNITION.to_owned(),
        ];
        // `scope_id` is required, so the Scope is malformed while its
        // viewpoint is a valid ownership trigger. `ke-ownership` is not
        // negotiated: the malformed shape must still answer first.
        let doc = json!({
            "session_id": "shape-first-session",
            "sequence": 0,
            "request_id": "shape-first-request",
            "op": "port.scope.list_knowledge_entries",
            "payload": {"scope": {"viewpoint": "kb_mira"}},
        });
        responder
            .dispatch_port_invoke(&doc, &negotiated)
            .await
            .expect("dispatch answers");
        let frame = pair.client.recv().await.expect("error frame");
        let response: Value = serde_json::from_slice(&frame).expect("response json");
        assert_eq!(response["error"]["code"], "INVALID_INPUT", "{response}");

        // Same payload plus an unsupported cognition requirement: the
        // baseline shape failure still wins, because the deferred raw §2.2
        // diagnosis only answers payloads whose baseline Scope is sound.
        let doc = json!({
            "session_id": "shape-first-session",
            "sequence": 1,
            "request_id": "shape-first-request-2",
            "op": "port.scope.list_knowledge_entries",
            "payload": {
                "scope": {
                    "viewpoint": "kb_mira",
                    "cognition": {"view": "current", "filter": {"q_min": 0.5}},
                },
            },
        });
        responder
            .dispatch_port_invoke(&doc, &negotiated)
            .await
            .expect("dispatch answers");
        let frame = pair.client.recv().await.expect("error frame");
        let response: Value = serde_json::from_slice(&frame).expect("response json");
        assert_eq!(response["error"]["code"], "INVALID_INPUT", "{response}");
        assert!(
            response["error"].get("details").is_none(),
            "the baseline shape denial carries no §2.2 verdict: {response}"
        );
    }

    /// Direct regression for the defensive non-string-op arm: the dispatch
    /// gate routes only string ops here, so a caller that bypasses it must
    /// still get a terminal `INVALID_INPUT` response — never the silent drop
    /// that leaves the sender waiting.
    #[tokio::test]
    async fn dispatch_memory_invoke_answers_invalid_input_for_a_non_string_op() {
        let pair = loopback_transport_pair();
        let responder = Arc::new(ConnectResponder::new(ConnectResponderOptions {
            transport: Arc::new(pair.server),
            identity: RemoteIdentity { seed: [0x11; 32] },
            manifest: test_manifest(),
            allowlist: Vec::new(),
            peer_keys: HashMap::new(),
            ports: None,
            invoke_timeout_ms: None,
        }));
        let doc = json!({
            "session_id": "test-session",
            "sequence": 0,
            "request_id": "test-request",
            "op": 42,
            "payload": {},
        });
        responder
            .dispatch_memory_invoke(&doc, &[])
            .await
            .expect("non-string op dispatch answers");
        let frame = pair.client.recv().await.expect("error frame");
        let response: Value = serde_json::from_slice(&frame).expect("response json");
        assert_eq!(response["error"]["code"], "INVALID_INPUT", "{response}");
        assert_eq!(
            response["error"]["details"]["retry_class"], "terminal",
            "§14.2: the defensive memory reject is classified terminal: {response}"
        );
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("op must be a string"),
            "{response}"
        );
        responder.close();
    }
    // ── Scoped put: anchor pre-read regression (QC2-11 / F-012) + CAS
    //    binding (contract §Anchor immutability) ─────────────────────────

    /// Minimal scoped store: `get` answers the injected absence code or a
    /// stored entry; `put` records the CAS base it was called with.
    struct ScopedPutStore {
        stored: Option<KnowledgeEntry>,
        get_code: SpokeRejectCode,
        cas_bases: Mutex<Vec<Option<u64>>>,
    }

    #[async_trait::async_trait]
    impl spoke_operations::ScopedKnowledgePort for ScopedPutStore {
        async fn get_scoped_knowledge_entry(
            &self,
            _scope: &Scope,
            _entry_id: &str,
        ) -> SpokeResult<KnowledgeEntry> {
            match &self.stored {
                Some(entry) => spoke_ok(entry.clone()),
                None => spoke_reject(self.get_code, "entry absent", None),
            }
        }

        async fn put_scoped_knowledge_entry(
            &self,
            _scope: &Scope,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            self.cas_bases
                .lock()
                .expect("cas_bases")
                .push(expected_base_revision);
            spoke_ok(entry)
        }

        async fn list_scoped_knowledge_entries(
            &self,
            _scope: &Scope,
        ) -> SpokeResult<Vec<KnowledgeEntry>> {
            spoke_ok(Vec::new())
        }
    }

    fn put_entry(entry_id: &str, anchor: Option<Value>) -> KnowledgeEntry {
        let mut entry = json!({
            "schema_version": 1,
            "entry_id": entry_id,
            "canonical_name": "fixture",
            "entry_type": "note",
            "status": "active",
            "body": {},
            "extensions": {},
        });
        if let Some(anchor) = anchor {
            entry["adoption_anchor"] = anchor;
        }
        serde_json::from_value(entry).expect("valid KnowledgeEntry")
    }

    fn put_anchor() -> Value {
        json!({
            "schema_version": 1,
            "source_scope_id": "src-scope",
            "source_entry_id": "src-entry",
            "source_revision": 3,
            "extensions": {},
        })
    }

    fn put_scope() -> Scope {
        serde_json::from_value(json!({"scope_id": "s1", "extensions": {}})).expect("valid Scope")
    }

    /// QC2-11 / qc1 F-012 regression: a scoped `get` answering
    /// `KNOWLEDGE_ENTRY_NOT_FOUND` is an absent destination — a
    /// create-only put (caller base `None`) proceeds as a create, not a
    /// refusal. A non-null caller base on an absent entry is an update
    /// on a missing entry and refuses `STORED_REVISION_STALE` before
    /// any write (QC2-12/F-014: the wire fence is never substituted).
    #[tokio::test]
    async fn scoped_put_creates_when_the_store_answers_entry_not_found() {
        let store = Arc::new(ScopedPutStore {
            stored: None,
            get_code: SpokeRejectCode::KnowledgeEntryNotFound,
            cas_bases: Mutex::new(Vec::new()),
        });
        let result = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(put_entry("new-entry", None), None),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        let SpokeResult::Ok(entry) = result else {
            panic!("expected create success, got {result:?}")
        };
        assert_eq!(entry["entry_id"], "new-entry");
        assert_eq!(
            *store.cas_bases.lock().expect("cas_bases"),
            vec![None],
            "a create carries no CAS base"
        );

        // An update fence on an absent entry refuses before any write.
        let stale = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(put_entry("absent-entry", None), Some(7)),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        assert!(
            matches!(
                stale,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::StoredRevisionStale,
                    ..
                })
            ),
            "an update base on an absent entry refuses: {stale:?}"
        );
        assert_eq!(
            store.cas_bases.lock().expect("cas_bases").len(),
            1,
            "the stale-base refusal happened before the write"
        );
    }

    /// Caller OCC + §Anchor immutability CAS binding: the write's
    /// compare-and-swap base is the wire-supplied
    /// `expected_base_revision`, accepted only when it equals the stored
    /// snapshot the anchor check inspected — a stale base refuses
    /// `STORED_REVISION_STALE`, a create-only `None` on an occupied slot
    /// refuses `REVISION_CONFLICT`, and a matched base writes (QC2-12 /
    /// qc3 F-014).
    #[tokio::test]
    async fn scoped_put_honours_the_caller_occ_fence() {
        let mut stored_entry = put_entry("anchored-entry", Some(put_anchor()));
        stored_entry.revision = Some(5);
        let store = Arc::new(ScopedPutStore {
            stored: Some(stored_entry),
            get_code: SpokeRejectCode::KnowledgeEntryNotFound,
            cas_bases: Mutex::new(Vec::new()),
        });
        // Anchor preserved byte-identically + a stale wire base: the put
        // refuses before any store write.
        let stale = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(
                put_entry("anchored-entry", Some(put_anchor())),
                Some(2),
            ),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        assert!(
            matches!(
                stale,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::StoredRevisionStale,
                    ..
                })
            ),
            "a stale caller base refuses even when the anchor is unchanged: {stale:?}"
        );
        assert!(
            store.cas_bases.lock().expect("cas_bases").is_empty(),
            "the stale-base refusal happened before the write"
        );

        // A create-only request cannot replace the occupied slot.
        let create = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(put_entry("anchored-entry", Some(put_anchor())), None),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        assert!(
            matches!(
                create,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::RevisionConflict,
                    ..
                })
            ),
            "a create-only put on an occupied slot refuses: {create:?}"
        );

        // D1 refusal precedence (qc2 QC2-14 / qc3 F-016): the SAME
        // occupied-create request whose candidate would REMOVE the stored
        // anchor still refuses the create-only `REVISION_CONFLICT` — the
        // caller OCC fence evaluates before the anchor-immutability
        // check on the same snapshot, matching the TS responder.
        let create_removing_anchor = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(put_entry("anchored-entry", None), None),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        assert!(
            matches!(
                create_removing_anchor,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::RevisionConflict,
                    ..
                })
            ),
            "the create-only refusal precedes the anchor refusal: {create_removing_anchor:?}"
        );
        assert!(
            store.cas_bases.lock().expect("cas_bases").is_empty(),
            "neither overlapping refusal reached a write"
        );

        // The matched base writes — the CAS base IS the caller fence,
        // equal to the inspected revision.
        let result = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(
                put_entry("anchored-entry", Some(put_anchor())),
                Some(5),
            ),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        assert!(matches!(result, SpokeResult::Ok(_)), "{result:?}");
        assert_eq!(
            *store.cas_bases.lock().expect("cas_bases"),
            vec![Some(5)],
            "the CAS base is the caller fence, verified against the checked revision"
        );

        // Mutate-then-put still refuses ADOPTION_ANCHOR_IMMUTABLE before
        // any store write.
        let mutated = call_knowledge_op(
            KnowledgeCall::PutKnowledgeEntry(put_entry("anchored-entry", None), Some(5)),
            store.as_ref(),
            &put_scope(),
            None,
        )
        .await;
        assert!(matches!(
            mutated,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::AdoptionAnchorImmutable,
                ..
            })
        ));
        assert_eq!(
            store.cas_bases.lock().expect("cas_bases").len(),
            1,
            "the immutable-anchor refusal happened before the write"
        );
    }
}
