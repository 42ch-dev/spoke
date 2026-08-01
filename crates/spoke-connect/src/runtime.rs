//! Internal runtime plumbing shared between the node event loop and session
//! handles.
//!
//! Everything in this module is crate-private; the public facade re-exports
//! only the locked API surface at the crate root (`lib.rs`). Keeping the
//! command channel protocol, the per-session shared state, and the response
//! correlation material here breaks the node ↔ session dependency cycle and
//! keeps libp2p transport internals out of the public surface.

use crate::core;
use crate::error::{map_core_invoke_error, ConnectError, InvokeError};
use libp2p::{Multiaddr, PeerId};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::ConnectInvokeResponse;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Generate a request/session correlation id: UUID v4 (RFC 4122) from 128
/// bits of CSPRNG entropy. Uses the same `getrandom` source as
/// [`crate::hello::generate_nonce`] so RNG failures surface as errors instead
/// of panicking.
pub(crate) fn generate_request_id() -> Result<String, ConnectError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| ConnectError::Transport(format!("CSPRNG failure: {e}")))?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string())
}

/// Commands from public handles to the node's event loop.
///
/// The invoke variant carries the full request envelope (HashMap + opaque
/// Value) — it is moved into the channel once and sent to the peer; the
/// loop-side correlation state stores only the small echo fields (see
/// [`InvokeCorrelation`]). The enum is a short-lived channel payload, so the
/// size difference is not worth boxing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum LoopCommand {
    /// Dial an address and complete the hello handshake into a session.
    Connect {
        addr: Multiaddr,
        reply: oneshot::Sender<Result<Arc<SessionHandle>, ConnectError>>,
    },
    /// Send an invoke request to `peer` and correlate the response.
    Invoke {
        peer: PeerId,
        request: ConnectInvokeRequest,
        reply: oneshot::Sender<Result<InvokeSuccess, InvokeError>>,
    },
}

/// The minimal correlation material needed to verify and map a response: the
/// request's wire echo fields (`session_id`, `sequence`, `request_id`) plus
/// the peer it was sent to.
#[derive(Debug)]
pub(crate) struct InvokeCorrelation {
    pub(crate) peer: PeerId,
    pub(crate) session_id: String,
    pub(crate) sequence: i64,
    pub(crate) request_id: String,
}

impl From<&InvokeCorrelation> for core::Correlation {
    fn from(correlation: &InvokeCorrelation) -> Self {
        Self {
            session_id: correlation.session_id.clone(),
            sequence: correlation.sequence,
            request_id: correlation.request_id.clone(),
        }
    }
}

/// Per-session shared state (private to the crate).
///
/// The event loop creates one of these per accepted peer and hands out
/// [`crate::PeerSession`] wrappers; `invoke` goes back through the loop via
/// the command channel.
#[derive(Debug)]
pub(crate) struct SessionHandle {
    pub(crate) session_id: String,
    pub(crate) remote_peer_id: PeerId,
    pub(crate) remote_manifest: HostCapabilityManifest,
    /// Capabilities negotiated at session establishment: the intersection of
    /// the local and remote `HostCapabilityManifest.capabilities` (normative
    /// rule, `.mstar/specs/spoke-connect.md` §Negotiation). The inbound op
    /// dispatch gate is evaluated against this set, never against the remote
    /// manifest alone.
    pub(crate) negotiated_capabilities: Vec<String>,
    /// Outbound sequence counter, starting at 0 (per-direction, per session).
    /// The pure counter rules live in [`core::OutboundSequence`]; the mutex is
    /// the transport's synchronization around them (concurrent invokes get
    /// distinct sequences).
    pub(crate) next_sequence: Mutex<core::OutboundSequence>,
    /// Set when the sequence space is exhausted or the session is otherwise
    /// unusable; further invokes fail fast with `SessionClosed`.
    pub(crate) closed: AtomicBool,
    /// Timeout applied while waiting for an invoke response.
    pub(crate) timeout: Duration,
    pub(crate) cmd_tx: mpsc::Sender<LoopCommand>,
}

impl SessionHandle {
    /// Create a new session handle for `peer` with the manifest carried by
    /// the peer's accepted hello and the capability set negotiated at
    /// establishment.
    pub(crate) fn new(
        session_id: String,
        remote_peer_id: PeerId,
        remote_manifest: HostCapabilityManifest,
        negotiated_capabilities: Vec<String>,
        timeout: Duration,
        cmd_tx: mpsc::Sender<LoopCommand>,
    ) -> Self {
        Self {
            session_id,
            remote_peer_id,
            remote_manifest,
            negotiated_capabilities,
            next_sequence: Mutex::new(core::OutboundSequence::new()),
            closed: AtomicBool::new(false),
            timeout,
            cmd_tx,
        }
    }

    /// Mark the session closed; further invokes fail fast with
    /// `SessionClosed`. The event loop calls this when the peer's last
    /// connection closes, before removing the session from its map.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Atomically assign the next outbound sequence.
    ///
    /// The counter starts at 0; on exhaustion (past the JSON-safe wire
    /// maximum) the session is closed and `SequenceExhausted` is returned —
    /// sequences never wrap. The allocate rule itself is `core`'s
    /// ([`core::OutboundSequence::allocate`]).
    pub(crate) fn allocate_sequence(&self) -> Result<u64, InvokeError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(InvokeError::SessionClosed);
        }
        let mut sequence = self
            .next_sequence
            .lock()
            .expect("sequence lock is never poisoned");
        sequence.allocate().map_err(|err| {
            // `allocate` only returns `SequenceExhausted`; exhaustion closes
            // the session so later invokes fail fast with `SessionClosed`.
            if matches!(err, core::CoreInvokeError::SequenceExhausted) {
                self.closed.store(true, Ordering::SeqCst);
            }
            map_core_invoke_error(err)
        })
    }

    pub(crate) async fn send_invoke(
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
/// `request_id` (normative echo rule — the check itself is
/// [`core::check_response_correlation`]).
pub(crate) fn verify_correlation(
    correlation: &InvokeCorrelation,
    response: &ConnectInvokeResponse,
) -> Result<(), InvokeError> {
    core::check_response_correlation(
        &core::Correlation::from(correlation),
        &core::Correlation::from(response),
    )
    .map_err(map_core_invoke_error)
}

/// Map a correlated response to [`InvokeSuccess`] or the wire error branch.
pub(crate) fn map_invoke_response(
    correlation: &InvokeCorrelation,
    response: ConnectInvokeResponse,
) -> Result<InvokeSuccess, InvokeError> {
    verify_correlation(correlation, &response)?;
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
    use crate::protocol::MAX_SEQUENCE;
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
        let manifest = manifest("remote-host");
        Arc::new(SessionHandle::new(
            "sess-1".into(),
            PeerId::random(),
            manifest.clone(),
            manifest.capabilities,
            timeout,
            cmd_tx,
        ))
    }

    fn correlation(sequence: i64) -> InvokeCorrelation {
        InvokeCorrelation {
            peer: PeerId::random(),
            session_id: "sess-1".into(),
            sequence,
            request_id: "req-1".into(),
        }
    }

    fn success_response(sequence: i64, request_id: &str) -> ConnectInvokeResponse {
        ConnectInvokeResponse::Variant0 {
            extensions: Default::default(),
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
                extensions: Default::default(),
                message: "spike check failed".into(),
            },
            extensions: Default::default(),
            request_id: "req-1".into(),
            sequence,
            session_id: "sess-1".into(),
        }
    }

    #[test]
    fn sequence_starts_at_zero_and_increments_monotonically() {
        let h = handle(Duration::from_secs(5));
        assert_eq!(h.allocate_sequence().expect("first"), 0);
        assert_eq!(h.allocate_sequence().expect("second"), 1);
        assert_eq!(h.allocate_sequence().expect("third"), 2);
        assert_eq!(h.next_sequence.lock().expect("lock").next(), 3);
    }

    #[test]
    fn sequence_exhaustion_closes_the_session() {
        let h = handle(Duration::from_secs(5));
        h.next_sequence.lock().expect("lock").set_next(MAX_SEQUENCE);
        assert_eq!(h.allocate_sequence().expect("last valid"), MAX_SEQUENCE);
        let err = h.allocate_sequence().expect_err("exhausted");
        assert!(matches!(err, InvokeError::SequenceExhausted));
        // The session is closed: further invokes fail fast.
        let err = h.allocate_sequence().expect_err("closed");
        assert!(matches!(err, InvokeError::SessionClosed));
    }

    #[test]
    fn close_marks_the_session_closed() {
        let h = handle(Duration::from_secs(5));
        assert_eq!(h.allocate_sequence().expect("open"), 0);
        h.close();
        let err = h.allocate_sequence().expect_err("closed");
        assert!(matches!(err, InvokeError::SessionClosed));
    }

    #[test]
    fn success_response_maps_to_invoke_success() {
        let mapped =
            map_invoke_response(&correlation(0), success_response(0, "req-1")).expect("success");
        assert_eq!(mapped.sequence, 0);
        assert_eq!(mapped.request_id, "req-1");
        assert_eq!(mapped.payload, serde_json::json!({ "findings": [] }));
    }

    #[test]
    fn error_branch_maps_to_invoke_error_wire() {
        let err = map_invoke_response(&correlation(1), error_response(1)).expect_err("wire error");
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
        let err = map_invoke_response(&correlation(0), success_response(1, "req-1"))
            .expect_err("sequence mismatch");
        assert!(matches!(err, InvokeError::CorrelationMismatch));
        // Wrong request_id echo.
        let err = map_invoke_response(&correlation(0), success_response(0, "other-req"))
            .expect_err("request id mismatch");
        assert!(matches!(err, InvokeError::CorrelationMismatch));
        // Wrong session_id echo (error branch too).
        let mut resp = error_response(0);
        if let ConnectInvokeResponse::Variant1 { session_id, .. } = &mut resp {
            *session_id = "other-session".into();
        }
        let err = map_invoke_response(&correlation(0), resp).expect_err("session mismatch");
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
