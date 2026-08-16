//! `RemoteAdapter` — drop-in async `BaselinePorts` over a connect session
//! (frozen contract: `.mstar/specs/spoke-remote-adapter.md`).
//!
//! PUBLIC surface: the async `BaselinePorts` (six families) + the
//! [`connect_remote_adapter`] dial entrypoint + read-only session info
//! ([`RemoteAdapter::state`], `session_id`, `remote_peer_id`,
//! `remote_manifest`) + [`RemoteAdapter::close`].
//!
//! INTERNAL (encapsulated — consumers never touch these): hello sign/verify,
//! allowlist, nonce single-use, sequence allocate/advance, `request_id`
//! correlation, the dispatch gate, optional capability-token attach, the
//! receive-loop demux, and invoke timeout timers. All of it reuses the pure
//! session-core (`crate::core`) — nothing is reimplemented here.
//!
//! Each `BaselinePorts` method maps to a reserved `port.*` product op with an
//! opaque JSON payload (frozen contract §5.2), sent as a `ConnectInvokeRequest`
//! over the [`Transport`], awaited via the adapter-owned receive loop, and
//! deserialized back to `SpokeResult`. The WebSocket implementation is
//! consumer-side; only the loopback [`Transport`] ships in-repo.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::{BoxFuture, FutureExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use spoke_operations::{
    from_error_envelope, parse_tool_capability_id, spoke_ok, spoke_reject, to_error_envelope,
    FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort,
    ScopeQueryPort, SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::{
    ConnectInvokeResponse, ErrorEnvelope as WireErrorEnvelope,
};
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::connect::ConnectSession;
use spoke_schemas::{
    Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Rule, Scope, TimelineEvent,
};

use crate::core::{
    check_response_correlation, derive_peer_id_from_ed25519_pubkey, dispatch_allowed, is_allowlisted,
    sign_hello_ed25519, verify_hello_ed25519, CapabilityTokenProof, CoreError, Correlation,
    EnvelopeAuthError, EnvelopeAuthErrorKind, InboundSequence, NonceStore, OutboundSequence,
};
use crate::hello::generate_nonce;
use crate::remote::transport::{Transport, TransportError};
use crate::runtime::generate_request_id;

/// Default bounded-wait deadline for the handshake and each invoke, ms
/// (parity with TS `DEFAULT_INVOKE_TIMEOUT_MS`).
const DEFAULT_INVOKE_TIMEOUT_MS: u64 = 5000;

/// Process-wide single-use store of **accepted** server-hello
/// `(peer_id, nonce)` pairs (spec §Nonce / replay protection: "Receiver MUST
/// reject a hello whose `(peer_id, nonce)` pair was already accepted"; "an
/// in-memory set for the life of the process is sufficient for the reference
/// stack").
///
/// The host-side gate (`gate.rs`) records the client's hello on accept; the
/// RemoteAdapter is the receiver of the **server's** hello, so it enforces
/// the same receiver rule here. A replayed signed server hello — captured on
/// an earlier dial through this process — is rejected before any
/// `ConnectSession` snapshot is accepted, so an active transport attacker
/// cannot re-enter `Established` with a stale signature. The store is
/// process-scoped (shared across adapter instances) for exactly this reason:
/// an adapter dials once, and the replay arrives on a later dial. Parity
/// with the TS adapter's module-level `acceptedServerHellos`.
static ACCEPTED_SERVER_HELLOS: Mutex<Option<NonceStore>> = Mutex::new(None);

/// Record `(peer_id, nonce)` unless already accepted; `false` on replay.
/// Call only after the hello passed every earlier gate (allowlist,
/// signature) so a rejected hello is not burned (retry-safe, spec §Nonce).
fn record_server_hello(peer_id: &str, nonce: &str) -> bool {
    ACCEPTED_SERVER_HELLOS
        .lock()
        .expect("accepted-server-hellos lock")
        .get_or_insert_with(NonceStore::new)
        .check_and_record(peer_id, nonce)
}

/// Test-only: reset the process-wide accepted-server-hello store (simulates
/// a process restart, so a replay test can target the dial binding — the
/// responder-hello `peer_nonce` assert — rather than the in-memory nonce
/// single-use gate). Mirrors the TS adapter's
/// `__resetAcceptedServerHellosForTest`.
pub fn reset_accepted_server_hellos_for_test() {
    *ACCEPTED_SERVER_HELLOS
        .lock()
        .expect("accepted-server-hellos lock") = None;
}

/// Session-lifecycle labels (frozen contract §4) — parity with TS
/// `RemoteAdapterState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAdapterState {
    Disconnected,
    Handshaking,
    Established,
    Closed,
}

impl RemoteAdapterState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Handshaking => "Handshaking",
            Self::Established => "Established",
            Self::Closed => "Closed",
        }
    }
}

impl std::fmt::Display for RemoteAdapterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Raw Ed25519 keypair material for the local connect peer (mirror of TS
/// `RemoteIdentity`). The 32-byte seed is type-enforced in Rust (TS checks
/// the length at runtime).
#[derive(Debug, Clone)]
pub struct RemoteIdentity {
    pub seed: [u8; 32],
}

/// Dial options for [`connect_remote_adapter`] (mirror of TS
/// `RemoteAdapterOptions`; frozen contract §3.3).
#[derive(Clone)]
pub struct RemoteAdapterOptions {
    /// Message-oriented transport (consumer-provided; loopback ships in-repo).
    pub transport: Arc<dyn Transport>,
    /// This adapter's raw Ed25519 seed.
    pub local_identity: RemoteIdentity,
    /// Host manifest advertised in this adapter's signed hello.
    pub local_manifest: HostCapabilityManifest,
    /// Preconfigured remote Ed25519 public key. How the key is obtained is
    /// transport-adapter-owned; the remote's peer_id is derived from it and
    /// must be on `allowlist` (fail-closed).
    pub remote_pubkey: [u8; 32],
    /// Trusted remote peer ids — must contain the remote's derived peer_id
    /// (fail-closed).
    pub allowlist: Vec<String>,
    /// Bounded-wait deadline for the handshake and each invoke, ms
    /// (default [`DEFAULT_INVOKE_TIMEOUT_MS`]).
    pub invoke_timeout_ms: Option<u64>,
    /// Optional capability-token proof attached as `auth` on outbound invokes
    /// (§3.2/§3.3).
    pub capability_token: Option<CapabilityTokenProof>,
}

/// Dial / handshake failures — thrown before an adapter is returned (frozen
/// contract §8.2 last row; no half-open `BaselinePorts` instance).
#[derive(Debug, thiserror::Error)]
pub enum RemoteAdapterError {
    #[error("{0}")]
    Config(String),
    #[error("handshake failed: {0}")]
    Handshake(String),
    /// A mixed-version responder hello — the peer's `protocol_version` does
    /// not match the core protocol version. Distinct from other handshake
    /// faults: the caller can tell version negotiation failure apart from
    /// signature / identity / nonce failures (spec §Error mapping
    /// `details.kind = protocol_version_mismatch`; FFI dial kind
    /// `"protocol_version_mismatch"`).
    #[error("protocol version mismatch: {0}")]
    ProtocolVersionMismatch(String),
    #[error("{0}")]
    Timeout(String),
}

/// Internal invoke-failure classes (frozen contract §8.2). Consumers only
/// ever observe these mapped to `SpokeResult` `INTERNAL_ERROR` rejects with
/// `details.kind` — except [`connect_remote_adapter`], which errors for
/// dial/hello failures (§8.2 last row). Parity with TS `RemoteErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteErrorKind {
    Transport,
    SessionClosed,
    Timeout,
    CorrelationMismatch,
    SequenceExhausted,
    // Envelope-auth rejections (envelope-auth contract §8): the locked
    // `details.kind` vocabulary, surfaced via `INTERNAL_ERROR` SpokeRejects.
    EnvelopeAuthMissing,
    EnvelopeAuthInvalid,
    EnvelopeAuthSessionUnbound,
}

impl RemoteErrorKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::SessionClosed => "session_closed",
            Self::Timeout => "timeout",
            Self::CorrelationMismatch => "correlation_mismatch",
            Self::SequenceExhausted => "sequence_exhausted",
            Self::EnvelopeAuthMissing => "envelope_auth_missing",
            Self::EnvelopeAuthInvalid => "envelope_auth_invalid",
            Self::EnvelopeAuthSessionUnbound => "envelope_auth_session_unbound",
        }
    }
}

/// Map an envelope-auth wire-rejection kind (contract §8) to the internal
/// `RemoteError` kind so `invoke_mapped` surfaces it as an `INTERNAL_ERROR`
/// `SpokeResult` reject with `details.kind` verbatim (parity with the TS
/// `RemoteError(error.kind, ...)` mapping).
fn envelope_auth_kind_to_remote_error(kind: EnvelopeAuthErrorKind) -> RemoteErrorKind {
    match kind {
        EnvelopeAuthErrorKind::Missing => RemoteErrorKind::EnvelopeAuthMissing,
        EnvelopeAuthErrorKind::Invalid => RemoteErrorKind::EnvelopeAuthInvalid,
        EnvelopeAuthErrorKind::SessionUnbound => RemoteErrorKind::EnvelopeAuthSessionUnbound,
    }
}

/// Internal invoke failure (mirror of TS `RemoteError`).
#[derive(Debug, Clone)]
pub(crate) struct RemoteError {
    pub(crate) kind: RemoteErrorKind,
    pub(crate) message: String,
}

impl RemoteError {
    pub(crate) fn new(kind: RemoteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `SpokeResult` reject with `INTERNAL_ERROR` + `details.kind` (contract
/// §8.2 transport/session/timeout/correlation rows).
pub(crate) fn internal_error<T>(kind: &str, message: impl Into<String>) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert("kind".into(), Value::String(kind.into()));
    spoke_reject(SpokeRejectCode::InternalError, message, Some(details))
}

/// Extract a human-readable message from a `catch_unwind` payload (mirror
/// of the `ffi.rs` helper of the same name — a panicking tool handler is
/// answered with the error branch, never allowed to crash the loop).
pub(crate) fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "tool handler panicked".to_owned()
}

/// Dispatch-deny wire codes (contract §8.2): the host answered that the op or
/// its required capability is not available → `CAPABILITY_PORT_MISSING`.
fn is_dispatch_deny(code: &str) -> bool {
    matches!(code, "op_unsupported" | "capability_missing")
}

/// Request-shape discriminator (mirror of TS `isConnectInvokeRequest`): an
/// inbound envelope carrying `op` plus the correlation echo fields and a
/// payload is a `ConnectInvokeRequest` — a reverse invoke.
///
/// Classification rule (normative `spoke-connect.md` §Request / response
/// classification): an inbound envelope carrying `op` is a request — NEVER
/// a response, even though a reverse request carries the same correlation
/// echo fields (`session_id` / `sequence` / `request_id`) and a `payload`
/// as the response success branch. Without the request-first order the
/// reverse request would satisfy the response discriminator and be silently
/// swallowed by the `request_id` demux.
pub(crate) fn is_connect_invoke_request(doc: &Value) -> bool {
    doc.get("session_id").is_some()
        && doc.get("sequence").is_some()
        && doc.get("request_id").is_some()
        && doc.get("op").is_some()
        && doc.get("payload").is_some()
}

/// Extract the correlation echo fields from a response-shaped wire document
/// (both `payload` and `error` branches carry the three fields at the top
/// level). `None` for any other envelope shape — the receive-loop
/// discriminator (mirror of TS `isConnectInvokeResponse`).
pub(crate) fn wire_response_correlation(doc: &Value) -> Option<Correlation> {
    // Classification rule (mirror of TS `isConnectInvokeResponse`): an
    // envelope carrying `op` is a `ConnectInvokeRequest` — NEVER a
    // response. A reverse request carries the same correlation echo fields
    // + a `payload` as the success branch, so without this exclusion it
    // would satisfy the response discriminator and be silently swallowed by
    // a `request_id` demux (architect HIGH finding). No response ever
    // carried `op` per the wire field tables, so rejecting `op`-bearing
    // docs is strictly hardening.
    if doc.get("op").is_some() {
        return None;
    }
    // Parity with TS `isConnectInvokeResponse`: the doc must carry at
    // least one response branch — the wire type is a `payload` XOR `error`
    // sum branch, so a branch-less fragment is not a response and is not
    // demuxed. (The exactly-one rule is enforced later by
    // `verify_invoke_response_auth`, which rejects a merged branch.)
    if doc.get("payload").is_none() && doc.get("error").is_none() {
        return None;
    }
    Some(Correlation {
        session_id: doc.get("session_id")?.as_str()?.to_string(),
        sequence: doc.get("sequence")?.as_i64()?,
        request_id: doc.get("request_id")?.as_str()?.to_string(),
    })
}

/// Map an error-branch envelope to a `SpokeResult` reject (contract §8.2).
///
/// The error branch carries the codegen-inline wire `ErrorEnvelope`, which is
/// field-identical to `spoke_schemas::ErrorEnvelope`; the shared
/// `from_error_envelope` mapping runs after a lossless value conversion.
pub(crate) fn map_error_envelope(error: &WireErrorEnvelope) -> SpokeReject {
    let mut details = error.details.clone();
    if is_dispatch_deny(&error.code) {
        details.insert("wire_code".into(), Value::String(error.code.clone()));
        return SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: error.message.clone(),
            details: Some(details),
        };
    }
    // Envelope-auth rejection (contract §8): the host answered `auth_failed`
    // for a request that failed envelope verification. `auth_failed` is not
    // a SpokeRejectCode, so the generic mapping would produce
    // INVALID_INPUT; the locked mapping is INTERNAL_ERROR with the
    // envelope-auth `details.kind` surfaced verbatim.
    if error.code == EnvelopeAuthError::CODE {
        return SpokeReject {
            code: SpokeRejectCode::InternalError,
            message: error.message.clone(),
            details: Some(details),
        };
    }
    let Ok(data_error) = serde_json::from_value::<spoke_schemas::ErrorEnvelope>(
        serde_json::to_value(error).expect("wire error envelope serializes"),
    ) else {
        return SpokeReject {
            code: SpokeRejectCode::InvalidInput,
            message: "unmappable error envelope".into(),
            details: Some(details),
        };
    };
    from_error_envelope(&data_error)
}

/// Convert the ops/data `HostCapabilityManifest` to the field-identical
/// hello wire type (codegen-inline shapes; lossless).
pub(crate) fn connect_manifest(manifest: &HostCapabilityManifest) -> ConnectHostCapabilityManifest {
    serde_json::from_value(serde_json::to_value(manifest).expect("manifest serializes"))
        .expect("field-identical manifest converts")
}

/// Established-session state, per peer (frozen contract §4: `peer_id`,
/// `session_id`, outbound sequence — adapter-local, never a process-global
/// singleton).
struct EstablishedSession {
    session_id: String,
    /// The verified remote peer id (the peer the session is bound to).
    responder_peer_id: String,
    /// This adapter's raw Ed25519 hello-identity seed — every outbound
    /// `ConnectInvokeRequest` is signed with it
    /// (`spoke-connect-invoke-request-jcs-v1`; v2 requires the `signature`
    /// on every post-hello envelope — envelope-auth contract §5). Same
    /// role as `SessionHandle.local_secret` in the node/session path.
    local_secret: [u8; 32],
    /// The session's negotiated capabilities (intersection of the local and
    /// remote hello manifests) — the dispatch gate for inbound invokes
    /// (`tools.*` ops require the op string itself, evaluated against this
    /// set; frozen §3).
    negotiated_capabilities: Vec<String>,
    sequence: Mutex<OutboundSequence>,
    /// Inbound invoke sequence expectation (receiver side) — the reverse
    /// serving gate peeks (non-mutating) before envelope-auth verify and
    /// advances only after verify passes (auth-before-advance, frozen §4).
    inbound: Mutex<InboundSequence>,
}

/// A parked invoke: correlation material + the waiter channel + the timeout
/// task (mirror of TS `PendingInvoke`; the timeout task plays `setTimeout`).
struct PendingInvoke {
    correlation: Correlation,
    tx: tokio::sync::mpsc::Sender<Result<ConnectInvokeResponse, RemoteError>>,
    timeout_task: tokio::task::JoinHandle<()>,
}

/// Registered tool handler (frozen contract §1/§6): receives the tool's
/// `arguments` JSON value from the request payload and resolves with the
/// tool result as a `SpokeResult`. A panicking handler answers the error
/// branch (mapped via `toErrorEnvelope` semantics) and never crashes the
/// receive loop (`catch_unwind` containment, mirroring the existing invoke
/// path's panic containment).
pub type ToolHandler =
    Arc<dyn Fn(Value) -> BoxFuture<'static, SpokeResult<Value>> + Send + Sync>;

/// Gate-phase outcome for a reverse invoke (see [`RemoteAdapter::run_reverse_gate`]).
enum ReverseGateResult {
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

/// Single-peer async `BaselinePorts` proxy over an established connect
/// session. Construct via [`connect_remote_adapter`] — the adapter is only
/// reachable in `Established` (or `Closed` after `close`); port calls fail
/// closed on any other state.
pub struct RemoteAdapter {
    transport: Arc<dyn Transport>,
    invoke_timeout: Duration,
    capability_token: Option<CapabilityTokenProof>,
    /// The remote peer's 32-byte hello Ed25519 public key — every inbound
    /// post-hello envelope (`ConnectSession` snapshot, `ConnectInvokeResponse`)
    /// is verified against it (`spoke-connect-session-jcs-v1` /
    /// `spoke-connect-invoke-response-jcs-v1`; envelope-auth contract §7).
    remote_pubkey: [u8; 32],

    /// Outbound send serialization lock (contract §5.3 / §10 — allocation
    /// order MUST equal wire order; the peer's inbound gate rejects an
    /// out-of-order request with `inbound_sequence_mismatch`). Acquired in
    /// `invoke_op` before the outbound sequence allocation and held through
    /// `transport.send().await`, so concurrent invokes reach the wire in
    /// exactly the order they allocated. Mirror of the TS adapter's
    /// `#sendTail` promise chain (whose tail entry is created synchronously
    /// before the async Ed25519 sign).
    send_lock: tokio::sync::Mutex<()>,

    state: Mutex<RemoteAdapterState>,
    session: Mutex<Option<EstablishedSession>>,
    remote_manifest: Mutex<Option<HostCapabilityManifest>>,
    pending: Arc<Mutex<HashMap<String, PendingInvoke>>>,
    receive_loop_running: AtomicBool,
    /// Tool-handler registry for reverse invokes (frozen contract §6):
    /// `register_tool_handler` fills it; the receive loop's serving path
    /// looks it up by exact capability id. The local manifest's `tools[]`
    /// (carried through hello) is the discovery source — this registry MUST
    /// NOT mutate the manifest; a registry/manifest mismatch is surfaced at
    /// invoke time — a manifest-declared tool with no registered handler is
    /// denied fail-closed (`op_unsupported` → `CAPABILITY_PORT_MISSING`);
    /// `validate_manifest_tools` checks manifest-internal consistency only.
    tool_handlers: Mutex<HashMap<String, ToolHandler>>,
}

impl RemoteAdapter {
    fn new(
        transport: Arc<dyn Transport>,
        remote_pubkey: [u8; 32],
        invoke_timeout: Duration,
        capability_token: Option<CapabilityTokenProof>,
    ) -> Self {
        Self {
            transport,
            invoke_timeout,
            capability_token,
            remote_pubkey,
            send_lock: tokio::sync::Mutex::new(()),
            state: Mutex::new(RemoteAdapterState::Disconnected),
            session: Mutex::new(None),
            remote_manifest: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            receive_loop_running: AtomicBool::new(false),
            tool_handlers: Mutex::new(HashMap::new()),
        }
    }

    /// Read-only session state (contract §4 labels).
    #[must_use]
    pub fn state(&self) -> RemoteAdapterState {
        *self.state.lock().expect("state lock")
    }

    /// The remote-assigned session id (`None` before establishment). TS
    /// returns `""`; Rust uses `Option` as the idiomatic equivalent.
    #[must_use]
    pub fn session_id(&self) -> Option<String> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    /// The verified remote peer id (`None` before establishment).
    #[must_use]
    pub fn remote_peer_id(&self) -> Option<String> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.responder_peer_id.clone())
    }

    /// The remote peer's `HostCapabilityManifest`, from the authenticated
    /// hello `host` (contract §7). `None` before establishment.
    #[must_use]
    pub fn remote_manifest(&self) -> Option<HostCapabilityManifest> {
        self.remote_manifest.lock().expect("manifest lock").clone()
    }

    /// Release the session and transport. Idempotent; pending invokes fail
    /// with `INTERNAL_ERROR` `details.kind = "session_closed"` (contract §6
    /// "Transport close mid-flight" / §8.2).
    pub fn close(&self) {
        self.close_session("local shutdown");
    }

    // ── Tool serving (reverse invokes) ─────────────────────────────────────

    /// Register a handler for a `tools.<ns>.<tool_id>` capability served on
    /// this adapter (frozen contract §6). A reverse invoke whose op passes
    /// the dispatch gate dispatches to the registered handler; the handler
    /// runs off the receive loop and its `SpokeResult` maps to a signed
    /// `ConnectInvokeResponse` (success `{ result }` branch, or the error
    /// branch via `to_error_envelope`).
    ///
    /// Grammar-asserted: a non-`tools.` id panics (programmer misuse, like
    /// the generated-type ergonomics of `tool_capability_id`). Duplicate
    /// registration for the same id OVERWRITES the previous handler
    /// (last-wins, documented).
    ///
    /// The registry does NOT mutate the local manifest — descriptor truth
    /// for discovery stays in the manifest's `tools[]` (sent through
    /// hello); registering a handler for a tool the manifest does not
    /// declare leaves that id unserved — invoking it answers the
    /// dispatch-deny branch; `validate_manifest_tools` checks
    /// manifest-internal consistency only.
    pub fn register_tool_handler(&self, capability_id: &str, handler: ToolHandler) {
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => {
                panic!("{}", reject.message);
            }
        }
        self.tool_handlers
            .lock()
            .expect("tool handlers lock")
            .insert(capability_id.to_owned(), handler);
    }

    /// Forward tool-invoke face (frozen contract §6): issue a
    /// `ConnectInvokeRequest` with `op = capability_id` toward the remote
    /// peer and resolve with the tool's `result` (extracted from the success
    /// `payload = { result: <opaque JSON> }`). Deny answers map via the
    /// existing D7 row (`op_unsupported` / `capability_missing` →
    /// `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved). Reuses
    /// the `invoke_op` wire-order serialization and its deferred-send
    /// poison-close.
    pub async fn invoke_tool(
        &self,
        capability_id: &str,
        arguments: Value,
    ) -> SpokeResult<Value> {
        // Fail fast on a non-tool capability id (the op string IS the
        // capability string; a non-`tools.` id is a programming error).
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
        // Tool invoke payload shape: `{ "arguments": <opaque JSON> }` (§4).
        let response = match self.invoke_op(capability_id, json!({ "arguments": arguments })).await {
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
                // `details.kind = "transport"` (mirrors the `invoke_mapped`
                // shape gate).
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

    // ── Session lifecycle (internal; called by connect_remote_adapter) ────

    /// Dial-only: the adapter is in `Handshaking` while dialing.
    fn begin_handshake(&self) {
        *self.state.lock().expect("state lock") = RemoteAdapterState::Handshaking;
    }

    /// Dial-only: bind the authenticated session and start the receive loop.
    fn establish(
        self: &Arc<Self>,
        session: EstablishedSession,
        remote_manifest: HostCapabilityManifest,
    ) {
        *self.session.lock().expect("session lock") = Some(session);
        *self.remote_manifest.lock().expect("manifest lock") = Some(remote_manifest);
        *self.state.lock().expect("state lock") = RemoteAdapterState::Established;
        self.spawn_receive_loop();
    }

    /// All failure paths that make the session unusable: mark `Closed`, fail
    /// every pending waiter with `session_closed`, and release the transport
    /// (fire-and-forget, mirroring TS `closeSession`).
    fn close_session(&self, reason: &str) {
        {
            let mut state = self.state.lock().expect("state lock");
            if *state == RemoteAdapterState::Closed {
                return;
            }
            *state = RemoteAdapterState::Closed;
        }

        let entries: Vec<PendingInvoke> = self
            .pending
            .lock()
            .expect("pending lock")
            .drain()
            .map(|(_request_id, entry)| entry)
            .collect();
        for entry in entries {
            entry.timeout_task.abort();
            let _ = entry.tx.try_send(Err(RemoteError::new(
                RemoteErrorKind::SessionClosed,
                format!("connect session closed: {reason}"),
            )));
        }

        let transport = Arc::clone(&self.transport);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = transport.close().await;
            });
        }
    }

    // ── Receive loop (adapter-owned; port methods never call recv) ────────

    fn spawn_receive_loop(self: &Arc<Self>) {
        if self.receive_loop_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let adapter = Arc::clone(self);
        tokio::spawn(async move {
            adapter.receive_loop().await;
            adapter.receive_loop_running.store(false, Ordering::SeqCst);
        });
    }

    async fn receive_loop(self: &Arc<Self>) {
        while *self.state.lock().expect("state lock") == RemoteAdapterState::Established {
            let bytes = match self.transport.recv().await {
                Ok(bytes) => bytes,
                Err(error) => {
                    // Transport loss: fail every pending waiter and transition
                    // to `Closed` (contract §6 / §8.2). Decode failure counts
                    // as transport loss, same as TS `decodeJsonMessage` in the
                    // recv try-block.
                    self.close_session(&format!("transport loss: {error}"));
                    return;
                }
            };
            let doc: Value = match serde_json::from_slice(&bytes) {
                Ok(doc) => doc,
                Err(error) => {
                    self.close_session(&format!("transport loss: {error}"));
                    return;
                }
            };
            // Classify request shape FIRST (normative `spoke-connect.md`
            // §Request / response classification): an inbound envelope
            // carrying `op` is a `ConnectInvokeRequest` — never a response,
            // even though a reverse request carries the same correlation
            // echo fields + payload as the response success branch. Without
            // this order the reverse request would satisfy the response
            // discriminator and be silently swallowed by the request_id
            // demux below.
            if is_connect_invoke_request(&doc) {
                // Reverse invoke serving. The gate (peek → verify →
                // advance) is awaited INLINE: the loop reads the next
                // envelope only after this request's gate completes, so
                // steps 3–5 are serialized per session (frozen §4). Dispatch
                // (gate → handler) fires without blocking the loop.
                match self.serve_reverse_invoke(&doc).await {
                    Ok(()) => continue,
                    Err(error) => {
                        // Unexpected serving failure (local key
                        // misconfiguration / internal bug): fail closed like
                        // transport loss — a live session must not silently
                        // drop reverse invokes.
                        self.close_session(&format!(
                            "reverse invoke serving failed: {error}"
                        ));
                        return;
                    }
                }
            }
            // Response-shape discriminator on the wire form (mirror of TS
            // `isConnectInvokeResponse`): both branches carry the three echo
            // fields at the top level. Everything else (hello / session /
            // unknown shape) is a post-handshake stray envelope — ignored.
            let Some(correlation) = wire_response_correlation(&doc) else {
                continue;
            };

            // Demux by request_id; unknown/duplicate responses are dropped
            // (protocol v1 defines no retry).
            let request_id = correlation.request_id.clone();
            let entry = self
                .pending
                .lock()
                .expect("pending lock")
                .remove(&request_id);
            let Some(entry) = entry else {
                continue;
            };
            entry.timeout_task.abort();
            let session_id = self
                .session
                .lock()
                .expect("session lock")
                .as_ref()
                .map(|session| session.session_id.clone());
            let Some(session_id) = session_id else {
                // The session vanished mid-loop (close raced this response):
                // fail every pending waiter and transition to `Closed`.
                self.close_session("session state missing in receive loop");
                return;
            };
            // Correlation echo check first (non-mutating wire-position
            // validation), then envelope-auth verify (contract §7: the
            // response must carry the peer's authentic signature over the
            // exact wire branch, against the peer's hello Ed25519 public
            // key). A forged/tampered response fails closed — only this
            // waiter, never a session-state mutation.
            let outcome = check_response_correlation(&entry.correlation, &correlation)
                .map_err(|_| {
                    RemoteError::new(
                        RemoteErrorKind::CorrelationMismatch,
                        "response echo fields do not match the request",
                    )
                })
                .and_then(|()| {
                    // Envelope-auth verify (contract §7). The three wire
                    // rejections map to their locked `details.kind`; the
                    // local `Crypto` case (`kind()` is `None` — wrong-length
                    // / invalid key bytes) is still a verification failure
                    // of the response's authenticator and surfaces as
                    // `envelope_auth_invalid` — never mislabeled as a
                    // correlation mismatch.
                    crate::core::verify_invoke_response_auth(&self.remote_pubkey, &doc, &session_id)
                        .map_err(|error| match error.kind() {
                            Some(kind) => RemoteError::new(
                                envelope_auth_kind_to_remote_error(kind),
                                error.to_string(),
                            ),
                            None => RemoteError::new(
                                RemoteErrorKind::EnvelopeAuthInvalid,
                                error.to_string(),
                            ),
                        })
                })
                .and_then(|()| {
                    // Verify passed ⇒ the wire doc is a well-formed response
                    // branch (the signer signed exactly the wire branch), so
                    // typed deserialization cannot fail.
                    serde_json::from_value::<ConnectInvokeResponse>(doc).map_err(|error| {
                        RemoteError::new(
                            RemoteErrorKind::Transport,
                            format!("response decode failed: {error}"),
                        )
                    })
                });
            let _ = entry.tx.try_send(outcome);
        }
    }

    // ── Reverse invoke serving (frozen §4 pipeline) ───────────────────────

    /// Send one signed response envelope, fire-and-forget (responses are
    /// demuxed by `request_id` on the peer; a send failure is not observable
    /// to this session).
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

    /// Sign + send an error-branch response to a reverse invoke. The echo
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
    ) -> Result<(), RemoteError> {
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
        let session_guard = self.session.lock().expect("session lock");
        let Some(session) = session_guard.as_ref() else {
            return Ok(()); // session gone — fire-and-forget boundary
        };
        let mut error = Map::new();
        error.insert("code".into(), Value::String(code.to_owned()));
        error.insert("message".into(), Value::String(message.to_owned()));
        if let Some(details) = details {
            error.insert("details".into(), Value::Object(details.clone()));
        }
        error.insert("extensions".into(), json!({}));
        let signed = crate::core::authenticate_invoke_response(
            &session.local_secret,
            &crate::core::InvokeResponseSignInput::Error {
                session_id: session_id.to_owned(),
                sequence,
                request_id: request_id.to_owned(),
                error: serde_json::from_value(Value::Object(error))
                    .map_err(|error| {
                        RemoteError::new(
                            RemoteErrorKind::Transport,
                            format!("error envelope build failed: {error}"),
                        )
                    })?,
            },
            HashMap::new(),
        )
        .map_err(|error| {
            RemoteError::new(
                RemoteErrorKind::Transport,
                format!("response sign failed: {error}"),
            )
        })?;
        self.send_reverse_response(
            &serde_json::to_value(signed).expect("signed response serializes"),
        );
        Ok(())
    }

    /// Serve one reverse invoke per the canonical order (frozen §4):
    /// classify (caller) → stray → sequence peek → envelope-auth verify →
    /// advance → gate → handler → signed response. The caller (receive
    /// loop) awaits this method inline so peek → verify → advance are
    /// serialized; dispatch fires without blocking the loop.
    async fn serve_reverse_invoke(self: &Arc<Self>, doc: &Value) -> Result<(), RemoteError> {
        match self.run_reverse_gate(doc).await? {
            ReverseGateResult::Stray => Ok(()),
            ReverseGateResult::Denied {
                code,
                message,
                details,
            } => {
                self.send_reverse_error_envelope(doc, &code, &message, details.as_ref())
                    .await?;
                Ok(())
            }
            ReverseGateResult::Ok => {
                let doc = doc.clone();
                let adapter = Arc::clone(self);
                tokio::spawn(async move {
                    adapter.dispatch_reverse_invoke(doc).await;
                });
                Ok(())
            }
        }
    }

    /// Gate phase — sequence peek (non-mutating) → envelope-auth verify →
    /// advance. Fail-closed (auth-before-advance, spec §Verify rules): a
    /// forged/tampered/stripped signature answers `auth_failed` and leaves
    /// the inbound counter unchanged. Returns [`ReverseGateResult::Stray`]
    /// for stray requests (ignored), a rejection spec for gate failures, or
    /// [`ReverseGateResult::Ok`] when dispatch may run.
    async fn run_reverse_gate(&self, doc: &Value) -> Result<ReverseGateResult, RemoteError> {
        let session_guard = self.session.lock().expect("session lock");
        let Some(session) = session_guard.as_ref() else {
            return Ok(ReverseGateResult::Stray); // stray — no established session
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
                    return Ok(ReverseGateResult::Denied {
                        code: "invalid_sequence".to_owned(),
                        message: format!("inbound sequence {value} is not the next expected"),
                        details: None,
                    });
                }
            },
            None => return Ok(ReverseGateResult::Stray),
        };
        // Stray check (single-peer adapter): a `session_id` bound to a
        // DIFFERENT live session would be ignored — this adapter owns one
        // session, so verify owns the session-binding assert
        // (`envelope_auth_session_unbound` on mismatch), mirroring TS.
        // 1. Sequence peek — non-mutating (auth-before-advance, frozen §4):
        //    the wire position is validated WITHOUT consuming it, so a
        //    bogus-signature envelope cannot desync the session.
        let peek_ok = session
            .inbound
            .lock()
            .expect("inbound lock")
            .peek(sequence)
            .is_ok();
        if !peek_ok {
            return Ok(ReverseGateResult::Denied {
                code: "invalid_sequence".to_owned(),
                message: format!("inbound sequence {sequence} is not the next expected"),
                details: None,
            });
        }
        // 2. Envelope-auth verify (contract §7 — auth-before-advance): the
        //    request signature is verified over the wire form against the
        //    remote's hello Ed25519 public key BEFORE the inbound counter
        //    advances. A forged / tampered / missing signature is answered
        //    `auth_failed` carrying the locked `details.kind`, and the
        //    session state is left untouched.
        if let Err(error) =
            crate::core::verify_invoke_request_auth(&self.remote_pubkey, doc, &session.session_id)
        {
            return match error.kind() {
                Some(kind) => Ok(ReverseGateResult::Denied {
                    code: EnvelopeAuthError::CODE.to_owned(),
                    message: error.to_string(),
                    details: Some(Map::from_iter([(
                        "kind".into(),
                        Value::String(kind.as_str().to_owned()),
                    )])),
                }),
                // Wrong-length key is adapter misconfiguration — fail loudly.
                None => Err(RemoteError::new(
                    RemoteErrorKind::EnvelopeAuthInvalid,
                    error.to_string(),
                )),
            };
        }
        // 3. Advance the inbound counter only after envelope-auth verify
        //    passed. The serialized gate makes the advance race-free (the
        //    loop awaits the gate inline before reading the next envelope).
        session
            .inbound
            .lock()
            .expect("inbound lock")
            .advance(sequence)
            .map_err(|error| {
                RemoteError::new(
                    RemoteErrorKind::Transport,
                    format!("inbound sequence advance failed: {error}"),
                )
            })?;
        Ok(ReverseGateResult::Ok)
    }

    /// Dispatch phase — runs after the serialized gate; may interleave with
    /// other invokes. Fully contained: a throwing/panicking handler answers
    /// the error branch and never crashes the loop.
    async fn dispatch_reverse_invoke(self: &Arc<Self>, doc: Value) {
        if let Err(error) = self.try_dispatch_reverse_invoke(&doc).await {
            // Unexpected serving failure (e.g. non-JSON-serializable handler
            // result): answer the error branch so the invoker never hangs,
            // and never crash the loop.
            let _ = self
                .send_reverse_error_envelope(
                    &doc,
                    SpokeRejectCode::InternalError.as_str(),
                    &format!("tool invoke failed: {error}"),
                    None,
                )
                .await;
        }
    }

    /// Contained dispatch body (see [`RemoteAdapter::dispatch_reverse_invoke`]).
    async fn try_dispatch_reverse_invoke(&self, doc: &Value) -> Result<(), RemoteError> {
        // Extract session material + wire echo fields under the guard; never
        // hold a MutexGuard across an await.
        let (local_secret, negotiated) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(()); // stray — belt-and-braces; the gate checked
            };
            (session.local_secret, session.negotiated_capabilities.clone())
        };
        let (Some(op), Some(session_id), Some(sequence), Some(request_id)) = (
            doc.get("op").and_then(Value::as_str),
            doc.get("session_id").and_then(Value::as_str),
            doc.get("sequence").and_then(Value::as_i64),
            doc.get("request_id").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        // Dispatch gate — `dispatch_allowed`-level logic (frozen §3):
        // `tools.*` ops require the op string itself, evaluated against
        // `negotiated_capabilities` (never a raw requirements-map
        // composition, which would deny the self-describing tools family).
        if !dispatch_allowed(op, &negotiated) {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("op {op} is not authorized by this session"),
                None,
            )
            .await?;
            return Ok(());
        }
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
        // the receive loop / serve loop never crashes), mirroring the
        // existing invoke path's panic containment.
        let result = match std::panic::AssertUnwindSafe(handler(arguments))
            .catch_unwind()
            .await
        {
            Ok(result) => result,
            Err(payload) => {
                let message = panic_payload_message(payload);
                SpokeResult::<Value>::Reject(SpokeReject {
                    code: SpokeRejectCode::InternalError,
                    message,
                    details: None,
                })
            }
        };
        match result {
            SpokeResult::Ok(value) => {
                // Success branch: `payload = { "result": <opaque JSON> }`
                // (frozen §4).
                let signed = crate::core::authenticate_invoke_response(
                    &local_secret,
                    &crate::core::InvokeResponseSignInput::Success {
                        session_id: session_id.to_owned(),
                        sequence,
                        request_id: request_id.to_owned(),
                        payload: json!({ "result": value }),
                    },
                    HashMap::new(),
                )
                .map_err(|error| {
                    RemoteError::new(
                        RemoteErrorKind::Transport,
                        format!("response sign failed: {error}"),
                    )
                })?;
                self.send_reverse_response(
                    &serde_json::to_value(signed).expect("signed response serializes"),
                );
                Ok(())
            }
            SpokeResult::Reject(reject) => {
                let error: WireErrorEnvelope =
                    serde_json::from_value(serde_json::to_value(to_error_envelope(&reject)).expect(
                        "ops error envelope serializes",
                    ))
                    .expect("field-identical error envelope converts");
                let signed = crate::core::authenticate_invoke_response(
                    &local_secret,
                    &crate::core::InvokeResponseSignInput::Error {
                        session_id: session_id.to_owned(),
                        sequence,
                        request_id: request_id.to_owned(),
                        error,
                    },
                    HashMap::new(),
                )
                .map_err(|error| {
                    RemoteError::new(
                        RemoteErrorKind::Transport,
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

    // ── Invoke path ───────────────────────────────────────────────────────

    /// Build the outbound `ConnectInvokeRequest`: atomic outbound sequence
    /// allocation, fresh `request_id`, optional capability-token `auth`,
    /// and the `spoke-connect-invoke-request-jcs-v1` signature over the
    /// request's signed fields (envelope-auth contract §5 — v2 requires
    /// the `signature` on every post-hello envelope).
    /// Session unusable → `Err(RemoteError)`; sequence exhaustion closes the
    /// session (contract §6 "Sequence overflow").
    fn build_request(&self, op: &str, payload: Value) -> Result<ConnectInvokeRequest, RemoteError> {
        let state = *self.state.lock().expect("state lock");
        if state != RemoteAdapterState::Established {
            return Err(RemoteError::new(
                RemoteErrorKind::SessionClosed,
                format!("connect session is not established (state {state})"),
            ));
        }
        let session_guard = self.session.lock().expect("session lock");
        let Some(session) = session_guard.as_ref() else {
            return Err(RemoteError::new(
                RemoteErrorKind::SessionClosed,
                format!("connect session is not established (state {state})"),
            ));
        };
        let allocate_result = {
            let mut sequence_guard = session.sequence.lock().expect("sequence lock");
            sequence_guard.allocate()
        };
        let sequence = match allocate_result {
            Ok(sequence) => sequence,
            Err(_) => {
                drop(session_guard);
                self.close_session("outbound sequence exhausted");
                return Err(RemoteError::new(
                    RemoteErrorKind::SequenceExhausted,
                    "outbound sequence space exhausted — reopen session",
                ));
            }
        };
        let request_id = generate_request_id()
            .map_err(|error| RemoteError::new(RemoteErrorKind::Transport, error.to_string()))?;
        let auth = self
            .capability_token
            .as_ref()
            .map(|token| serde_json::to_value(token).expect("capability token proof serializes"));
        // Validate the op and the derived wire strings up front (empty or
        // malformed values are caller errors — `authenticate_invoke_request`
        // re-parses them infallibly after this gate), mirroring
        // `session.rs::invoke_with_auth`.
        let _: spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequestOp =
            op.parse().map_err(
                |error: spoke_schemas::connect::connect_invoke_request::error::ConversionError| {
                    RemoteError::new(
                        RemoteErrorKind::Transport,
                        format!("invalid op {op:?}: {error}"),
                    )
                },
            )?;
        let _: spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequestRequestId =
            request_id
                .parse()
                .map_err(|error| {
                    RemoteError::new(
                        RemoteErrorKind::Transport,
                        format!("invalid request_id: {error}"),
                    )
                })?;
        let _: spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequestSessionId =
            session.session_id.parse().map_err(|error| {
                RemoteError::new(
                    RemoteErrorKind::Transport,
                    format!("invalid session_id: {error}"),
                )
            })?;
        // Authenticate the outbound request with this adapter's hello
        // identity seed (the seed the server verified at the hello
        // exchange; envelope-auth signs every post-hello envelope with the
        // same key). Signing is infallible for the validated 32-byte seed
        // stored on the established session.
        crate::core::authenticate_invoke_request(
            &session.local_secret,
            &crate::core::InvokeRequestSignInput {
                session_id: session.session_id.clone(),
                sequence: sequence as i64,
                request_id: request_id.clone(),
                op: op.to_owned(),
                payload: payload.clone(),
                auth: auth.clone(),
            },
            HashMap::new(),
        )
        .map_err(|error| {
            RemoteError::new(
                RemoteErrorKind::Transport,
                format!("envelope auth signing failed: {error}"),
            )
        })
    }

    /// Send one op invoke and resolve with its correlated response envelope.
    /// Errors with [`RemoteError`] on timeout / transport failure / session
    /// close / correlation mismatch / sequence exhaustion.
    ///
    /// Outbound wire-order serialization (contract §5.3 / §10 — allocation
    /// order MUST equal wire order; the peer's inbound gate rejects an
    /// out-of-order request with `inbound_sequence_mismatch`): the send
    /// lock is acquired BEFORE the outbound sequence allocation in
    /// `build_request` and held through the wire send, so concurrent
    /// invokes reach the transport in exactly the order they allocated —
    /// allocate + sign + send happen under a single lock. Mirror of the TS
    /// adapter's `#sendTail` promise chain (whose tail entry is created
    /// synchronously before the async Ed25519 sign; Rust signs
    /// synchronously). The lock is released before awaiting the response so
    /// concurrent invokes still demux in parallel.
    async fn invoke_op(
        &self,
        op: &str,
        payload: Value,
    ) -> Result<ConnectInvokeResponse, RemoteError> {
        let _send_guard = self.send_lock.lock().await;
        let request = self.build_request(op, payload)?;
        let request_id = request.request_id.to_string();

        // Register the pending waiter + its timeout task (TS `setTimeout`):
        // on elapse, remove the entry and fail only this waiter — the session
        // stays open (contract §6 "Invoke timeout").
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
                    let _ = tx_timeout.try_send(Err(RemoteError::new(
                        RemoteErrorKind::Timeout,
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
            PendingInvoke {
                correlation: Correlation::from(&request),
                tx: tx.clone(),
                timeout_task,
            },
        );

        // Encode + send. A synchronous encode failure or an async send
        // failure settles this invoke now — no dead entry waits out the
        // timeout (TS mirrors both paths).
        let envelope = match serde_json::to_vec(&request) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.fail_pending_send(&request_id, &format!("invoke encode failed: {error}"));
                return Err(RemoteError::new(
                    RemoteErrorKind::Transport,
                    format!("invoke encode failed: {error}"),
                ));
            }
        };
        if let Err(error) = self.transport.send(&envelope).await {
            self.fail_pending_send(&request_id, &format!("invoke send failed: {error}"));
            return Err(RemoteError::new(
                RemoteErrorKind::Transport,
                format!("invoke send failed: {error}"),
            ));
        }
        // Wire order is settled — release the send lock so later-allocated
        // invokes can transmit while this one awaits its correlated response.
        drop(_send_guard);

        match rx.recv().await {
            Some(result) => result,
            None => Err(RemoteError::new(
                RemoteErrorKind::Transport,
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
            let _ = entry.tx.try_send(Err(RemoteError::new(
                RemoteErrorKind::Transport,
                message.to_owned(),
            )));
        }
    }

    /// Invoke a port op and map the response to `SpokeResult` (contract
    /// §5.3/§8). Port calls always settle to `SpokeResult` (§8.3).
    async fn invoke_mapped<T: DeserializeOwned>(&self, op: &str, payload: Value) -> SpokeResult<T> {
        match self.invoke_op(op, payload).await {
            Ok(ConnectInvokeResponse::Variant0 { payload, .. }) => {
                match serde_json::from_value(payload) {
                    Ok(value) => spoke_ok(value),
                    Err(error) => internal_error(
                        "transport",
                        format!("response payload decode failed: {error}"),
                    ),
                }
            }
            Ok(ConnectInvokeResponse::Variant1 { error, .. }) => {
                SpokeResult::Reject(map_error_envelope(&error))
            }
            Err(error) => internal_error(error.kind.as_str(), error.message),
        }
    }
}

// ── BaselinePorts (async) — port-method → `port.*` catalogue (§5.2) ───────

#[async_trait]
impl KnowledgeEntryPort for RemoteAdapter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.invoke_mapped("port.knowledge.get", json!({ "entry_id": entry_id }))
            .await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.invoke_mapped(
            "port.knowledge.put",
            json!({
                "entry": entry,
                "expected_base_revision": expected_base_revision,
            }),
        )
        .await
    }
}

#[async_trait]
impl RelationPort for RemoteAdapter {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.invoke_mapped("port.relation.get", json!({ "relation_id": relation_id }))
            .await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.invoke_mapped(
            "port.relation.put",
            json!({
                "relation": relation,
                "expected_base_revision": expected_base_revision,
            }),
        )
        .await
    }
}

#[async_trait]
impl ScopeQueryPort for RemoteAdapter {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.invoke_mapped(
            "port.scope.list_knowledge_entries",
            json!({ "scope": scope }),
        )
        .await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.invoke_mapped("port.scope.list_timeline_events", json!({ "scope": scope }))
            .await
    }
}

#[async_trait]
impl FindingPort for RemoteAdapter {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.invoke_mapped("port.finding.put", json!({ "findings": findings }))
            .await
    }
}

#[async_trait]
impl RuleQueryPort for RemoteAdapter {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.invoke_mapped("port.rule.list", json!({ "rule_refs": rule_refs }))
            .await
    }
}

#[async_trait]
impl HostManifestPort for RemoteAdapter {
    /// HostManifestPort: "self" on a RemoteAdapter is the **remote** peer
    /// (contract §7). Returns the authenticated hello `host` from the session
    /// cache — cache-only, no network round-trip.
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        match self.remote_manifest.lock().expect("manifest lock").clone() {
            Some(manifest) => spoke_ok(manifest),
            None => internal_error("session_closed", "connect session is not established"),
        }
    }

    /// Proxy to the remote's product-seeded peer list (contract §7).
    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.invoke_mapped("port.host.list_peer_manifests", json!({}))
            .await
    }
}

// The blanket impl in spoke-operations lifts the six traits above into
// `BaselinePorts`, so `RemoteAdapter` is a drop-in for
// `orchestrate_upsert(remote, req)` etc.

/// Dial a remote peer over `transport`: perform the signed hello exchange +
/// session snapshot, then return an `Established` adapter. Errors on any
/// handshake / allowlist / verification failure (contract §3.3/§8.2 — no
/// half-open `BaselinePorts` instance).
pub async fn connect_remote_adapter(
    options: RemoteAdapterOptions,
) -> Result<Arc<RemoteAdapter>, RemoteAdapterError> {
    let RemoteAdapterOptions {
        transport,
        local_identity,
        local_manifest,
        remote_pubkey,
        allowlist,
        invoke_timeout_ms,
        capability_token,
    } = options;
    let invoke_timeout =
        Duration::from_millis(invoke_timeout_ms.unwrap_or(DEFAULT_INVOKE_TIMEOUT_MS));

    // Fail-closed before any transport I/O: the remote peer must be on the
    // allowlist (TS dial does the same checks first).
    let remote_peer_id = derive_peer_id_from_ed25519_pubkey(&remote_pubkey);
    if !is_allowlisted(&allowlist, &remote_peer_id) {
        return Err(RemoteAdapterError::Config(format!(
            "remote peer {remote_peer_id} is not on the allowlist (fail-closed)"
        )));
    }
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&local_identity.seed);
    let local_pubkey = signing_key.verifying_key().to_bytes();
    let local_peer_id = derive_peer_id_from_ed25519_pubkey(&local_pubkey);

    let adapter = Arc::new(RemoteAdapter::new(
        transport,
        remote_pubkey,
        invoke_timeout,
        capability_token,
    ));
    adapter.begin_handshake();

    let outcome = async {
        // 1. Send our signed hello (nonce generated internally — single-use).
        let nonce =
            generate_nonce().map_err(|error| RemoteAdapterError::Config(error.to_string()))?;
        let hello = sign_hello_ed25519(
            &local_identity.seed,
            &nonce,
            &connect_manifest(&local_manifest),
            None,
        )
        .map_err(|error| {
            RemoteAdapterError::Handshake(format!("local hello sign failed: {error}"))
        })?;
        let bytes = serde_json::to_vec(&hello).map_err(|error| {
            RemoteAdapterError::Handshake(format!("hello encode failed: {error}"))
        })?;
        bounded(adapter.transport.send(&bytes), invoke_timeout, "hello send")
            .await?
            .map_err(|error| {
                RemoteAdapterError::Handshake(format!("hello send failed: {error}"))
            })?;

        // 2. Await + verify the server's signed hello (signature + identity).
        let recv: Result<Vec<u8>, TransportError> =
            bounded(adapter.transport.recv(), invoke_timeout, "server hello").await?;
        let bytes = recv.map_err(|error| {
            RemoteAdapterError::Handshake(format!("server hello recv failed: {error}"))
        })?;
        let hello: ConnectHello = serde_json::from_slice(&bytes).map_err(|_| {
            RemoteAdapterError::Handshake("expected ConnectHello from server".into())
        })?;
        verify_hello_ed25519(&remote_pubkey, &remote_peer_id, &hello, Some(&nonce)).map_err(
            |error| match error {
                // A mixed-version hello is a version-negotiation failure, not
                // a generic handshake fault: preserve the dedicated kind so
                // callers (and the FFI dial surface) can tell it apart from
                // signature / identity / nonce failures.
                CoreError::ProtocolVersionMismatch { reason } => {
                    RemoteAdapterError::ProtocolVersionMismatch(format!(
                        "remote hello protocol version mismatch: {reason}"
                    ))
                }
                error => {
                    RemoteAdapterError::Handshake(format!("remote hello verification failed: {error}"))
                }
            },
        )?;
        // Receiver-side nonce single-use (spec §Nonce / replay protection;
        // parity with the TS dial): record the accepted server hello and
        // reject a replayed one. An active transport attacker cannot reuse a
        // previously-signed hello from the allowlisted peer to re-enter
        // `Established` on a later dial — the replay is rejected here,
        // before any `ConnectSession` snapshot (which a forged-session
        // attack would fabricate) is accepted.
        if !record_server_hello(&remote_peer_id, hello.nonce.as_str()) {
            return Err(RemoteAdapterError::Handshake(format!(
                "server hello replay rejected: (peer_id {remote_peer_id}, nonce {}) was already accepted",
                hello.nonce.as_str()
            )));
        }
        let remote_manifest: HostCapabilityManifest = {
            let wire: ConnectHostCapabilityManifest = hello.host;
            serde_json::from_value(serde_json::to_value(&wire).expect("hello host serializes"))
                .map_err(|error| {
                    RemoteAdapterError::Handshake(format!(
                        "remote hello host decode failed: {error}"
                    ))
                })?
        };

        // 3. Await + validate the responder-assigned session snapshot.
        //    Envelope-auth verify runs on the wire form BEFORE typed
        //    deserialization (`spoke-connect-session-jcs-v1` against the
        //    responder's hello Ed25519 public key; the step-6 peer-id
        //    binding assert replaces the old manual comparison). A missing /
        //    invalid signature or peer-id mismatch fails the dial closed —
        //    no session state exists yet.
        let recv: Result<Vec<u8>, TransportError> =
            bounded(adapter.transport.recv(), invoke_timeout, "session snapshot").await?;
        let bytes = recv.map_err(|error| {
            RemoteAdapterError::Handshake(format!("session snapshot recv failed: {error}"))
        })?;
        let session_doc: Value = serde_json::from_slice(&bytes).map_err(|_| {
            RemoteAdapterError::Handshake(
                "expected ConnectSession snapshot after server hello".into(),
            )
        })?;
        crate::core::verify_session_auth(
            &remote_pubkey,
            &session_doc,
            &local_peer_id,
            &remote_peer_id,
        )
        .map_err(|error| {
            RemoteAdapterError::Handshake(format!(
                "session snapshot verification failed: {error}"
            ))
        })?;
        let session_doc: ConnectSession =
            serde_json::from_value(session_doc).map_err(|_| {
                RemoteAdapterError::Handshake(
                    "expected ConnectSession snapshot after server hello".into(),
                )
            })?;
        if session_doc.session_id.as_str().is_empty() {
            return Err(RemoteAdapterError::Handshake(
                "session snapshot session_id must not be empty".into(),
            ));
        }
        if session_doc.initial_sequence != 0 {
            return Err(RemoteAdapterError::Handshake(
                "session snapshot initial_sequence must be 0".into(),
            ));
        }

        // 4. Bind the authenticated session and start the receive loop.
        //    The negotiated capability set is the intersection of the two
        //    authenticated hello manifests (mirror of TS
        //    `negotiatedCapabilities`); the reverse-invoke dispatch gate
        //    evaluates `tools.*` ops against it (frozen §3).
        let negotiated: Vec<String> = local_manifest
            .capabilities
            .iter()
            .filter(|cap| remote_manifest.capabilities.iter().any(|remote| remote == *cap))
            .cloned()
            .collect();
        adapter.establish(
            EstablishedSession {
                session_id: session_doc.session_id.to_string(),
                responder_peer_id: remote_peer_id,
                local_secret: local_identity.seed,
                negotiated_capabilities: negotiated,
                sequence: Mutex::new(OutboundSequence::new()),
                inbound: Mutex::new(InboundSequence::new()),
            },
            remote_manifest,
        );
        Ok(())
    }
    .await;

    match outcome {
        Ok(()) => Ok(adapter),
        Err(error) => {
            // Handshake rejection: release the transport so the peer sees a
            // clean disconnect (mirrors the TS dial rejection path).
            adapter.close();
            Err(error)
        }
    }
}

/// Bound `future` against the handshake/invoke deadline; elapse maps to a
/// [`RemoteAdapterError::Timeout`] with the TS dial message shape
/// (`connect: {what} timed out after {ms}ms`).
async fn bounded<F>(
    future: F,
    timeout: Duration,
    what: &str,
) -> Result<F::Output, RemoteAdapterError>
where
    F: std::future::Future,
{
    tokio::time::timeout(timeout, future).await.map_err(|_| {
        RemoteAdapterError::Timeout(format!(
            "connect: {what} timed out after {}ms",
            timeout.as_millis()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::MAX_SEQUENCE;
    use crate::remote::transport::loopback_transport_pair;

    /// Guard-level misclassification regression (frozen §4, mirror of the
    /// TS `classifies request shape before response shape` test): an
    /// op-bearing doc is a `ConnectInvokeRequest` — NEVER a response, even
    /// though a reverse request carries the same correlation echo fields +
    /// a payload as the response success branch. Without the hardening the
    /// reverse request would satisfy the response discriminator and be
    /// silently swallowed by the `request_id` demux.
    #[test]
    fn request_shaped_docs_are_never_demuxed_as_responses() {
        let request_shaped = json!({
            "session_id": "s",
            "sequence": 0,
            "request_id": "r",
            "op": "tools.math.add",
            "payload": { "arguments": {} },
            "signature": "x",
            "extensions": {},
        });
        assert!(is_connect_invoke_request(&request_shaped));
        assert_eq!(wire_response_correlation(&request_shaped), None);

        let response_shaped_with_op = json!({
            "session_id": "s",
            "sequence": 0,
            "request_id": "r",
            "op": "tools.math.add",
            "payload": { "result": 1 },
            "signature": "x",
            "extensions": {},
        });
        assert_eq!(wire_response_correlation(&response_shaped_with_op), None);

        // A real response (no `op`) still correlates — the hardening is
        // strictly an exclusion, never a regression for response demux.
        let response = json!({
            "session_id": "s",
            "sequence": 0,
            "request_id": "r",
            "payload": { "result": 1 },
            "signature": "x",
            "extensions": {},
        });
        assert_eq!(
            wire_response_correlation(&response),
            Some(Correlation {
                session_id: "s".into(),
                sequence: 0,
                request_id: "r".into(),
            })
        );
        assert!(!is_connect_invoke_request(&response));
    }

    /// Outbound sequence exhaustion: the next `allocate` fails, the session
    /// is closed (no wrap-around), and the port call settles to
    /// `INTERNAL_ERROR` `details.kind = "sequence_exhausted"` (contract
    /// §8.2 / §6 "Sequence overflow").
    ///
    /// This sits in the adapter module (not `tests/remote_loopback.rs`)
    /// because the private counter is only reachable through the test-only
    /// `OutboundSequence::set_next` (`#[cfg(test)] pub(crate)`), which
    /// integration tests cannot see; the TS twin (`setNext`) covers the same
    /// path in the loopback suite.
    #[tokio::test]
    async fn sequence_exhaustion_maps_to_internal_error_and_closes_session() {
        let pair = loopback_transport_pair();
        let adapter = Arc::new(RemoteAdapter::new(
            Arc::new(pair.client),
            [0xaa; 32], // unused here — no inbound responses are received
            Duration::from_millis(DEFAULT_INVOKE_TIMEOUT_MS),
            None,
        ));
        adapter.begin_handshake();
        let mut sequence = OutboundSequence::new();
        sequence.set_next(MAX_SEQUENCE + 1);
        let manifest: HostCapabilityManifest = serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": "test-host",
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline"],
            "namespaces": ["toy_world"],
            "extensions": {},
        }))
        .expect("valid manifest");
        adapter.establish(
            EstablishedSession {
                session_id: "test-session-exhausted".into(),
                responder_peer_id: "test-remote-peer".into(),
                local_secret: [0x2a; 32],
                negotiated_capabilities: vec!["spoke-baseline".to_owned()],
                sequence: Mutex::new(sequence),
                inbound: Mutex::new(InboundSequence::new()),
            },
            manifest,
        );

        let result = adapter.get_knowledge_entry("kb_tw_mira").await;
        match result {
            SpokeResult::Ok(_) => panic!("exhausted sequence must reject"),
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InternalError);
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(|kind| kind.as_str()),
                    Some("sequence_exhausted")
                );
            }
        }
        // Exhaustion makes the session unusable (no wrap-around) — Closed.
        assert_eq!(adapter.state(), RemoteAdapterState::Closed);
    }
}
