//! Peer sessions: the public session handle over the shared per-session
//! state.
//!
//! A [`PeerSession`] is created by [`crate::SpokeConnectNode::connect`] once
//! both sides of a connection have completed the authenticated hello exchange
//! (see [`crate::node`]). The shared state itself — the per-session outbound
//! sequence counter (starts at 0, never wraps), the invoke command channel,
//! and the response correlation rules — lives in
//! [`crate::runtime::SessionHandle`], which the node event loop and the
//! session share. The receiver-side inbound expectation (next expected
//! inbound sequence per session, also starting at 0) is tracked by the node
//! event loop alongside its sessions — see [`crate::node`].
//!
//! Wire types are the generated `spoke-schemas` connect envelopes only. The
//! `ConnectInvokeResponse` error branch carries the codegen-*inline*
//! `ErrorEnvelope` (`spoke_schemas::connect::connect_invoke_response`) —
//! field-identical to `common::ErrorEnvelope` but a distinct generated type,
//! matching the `HostCapabilityManifest` inline precedent. `InvokeError::Wire`
//! uses that exact inline type so no conversion is ever needed.

use crate::error::InvokeError;
use crate::runtime::{generate_request_id, InvokeSuccess, SessionHandle};
use libp2p::PeerId;
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use std::collections::HashMap;
use std::sync::Arc;

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

    /// This session's opaque id (UUID v4 string). Session ids are per-side in
    /// protocol v1 — each node records its own opaque id for the pairing.
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
        self.inner
            .next_sequence
            .lock()
            .expect("sequence lock is never poisoned")
            .next()
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
