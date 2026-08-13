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
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use spoke_operations::{
    from_error_envelope, spoke_ok, spoke_reject, FindingPort, HostManifestPort, KnowledgeEntryPort,
    RelationPort, RuleQueryPort, ScopeQueryPort, SpokeReject, SpokeRejectCode, SpokeResult,
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
    check_response_correlation, derive_peer_id_from_ed25519_pubkey, is_allowlisted,
    sign_hello_ed25519, verify_hello_ed25519, CapabilityTokenProof, CoreError, Correlation,
    EnvelopeAuthError, EnvelopeAuthErrorKind, NonceStore, OutboundSequence,
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
enum RemoteErrorKind {
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
    const fn as_str(self) -> &'static str {
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
struct RemoteError {
    kind: RemoteErrorKind,
    message: String,
}

impl RemoteError {
    fn new(kind: RemoteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// `SpokeResult` reject with `INTERNAL_ERROR` + `details.kind` (contract
/// §8.2 transport/session/timeout/correlation rows).
fn internal_error<T>(kind: &str, message: impl Into<String>) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert("kind".into(), Value::String(kind.into()));
    spoke_reject(SpokeRejectCode::InternalError, message, Some(details))
}

/// Dispatch-deny wire codes (contract §8.2): the host answered that the op or
/// its required capability is not available → `CAPABILITY_PORT_MISSING`.
fn is_dispatch_deny(code: &str) -> bool {
    matches!(code, "op_unsupported" | "capability_missing")
}

/// Extract the correlation echo fields from a response-shaped wire document
/// (both `payload` and `error` branches carry the three fields at the top
/// level). `None` for any other envelope shape — the receive-loop
/// discriminator (mirror of TS `isConnectInvokeResponse`).
fn wire_response_correlation(doc: &Value) -> Option<Correlation> {
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
fn map_error_envelope(error: &WireErrorEnvelope) -> SpokeReject {
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
fn connect_manifest(manifest: &HostCapabilityManifest) -> ConnectHostCapabilityManifest {
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
    sequence: Mutex<OutboundSequence>,
}

/// A parked invoke: correlation material + the waiter channel + the timeout
/// task (mirror of TS `PendingInvoke`; the timeout task plays `setTimeout`).
struct PendingInvoke {
    correlation: Correlation,
    tx: tokio::sync::mpsc::Sender<Result<ConnectInvokeResponse, RemoteError>>,
    timeout_task: tokio::task::JoinHandle<()>,
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

    async fn receive_loop(&self) {
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
            // Response-shape discriminator on the wire form (mirror of TS
            // `isConnectInvokeResponse`): both branches carry the three echo
            // fields at the top level. Everything else (hello / session /
            // unknown shape) is a post-handshake stray envelope — ignored.
            // Unexpected invoke requests are host-role — out of the
            // single-peer client scope (contract §4).
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
        adapter.establish(
            EstablishedSession {
                session_id: session_doc.session_id.to_string(),
                responder_peer_id: remote_peer_id,
                local_secret: local_identity.seed,
                sequence: Mutex::new(OutboundSequence::new()),
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
                sequence: Mutex::new(sequence),
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
