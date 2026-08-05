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

    /// The capabilities negotiated for this session: the intersection of the
    /// local and remote `capabilities[]` at session establishment (normative
    /// rule, `.mstar/specs/spoke-connect.md` §Negotiation). The op dispatch
    /// gate is evaluated against this set — never against the remote
    /// manifest alone.
    #[must_use]
    pub fn negotiated_capabilities(&self) -> &[String] {
        &self.inner.negotiated_capabilities
    }

    /// Whether this session completed the capability-token challenge with a
    /// valid token (normative `capability_token_ok`). `false` while the
    /// challenge is pending or was rejected — invokes on a session whose
    /// peer requires a token are then rejected with an `auth_failed` wire
    /// envelope until a valid token is presented.
    #[must_use]
    pub fn capability_token_ok(&self) -> bool {
        self.inner.token_ok()
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
        self.invoke_with_auth(op, payload, None).await
    }

    /// Send a `ConnectInvokeRequest` with an optional capability-token
    /// `auth` proof (the `{ v, claims, sig }` object) and wait for the
    /// correlated response.
    ///
    /// When `auth` is present the receiver validates the proof on **every**
    /// invoke (normative §Challenge / response and invoke `auth`); when the
    /// session already completed the capability-token challenge, `None`
    /// suffices. Same correlation and error semantics as
    /// [`PeerSession::invoke`].
    pub async fn invoke_with_auth(
        &self,
        op: impl Into<String>,
        payload: serde_json::Value,
        auth: Option<serde_json::Value>,
    ) -> Result<InvokeSuccess, InvokeError> {
        let op = op.into();
        let sequence = self.inner.allocate_sequence()?;
        let request_id =
            generate_request_id().map_err(|e| InvokeError::Transport(e.to_string()))?;
        // Validate the caller-supplied op and the derived wire strings up
        // front (empty strings are caller errors, rejected exactly as the
        // pre-v2 path did — `envelope_auth` re-parses them infallibly after
        // this gate).
        let _: spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequestOp =
            op.parse().map_err(
                |e: spoke_schemas::connect::connect_invoke_request::error::ConversionError| {
                    InvokeError::Transport(format!("invalid op {op:?}: {e}"))
                },
            )?;
        let _: spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequestRequestId =
            request_id
                .parse()
                .map_err(|e| InvokeError::Transport(format!("invalid request_id: {e}")))?;
        let _: spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequestSessionId = self
            .inner
            .session_id
            .parse()
            .map_err(|e| InvokeError::Transport(format!("invalid session_id: {e}")))?;
        // Authenticate the outbound request with this node's hello identity
        // seed (`spoke-connect-invoke-request-jcs-v1` — v2 requires the
        // signature on every post-hello envelope). Signing is infallible
        // for the validated 32-byte seed stored on the session handle.
        let request = crate::core::authenticate_invoke_request(
            &self.inner.local_secret,
            &crate::core::InvokeRequestSignInput {
                session_id: self.inner.session_id.clone(),
                sequence: sequence as i64,
                request_id: request_id.clone(),
                op: op.clone(),
                payload: payload.clone(),
                auth: auth.clone(),
            },
            HashMap::new(),
        )
        .map_err(|e| InvokeError::Transport(format!("envelope auth signing failed: {e}")))?;
        self.inner.send_invoke(request).await
    }
}
