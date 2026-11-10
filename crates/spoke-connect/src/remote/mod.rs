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
//! optional [`RemoteExtractService`] `extract` face), the
//! [`Transport`] trait, the in-repo loopback transport pair for tests, and
//! the multi-peer capability router ([`MultiPeerRouter`] +
//! [`connect_multi_peer_router`] — capability-selected async `BaselinePorts`
//! over N registered per-peer adapters, frozen multi-peer routing contract).

use serde::Deserialize;
use serde_json::Value;
use spoke_operations::{SpokeReject, SpokeRejectCode};
use spoke_schemas::Scope;

use crate::core::{CAPABILITY_L2_COMPUTABLE, CAPABILITY_SPOKE_BASELINE};

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
    MemoryAdmission, MemoryPortService, RemoteExtractService, RemoteMemoryService,
    RemoteServePorts, RemoteServePortsComposite,
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
/// requires `spoke-baseline`; the optional families require their negotiated
/// family capability (`l2-computable` / `l5-fork`).
///
/// This is the requirement only — each surface keeps its own evaluation and
/// failure shape (serve-gate deny vs. selection reject), and neither surface
/// gains an op the other does not have.
pub(crate) fn port_op_requirement(op: &str) -> Option<&'static str> {
    match op {
        "port.knowledge.get" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.knowledge.put" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.relation.get" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.relation.put" => Some(CAPABILITY_SPOKE_BASELINE),
        "port.scope.list_knowledge_entries" => Some(CAPABILITY_SPOKE_BASELINE),
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

// ── `ke-ownership` conditional requirement (frozen F2) ─────────────────────
//
// One conclusion, three differently shaped integration sites: the responder
// evaluates it as a supplementary gate before probing or calling a provider,
// and the router applies it as a hard manifest filter. Core keeps its
// op-only table (see `core::dispatch`), so the payload-dependent rule lives
// here at the remote product boundary.

/// The three existing Scope-bearing remote ops. Only these declare the
/// `payload.scope` location the ownership predicate reads — `extract` and
/// the `tools.*` family carry no Scope and are never ownership-gated.
pub(crate) const SCOPE_BEARING_OPS: [&str; 3] = [
    "port.scope.list_knowledge_entries",
    "port.scope.list_timeline_events",
    "port.fork.list_timeline_events",
];

/// Whether a request requires the `ke-ownership` capability: it targets one
/// of [`SCOPE_BEARING_OPS`] **and** declares a non-empty string viewpoint at
/// `payload.scope.viewpoint`.
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
/// Runs at the responder before the provider probe and any host call, so a
/// malformed Scope is an input failure rather than a silent baseline
/// success. Non-Scope-bearing ops are untouched.
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
        return Err(invalid("missing scope".to_owned()));
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
