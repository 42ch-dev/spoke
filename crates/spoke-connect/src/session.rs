//! Peer sessions: per-peer session state and op invocation.
//!
//! A [`PeerSession`] is created by [`crate::SpokeConnectNode::connect`] once
//! both sides of a connection have completed the authenticated hello exchange
//! (see [`crate::node`]). It owns the per-session **outbound** sequence
//! counter (starts at 0, one counter per direction, never wraps) and routes
//! `ConnectInvokeRequest`s to the node's event loop, which correlates the
//! response by request-response id and verifies the wire echo
//! (`session_id`, `sequence`, `request_id`).
//!
//! Wire types are the generated `spoke-schemas` connect envelopes only. The
//! `ConnectInvokeResponse` error branch carries the codegen-*inline*
//! `ErrorEnvelope` (`spoke_schemas::connect::connect_invoke_response`) —
//! field-identical to `common::ErrorEnvelope` but a distinct generated type,
//! matching the `HostCapabilityManifest` inline precedent. `InvokeError::Wire`
//! uses that exact inline type so no conversion is ever needed.

use crate::error::{ConnectError, InvokeError};
use crate::node::LoopCommand;
use crate::protocol::MAX_SEQUENCE;
use libp2p::PeerId;
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::ConnectInvokeResponse;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Generate a request/session correlation id: UUID v4 (RFC 4122) from 128
/// bits of CSPRNG entropy. Uses the same `getrandom` source as
/// [`crate::generate_nonce`] so RNG failures surface as errors instead of
/// panicking.
pub(crate) fn generate_request_id() -> Result<String, ConnectError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| ConnectError::Transport(format!("CSPRNG failure: {e}")))?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string())
}

/// Per-session shared state (private to the crate).
///
/// The event loop creates one of these per accepted peer and hands out
/// [`PeerSession`] wrappers; `invoke` goes back through the loop via the
/// command channel.
#[derive(Debug)]
pub(crate) struct SessionHandle {
    session_id: String,
    remote_peer_id: PeerId,
    remote_manifest: HostCapabilityManifest,
    /// Outbound sequence counter, starting at 0 (per-direction, per session).
    next_sequence: AtomicU64,
    /// Set when the sequence space is exhausted or the session is otherwise
    /// unusable; further invokes fail fast with `SessionClosed`.
    closed: AtomicBool,
    /// Timeout applied while waiting for an invoke response.
    timeout: Duration,
    cmd_tx: mpsc::Sender<LoopCommand>,
}

impl SessionHandle {
    /// Create a new session handle for `peer` with the manifest carried by
    /// the peer's accepted hello.
    pub(crate) fn new(
        session_id: String,
        remote_peer_id: PeerId,
        remote_manifest: HostCapabilityManifest,
        timeout: Duration,
        cmd_tx: mpsc::Sender<LoopCommand>,
    ) -> Self {
        Self {
            session_id,
            remote_peer_id,
            remote_manifest,
            next_sequence: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            timeout,
            cmd_tx,
        }
    }

    /// Atomically assign the next outbound sequence.
    ///
    /// The counter starts at 0; on exhaustion (past [`MAX_SEQUENCE`], the
    /// JSON-safe wire maximum) the session is closed and
    /// `SequenceExhausted` is returned — sequences never wrap.
    fn allocate_sequence(&self) -> Result<u64, InvokeError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(InvokeError::SessionClosed);
        }
        // simplify: a single fetch_add can overshoot by one between
        // `closed` check and increment; u64 overflow is unreachable in
        // practice (2^53-1 calls per session). If sessions ever reach
        // saturation, replace with a compare_exchange loop.
        let sequence = self.next_sequence.fetch_add(1, Ordering::SeqCst);
        if sequence > MAX_SEQUENCE {
            self.closed.store(true, Ordering::SeqCst);
            return Err(InvokeError::SequenceExhausted);
        }
        Ok(sequence)
    }

    async fn send_invoke(
        &self,
        request: ConnectInvokeRequest,
    ) -> Result<InvokeSuccess, InvokeError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(LoopCommand::Invoke {
                peer: self.remote_peer_id,
                request,
                reply: reply_tx,
            })
            .await
            .map_err(|_| InvokeError::SessionClosed)?;
        tokio::time::timeout(self.timeout, reply_rx)
            .await
            .map_err(|_| InvokeError::Transport("invoke response timed out".into()))?
            .map_err(|_| InvokeError::SessionClosed)?
    }
}

/// A live session with an authenticated peer.
///
/// `Clone` shares one session (one sequence counter); concurrent `invoke`
/// calls receive distinct atomic sequences. All handles are `Send + Sync`.
#[derive(Debug, Clone)]
pub struct PeerSession {
    inner: Arc<SessionHandle>,
}

impl PeerSession {
    pub(crate) fn new(handle: Arc<SessionHandle>) -> Self {
        Self { inner: handle }
    }

    /// This session's opaque id (UUID v4 string).
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.inner.session_id
    }

    /// The noise-authenticated remote peer.
    #[must_use]
    pub fn remote_peer_id(&self) -> PeerId {
        self.inner.remote_peer_id
    }

    /// The remote `HostCapabilityManifest` carried by the peer's accepted
    /// hello (wire type from `spoke-schemas`).
    #[must_use]
    pub fn remote_manifest(&self) -> &HostCapabilityManifest {
        &self.inner.remote_manifest
    }

    /// The next sequence that will be assigned on the following `invoke`
    /// (starts at 0).
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.inner.next_sequence.load(Ordering::SeqCst)
    }

    /// Send a `ConnectInvokeRequest` and wait for the correlated
    /// `ConnectInvokeResponse`.
    ///
    /// `op` is an open string from the connect `op` core vocabulary (e.g.
    /// `"check"`); `payload` is the opaque ops request envelope JSON. On
    /// success, `sequence` is the session's outbound sequence, `request_id`
    /// echoes the generated correlation id, and `payload` is the opaque ops
    /// response success body. Remote application failures return
    /// [`InvokeError::Wire`]; transport / session failures use the other
    /// variants.
    pub async fn invoke(
        &self,
        op: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<InvokeSuccess, InvokeError> {
        let op = op.into();
        let sequence = self.inner.allocate_sequence()?;
        let request_id =
            generate_request_id().map_err(|e| InvokeError::Transport(e.to_string()))?;
        let request = ConnectInvokeRequest {
            auth: None,
            extensions: HashMap::new(),
            op: op.parse().map_err(
                |e: spoke_schemas::connect::connect_invoke_request::error::ConversionError| {
                    InvokeError::Transport(format!("invalid op {op:?}: {e}"))
                },
            )?,
            payload,
            request_id: request_id
                .parse()
                .map_err(|e| InvokeError::Transport(format!("invalid request_id: {e}")))?,
            sequence: sequence as i64,
            session_id: self
                .inner
                .session_id
                .parse()
                .map_err(|e| InvokeError::Transport(format!("invalid session_id: {e}")))?,
        };
        self.inner.send_invoke(request).await
    }
}

/// Successful op invocation: the correlated response success branch.
#[derive(Debug, Clone)]
pub struct InvokeSuccess {
    /// The session's outbound sequence assigned to this invoke.
    pub sequence: u64,
    /// The generated correlation id, echoed by the responder.
    pub request_id: String,
    /// Ops response success body (`OpaqueJson`).
    pub payload: serde_json::Value,
}

/// Verify that a response echoes the request's `session_id`, `sequence`, and
/// `request_id` (normative echo rule).
pub(crate) fn verify_correlation(
    request: &ConnectInvokeRequest,
    response: &ConnectInvokeResponse,
) -> Result<(), InvokeError> {
    let (session_id, sequence, request_id) = match response {
        ConnectInvokeResponse::Variant0 {
            session_id,
            sequence,
            request_id,
            ..
        }
        | ConnectInvokeResponse::Variant1 {
            session_id,
            sequence,
            request_id,
            ..
        } => (session_id.as_str(), *sequence, request_id.as_str()),
    };
    if session_id != request.session_id.as_str() {
        return Err(InvokeError::CorrelationMismatch);
    }
    if sequence != request.sequence {
        return Err(InvokeError::CorrelationMismatch);
    }
    if request_id != request.request_id.as_str() {
        return Err(InvokeError::CorrelationMismatch);
    }
    Ok(())
}

/// Map a correlated response to [`InvokeSuccess`] or the wire error branch.
pub(crate) fn map_invoke_response(
    request: &ConnectInvokeRequest,
    response: ConnectInvokeResponse,
) -> Result<InvokeSuccess, InvokeError> {
    verify_correlation(request, &response)?;
    match response {
        ConnectInvokeResponse::Variant0 {
            sequence,
            request_id,
            payload,
            ..
        } => Ok(InvokeSuccess {
            sequence: sequence as u64,
            request_id,
            payload,
        }),
        ConnectInvokeResponse::Variant1 { error, .. } => Err(InvokeError::Wire(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;

    fn manifest(host_id: &str) -> HostCapabilityManifest {
        HostCapabilityManifest {
            authority: None,
            capabilities: vec!["spoke-connect".into()],
            extensions: Default::default(),
            host_id: host_id.parse().expect("host id parses"),
            namespaces: Vec::new(),
            roles: vec!["data-store".into()],
            schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
        }
    }

    fn handle(timeout: Duration) -> Arc<SessionHandle> {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        Arc::new(SessionHandle::new(
            "sess-1".into(),
            PeerId::random(),
            manifest("remote-host"),
            timeout,
            cmd_tx,
        ))
    }

    fn request(sequence: i64) -> ConnectInvokeRequest {
        ConnectInvokeRequest {
            auth: None,
            extensions: HashMap::new(),
            op: "check".parse().expect("op"),
            payload: serde_json::json!({ "scope": { "scope_id": "s1" } }),
            request_id: "req-1".parse().expect("request id"),
            sequence,
            session_id: "sess-1".parse().expect("session id"),
        }
    }

    fn success_response(sequence: i64, request_id: &str) -> ConnectInvokeResponse {
        ConnectInvokeResponse::Variant0 {
            extensions: HashMap::new(),
            payload: serde_json::json!({ "findings": [] }),
            request_id: request_id.into(),
            sequence,
            session_id: "sess-1".into(),
        }
    }

    fn error_response(sequence: i64) -> ConnectInvokeResponse {
        ConnectInvokeResponse::Variant1 {
            error: ErrorEnvelope {
                code: "check_failed".into(),
                details: Default::default(),
                extensions: HashMap::new(),
                message: "spike check failed".into(),
            },
            extensions: HashMap::new(),
            request_id: "req-1".into(),
            sequence,
            session_id: "sess-1".into(),
        }
    }

    #[test]
    fn sequence_starts_at_zero_and_increments_monotonically() {
        let session = PeerSession::new(handle(Duration::from_secs(5)));
        assert_eq!(session.next_sequence(), 0);
        let h = &session.inner;
        assert_eq!(h.allocate_sequence().expect("first"), 0);
        assert_eq!(h.allocate_sequence().expect("second"), 1);
        assert_eq!(h.allocate_sequence().expect("third"), 2);
        assert_eq!(session.next_sequence(), 3);
    }

    #[test]
    fn sequence_exhaustion_closes_the_session() {
        let session = PeerSession::new(handle(Duration::from_secs(5)));
        let h = &session.inner;
        h.next_sequence.store(MAX_SEQUENCE, Ordering::SeqCst);
        assert_eq!(h.allocate_sequence().expect("last valid"), MAX_SEQUENCE);
        let err = h.allocate_sequence().expect_err("exhausted");
        assert!(matches!(err, InvokeError::SequenceExhausted));
        // The session is closed: further invokes fail fast.
        let err = h.allocate_sequence().expect_err("closed");
        assert!(matches!(err, InvokeError::SessionClosed));
    }

    #[test]
    fn success_response_maps_to_invoke_success() {
        let req = request(0);
        let mapped = map_invoke_response(&req, success_response(0, "req-1")).expect("success");
        assert_eq!(mapped.sequence, 0);
        assert_eq!(mapped.request_id, "req-1");
        assert_eq!(mapped.payload, serde_json::json!({ "findings": [] }));
    }

    #[test]
    fn error_branch_maps_to_invoke_error_wire() {
        let req = request(1);
        let err = map_invoke_response(&req, error_response(1)).expect_err("wire error");
        match err {
            InvokeError::Wire(envelope) => {
                assert_eq!(envelope.code, "check_failed");
                assert_eq!(envelope.message, "spike check failed");
            }
            other => panic!("expected Wire, got {other:?}"),
        }
    }

    #[test]
    fn correlation_mismatches_are_detected() {
        // Wrong sequence echo.
        let err = map_invoke_response(&request(0), success_response(1, "req-1"))
            .expect_err("sequence mismatch");
        assert!(matches!(err, InvokeError::CorrelationMismatch));
        // Wrong request_id echo.
        let err = map_invoke_response(&request(0), success_response(0, "other-req"))
            .expect_err("request id mismatch");
        assert!(matches!(err, InvokeError::CorrelationMismatch));
        // Wrong session_id echo (error branch too).
        let mut resp = error_response(0);
        if let ConnectInvokeResponse::Variant1 { session_id, .. } = &mut resp {
            *session_id = "other-session".into();
        }
        let err = map_invoke_response(&request(0), resp).expect_err("session mismatch");
        assert!(matches!(err, InvokeError::CorrelationMismatch));
    }

    #[test]
    fn request_ids_are_unique_uuid_v4_strings() {
        let a = generate_request_id().expect("id a");
        let b = generate_request_id().expect("id b");
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        // Version nibble is 4 (RFC 4122 v4).
        assert_eq!(a.as_bytes()[14], b'4');
        let dashes: Vec<char> = a.chars().filter(|c| *c == '-').collect();
        assert_eq!(dashes.len(), 4);
    }
}
