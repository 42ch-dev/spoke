//! RemoteAdapter + message-oriented Transport (frozen contract:
//! `.agents/specs/adapters/spoke-remote-adapter.md`).
//!
//! This module is gated behind the `remote-adapter` cargo feature;
//! `spoke-operations` is an unconditional dependency of every connect build
//! (the generic node dispatch shares its cognition-requirement diagnosis
//! regardless of this feature). The TS equivalent is the `./remote` subpath
//! export.
//!
//! Public surface: [`RemoteAdapter`] (async `BaselinePorts` + read-only
//! session info + `close` + tool serving via
//! [`RemoteAdapter::register_tool_handler`] / [`RemoteAdapter::invoke_tool`]
//! — reverse invokes served per the frozen §4 pipeline + the `extract` core
//! op via [`RemoteAdapter::extract`]),
//! [`connect_remote_adapter`] (dial), [`connect_responder`] (the
//! productized responder: handshake recipe + `port.*` serving through the
//! [`RemoteServePorts`] seam + the reverse `invoke_tool` face + the
//! optional [`RemoteExtractService`] `extract` face + the optional
//! [`RemoteHistoryService`] `history.*` face + the optional
//! [`RemoteAdoptionService`] `knowledge.*` face), the
//! [`Transport`] trait, the in-repo loopback transport pair for tests, and
//! the multi-peer capability router ([`MultiPeerRouter`] +
//! [`connect_multi_peer_router`] — capability-selected async `BaselinePorts`
//! over N registered per-peer adapters, frozen multi-peer routing contract).

use serde::Deserialize;
use serde_json::Value;
use spoke_operations::{SpokeReject, SpokeRejectCode};
use spoke_schemas::Scope;

use crate::core::{
    capability_satisfies, dispatch_allowed, reserved_prefix_op, CAPABILITY_L2_COMPUTABLE,
    CAPABILITY_SPOKE_BASELINE, CAPABILITY_SPOKE_KNOWLEDGE,
};

mod multi_peer_router;
mod remote_adapter;
mod responder;
mod serve_ports;
pub mod transport;

pub use multi_peer_router::{
    connect_multi_peer_router, select_peer_for_op, MultiPeerRouter, MultiPeerRouterError,
    MultiPeerRouterOptions, RoutedRemoteAdapter, SelectablePeer,
};
pub use remote_adapter::{
    connect_remote_adapter, reset_accepted_server_hellos_for_test, RemoteAdapter,
    RemoteAdapterError, RemoteAdapterOptions, RemoteAdapterState, RemoteIdentity, ToolHandler,
};
pub use responder::{
    connect_responder, ConnectResponder, ConnectResponderOptions, ConnectResponderState,
};
pub use serve_ports::{
    AdoptionAdmission, AdoptionPortService, HistoryAdmission, HistoryDedupLookup,
    HistoryDedupRecord, HistoryDedupStore, HistoryPortService, InMemoryHistoryDedupStore,
    KnowledgeAdmission, MemoryAdmission, MemoryPortService, RemoteAdoptionService,
    RemoteExtractService, RemoteHistoryService, RemoteKnowledgeFace, RemoteMemoryService,
    RemoteScopedKnowledgeFace, RemoteServePorts, RemoteServePortsComposite,
    ScopedAdoptionAdmission, ScopedHistoryAdmission, ScopedKnowledgeAdmission, HISTORY_OP_GET,
    HISTORY_OP_PRUNE, KNOWLEDGE_OP_ADOPT,
};
pub use transport::{
    loopback_transport_pair, LoopbackTransport, LoopbackTransportPair, Transport, TransportError,
};

// ── `port.*` capability map (D4 catalogue) ────────────────────────────────
//
// One product map, two readers: the responder's serve gate and the router's
// selection table. Core keeps its op-only table (`core::dispatch`, which
// answers `None` for `port.*` deliberately), so the product rows live here
// once instead of being re-declared per surface.

/// Optional family capability for the fork timeline query. The core
/// capability table has no `l5-fork` op row (the fork query is a remote
/// `port.*` op), so the product map that requires it owns the one canonical
/// spelling.
const CAPABILITY_L5_FORK: &str = "l5-fork";

/// Required capability for a `port.*` op of the D4 catalogue (frozen
/// contract §5.2), or `None` for an op outside it. Every baseline row
/// requires `spoke-baseline`; the three knowledge ops require
/// `spoke-knowledge` **explicitly** (the knowledge face is their only
/// server, so no flag implies it); the optional
/// families require their negotiated family capability (`l2-computable` /
/// `l5-fork`).
///
/// This is the requirement only — each surface keeps its own evaluation and
/// failure shape (serve-gate deny vs. selection reject), and neither surface
/// gains an op the other does not have.
pub(crate) fn port_op_requirement(op: &str) -> Option<&'static str> {
    match op {
        "port.knowledge.get" => Some(CAPABILITY_SPOKE_KNOWLEDGE),
        "port.knowledge.put" => Some(CAPABILITY_SPOKE_KNOWLEDGE),
        "port.relation.get" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.relation.put" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.scope.list_knowledge_entries" => Some(CAPABILITY_SPOKE_KNOWLEDGE),
        "port.scope.list_timeline_events" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.finding.put" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.rule.list" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.host.list_peer_manifests" => Some(CAPABILITY_SPOKE_BASELINE),
        // Optional families (D4 catalogue): project / compute require the
        // `l2-computable` family capability; the fork timeline query
        // requires `l5-fork` — all evaluated against the negotiated
        // capabilities (both-hello intersection).
        "port.computable.project" => Some(CAPABILITY_L2_COMPUTABLE),
        "port.computable.compute" => Some(CAPABILITY_L2_COMPUTABLE),
        "port.fork.list_timeline_events" => Some(CAPABILITY_L5_FORK),
        _ => None,
    }
}

/// The responder's combined serve gate for one op: the core op table first
/// (which also covers the self-describing `tools.*` family), then the product
/// `port.*` map.
///
/// A core-reserved prefix name with no core-table row is terminal at the core
/// boundary (spoke-knowledge-adoption contract §Connect operation): unknown
/// `knowledge.*` names refuse `op_unsupported` here and are ineligible for the
/// product fallback — custom mappings apply only outside reserved families.
///
/// Named and pure so the serve decision is asserted directly from the shared
/// capability matrix instead of only through the per-table lookups.
pub(crate) fn port_dispatch_allowed(op: &str, negotiated: &[String]) -> bool {
    if reserved_prefix_op(op) {
        return false;
    }
    dispatch_allowed(op, negotiated)
        || port_op_requirement(op)
            .is_some_and(|required| capability_satisfies(negotiated, required))
}

// ── `ke-ownership` conditional requirement (frozen F2) ─────────────────────
//
// One conclusion, three differently shaped integration sites: the responder
// evaluates it as a supplementary gate before probing or calling a provider,
// and the router applies it as a hard manifest filter. Core keeps its
// op-only table (see `core::dispatch`), so the payload-dependent rule lives
// here at the remote product boundary.

/// The remote ops that may declare `payload.scope` — the location the
/// ownership predicate and the declared-Scope validation read. Only these
/// carry a Scope; `extract` and the `tools.*` family never are
/// ownership-gated.
///
/// The three list/query ops require the member; `port.knowledge.get` / `put`
/// accept it optionally (A2-D6). Every entry still takes the `ke-ownership`
/// and `spoke-cognition` conjuncts when it declares the corresponding Scope
/// members.
pub(crate) const SCOPE_BEARING_OPS: [&str; 5] = [
    "port.scope.list_knowledge_entries",
    "port.scope.list_timeline_events",
    "port.fork.list_timeline_events",
    "port.knowledge.get",
    "port.knowledge.put",
];

/// The subset of [`SCOPE_BEARING_OPS`] whose wire request requires `scope`;
/// the remainder accept it optionally (a present scope is still validated).
const REQUIRED_SCOPE_OPS: [&str; 3] = [
    "port.scope.list_knowledge_entries",
    "port.scope.list_timeline_events",
    "port.fork.list_timeline_events",
];

/// Whether `op` accepts an **optional** declared `Scope` (the A2 knowledge
/// ops). Such a present scope demands the negotiated
/// `spoke-knowledge-scoped` marker; the required-scope ops keep their
/// `0.18.1` behaviour untouched.
pub(crate) fn optional_scope_op(op: &str) -> bool {
    SCOPE_BEARING_OPS.contains(&op) && !REQUIRED_SCOPE_OPS.contains(&op)
}

/// Whether a request targets one of [`SCOPE_BEARING_OPS`] **and** declares a
/// non-empty string viewpoint at `payload.scope.viewpoint`.
///
/// Declared-location predicate only — no trimming, normalization, holder
/// lookup, owner comparison or recursive search, and no audience expansion.
/// Whichever value it carries, `viewpoint` is a reader context, never a
/// credential.
pub(crate) fn requires_ownership_capability(op: &str, payload: &Value) -> bool {
    if !SCOPE_BEARING_OPS.contains(&op) {
        return false;
    }
    matches!(
        payload
            .get("scope")
            .and_then(Value::as_object)
            .and_then(|scope| scope.get("viewpoint")),
        Some(Value::String(viewpoint)) if !viewpoint.is_empty()
    )
}

/// Validate a Scope-bearing request's declared Scope: it must be a
/// well-formed `Scope`, and a present `viewpoint` must be a non-empty
/// string. A present JSON `null` is malformed here even though the generated
/// `Scope` decodes it as an absent viewpoint — the "no viewpoint" reading
/// belongs to the key being absent.
///
/// The three list/query ops require the member; `port.knowledge.get` / `put`
/// accept an absent one (A2-D6). Runs at the responder before the provider
/// probe and any host call, so a malformed Scope is an input failure rather
/// than a silent baseline success. Non-Scope-bearing ops are untouched.
///
/// Returns the decoded `Scope` for the ops that carry one, so the dispatch
/// catalogue serves that value instead of decoding the same JSON a second
/// time.
pub(crate) fn validate_scope_declaration(
    op: &str,
    payload: &Value,
) -> Result<Option<Scope>, SpokeReject> {
    if !SCOPE_BEARING_OPS.contains(&op) {
        return Ok(None);
    }
    let invalid = |detail: String| SpokeReject {
        code: SpokeRejectCode::InvalidInput,
        message: format!("invalid {op} payload: {detail}"),
        details: None,
    };
    let Some(scope) = payload.get("scope") else {
        return if REQUIRED_SCOPE_OPS.contains(&op) {
            Err(invalid("missing scope".to_owned()))
        } else {
            Ok(None)
        };
    };
    if let Some(viewpoint) = scope.as_object().and_then(|scope| scope.get("viewpoint")) {
        if !matches!(viewpoint, Value::String(value) if !value.is_empty()) {
            return Err(invalid(
                "scope.viewpoint must be a non-empty string".to_owned(),
            ));
        }
    }
    Scope::deserialize(scope)
        .map(Some)
        .map_err(|error| invalid(error.to_string()))
}

/// The baseline shape verdict for a payload that also declares the optional
/// `Scope.cognition` selection: the same declared-`Scope` gate with that one
/// member removed.
///
/// Used only after the full decode already failed on such a payload, to decide
/// whether the failure belongs to the baseline Scope (which keeps its original
/// shape-first denial) or to the new optional subtree, whose raw §2.2
/// diagnosis then answers instead. Same generated `Scope` decode — no second
/// schema, no re-implemented member list.
pub(crate) fn baseline_scope_shape(op: &str, payload: &Value) -> Result<(), SpokeReject> {
    let Some(scope) = payload.get("scope").and_then(Value::as_object) else {
        return validate_scope_declaration(op, payload).map(|_| ());
    };
    if !scope.contains_key("cognition") {
        return validate_scope_declaration(op, payload).map(|_| ());
    }
    let mut baseline = scope.clone();
    baseline.remove("cognition");
    validate_scope_declaration(op, &serde_json::json!({ "scope": baseline })).map(|_| ())
}

#[cfg(test)]
mod capability_matrix {
    use super::{port_dispatch_allowed, port_op_requirement, select_peer_for_op, SelectablePeer};
    use crate::core::{capability_satisfies, required_capability};
    use serde_json::{json, Value};
    use spoke_operations::SpokeResult;

    /// A payload valid for the row's op (Scope-bearing rows declare a Scope,
    /// no `viewpoint` — the row's capability is the only gate under test).
    fn payload_for(op: &str) -> Value {
        match op {
            "port.scope.list_knowledge_entries" | "port.scope.list_timeline_events" => {
                json!({ "scope": { "scope_id": "s" } })
            }
            "port.knowledge.get" => json!({ "entry_id": "e" }),
            "port.knowledge.put" => json!({ "entry": {} }),
            _ => json!({}),
        }
    }

    /// One candidate whose manifest advertises exactly the row's negotiated
    /// capabilities, so selection's answer follows from that set alone.
    fn candidate(negotiated: &[String]) -> SelectablePeer {
        SelectablePeer {
            peer_id: "peer-matrix".to_string(),
            manifest: serde_json::from_value(json!({
                "schema_version": 1,
                "host_id": "h-matrix",
                "roles": ["data-store"],
                "capabilities": negotiated,
                "namespaces": ["ns"],
                "extensions": {},
            }))
            .expect("manifest shaped with the row's negotiated capabilities"),
        }
    }

    // The crate reads its own fixture; byte equality with the TS copy is
    // enforced outside the crate by `tooling/connect/golden-vector-sync.mjs`,
    // so a packaged crate never depends on a sibling package's path.
    const FIXTURE: &str = include_str!("../../tests/fixtures/capability-matrix.json");

    #[test]
    fn the_op_tables_match_the_shared_capability_matrix() {
        let matrix: serde_json::Value =
            serde_json::from_str(FIXTURE).expect("capability matrix is valid JSON");
        let cases = matrix["cases"]
            .as_array()
            .expect("cases is an array of matrix rows");
        assert!(!cases.is_empty(), "the matrix must carry at least one case");

        for case in cases {
            let op = case["op"].as_str().expect("op is a string");
            let table = case["table"].as_str().expect("table is a string");
            let negotiated: Vec<String> = case["negotiated"]
                .as_array()
                .expect("negotiated is an array")
                .iter()
                .map(|cap| {
                    cap.as_str()
                        .expect("negotiated capabilities are strings")
                        .to_owned()
                })
                .collect();
            let expected_required = case["required"].as_str().expect("required is a string");
            let expected_allowed = case["allowed"].as_bool().expect("allowed is a boolean");

            let required = match table {
                "core" => required_capability(op),
                "product" => port_op_requirement(op),
                other => panic!("{op}: unknown table {other}"),
            };
            assert_eq!(
                required,
                Some(expected_required),
                "{op}: {table} table requirement drifted from the shared matrix"
            );
            assert_eq!(
                capability_satisfies(&negotiated, expected_required),
                expected_allowed,
                "{op}: dispatch verdict drifted for negotiated {negotiated:?}"
            );

            // The responder's actual combined gate, not just the row's owning
            // table: a core-table entry for a `port.*` op would otherwise pass
            // both lookups above while admitting the wrong session.
            assert_eq!(
                port_dispatch_allowed(op, &negotiated),
                expected_allowed,
                "{op}: serve gate drifted for negotiated {negotiated:?}"
            );
            if op.starts_with("port.") {
                assert!(
                    required_capability(op).is_none(),
                    "{op}: the core table must not claim a `port.*` op"
                );
            }

            // The real selection path, not just the table lookup: one
            // candidate carrying exactly this row's negotiated set must be
            // selected for an allowed row and rejected for a denied one.
            match (
                expected_allowed,
                select_peer_for_op(&[candidate(&negotiated)], op, &payload_for(op)),
            ) {
                (true, SpokeResult::Ok(peer)) => assert_eq!(
                    peer.peer_id, "peer-matrix",
                    "{op}: router selected an unexpected peer"
                ),
                (true, SpokeResult::Reject(reject)) => panic!(
                    "{op}: router rejected an allowed row for negotiated {negotiated:?}: {reject:?}"
                ),
                (false, SpokeResult::Ok(peer)) => panic!(
                    "{op}: router selected {} for a denied row with negotiated {negotiated:?}",
                    peer.peer_id
                ),
                (false, SpokeResult::Reject(_)) => {}
            }
        }
    }

    /// Reserved `knowledge.*` unknowns refuse at the core boundary: the
    /// combined gate never reaches the product-map fallback for an
    /// unregistered reserved name (spoke-knowledge-adoption contract
    /// §Connect operation), even with every capability negotiated.
    #[test]
    fn reserved_knowledge_unknowns_never_reach_the_product_map() {
        let negotiated: Vec<String> = [
            "spoke-baseline",
            "spoke-knowledge",
            "spoke-knowledge-adoption",
            "spoke-history",
        ]
        .iter()
        .map(|cap| (*cap).to_owned())
        .collect();
        for op in [
            "knowledge.made_up",
            "knowledge.entries.delete",
            "knowledge.adopt.v2",
        ] {
            assert!(
                !port_dispatch_allowed(op, &negotiated),
                "{op}: a reserved unknown must refuse before any product map"
            );
            assert!(
                matches!(
                    select_peer_for_op(&[candidate(&negotiated)], op, &payload_for(op)),
                    SpokeResult::Reject(_)
                ),
                "{op}: router selection must refuse a reserved unknown"
            );
        }
        // The registered row still serves under both required capabilities.
        assert!(
            port_dispatch_allowed("knowledge.adopt", &negotiated),
            "knowledge.adopt: the registered op must serve"
        );
        // A name outside the reserved family still takes the fallback — the
        // refusal is reservation, not prefix-blindness: `memory.*` is
        // core-owned (denied without its capability is irrelevant here), so
        // prove the fallback lives for a `port.*` row instead.
        assert!(
            port_dispatch_allowed("port.relation.get", &negotiated),
            "port.relation.get: the product map still applies outside reserved families"
        );
    }
}

#[cfg(test)]
mod scope_declaration {
    use super::{optional_scope_op, validate_scope_declaration};
    use serde_json::json;

    const REQUIRED_SCOPE_OPS: [&str; 3] = [
        "port.scope.list_knowledge_entries",
        "port.scope.list_timeline_events",
        "port.fork.list_timeline_events",
    ];

    #[test]
    fn get_and_put_accept_an_absent_scope_while_the_query_ops_require_one() {
        for op in REQUIRED_SCOPE_OPS {
            assert!(!optional_scope_op(op));
            assert!(
                validate_scope_declaration(op, &json!({})).is_err(),
                "{op}: a missing scope is an input failure"
            );
        }
        for op in ["port.knowledge.get", "port.knowledge.put"] {
            assert!(optional_scope_op(op));
            assert!(
                matches!(
                    validate_scope_declaration(op, &json!({ "entry_id": "e" })),
                    Ok(None)
                ),
                "{op}: an absent scope keeps 0.18.1 behaviour"
            );
            match validate_scope_declaration(op, &json!({ "scope": { "scope_id": "s" } })) {
                Ok(Some(scope)) => assert_eq!(scope.scope_id.as_str(), "s"),
                other => panic!("{op}: a well-formed scope must decode, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_present_null_scope_is_malformed_on_every_scope_bearing_op() {
        for op in REQUIRED_SCOPE_OPS
            .into_iter()
            .chain(["port.knowledge.get", "port.knowledge.put"])
        {
            assert!(
                validate_scope_declaration(op, &json!({ "scope": null })).is_err(),
                "{op}: a present null scope is never read as absent"
            );
        }
    }
}
