//! RemoteAdapter + message-oriented Transport (frozen contract:
//! `.mstar/specs/spoke-remote-adapter.md`).
//!
//! This module is gated behind the `remote-adapter` cargo feature so default
//! connect builds stay lean (no `spoke-operations` dependency); the TS
//! equivalent is the `./remote` subpath export.
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
pub use serve_ports::{RemoteExtractService, RemoteServePorts, RemoteServePortsComposite};
pub use transport::{
    loopback_transport_pair, LoopbackTransport, LoopbackTransportPair, Transport, TransportError,
};

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
pub(crate) fn validate_scope_declaration(op: &str, payload: &Value) -> Result<(), SpokeReject> {
    if !SCOPE_BEARING_OPS.contains(&op) {
        return Ok(());
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
        .map(|_| ())
        .map_err(|error| invalid(error.to_string()))
}
