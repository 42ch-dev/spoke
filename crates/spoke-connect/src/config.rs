//! [`ConnectConfig`] — node configuration and validation.

use crate::error::ConnectError;
use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Default dial / handshake timeout when [`ConnectConfig::handshake_timeout`]
/// is not set. ≥ 5s keeps loopback CI reliable.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Remote op dispatcher hook (spike-scoped).
///
/// Called by the accept path for every inbound invoke: `(op, payload)` →
/// opaque ops response success body or a wire [`ErrorEnvelope`]. The
/// dispatcher is **adapter-owned** per the connect spec — this hook exists so
/// the reference spike and its tests can close the invoke loop; it is not
/// part of the locked uniffi-facing surface.
///
/// Execution contract: the hook runs **synchronously on the node's network
/// event loop**. It must return promptly and must not block on I/O — every
/// handshake, invoke response, and timeout sweep on the node stalls behind
/// it. Panics are contained (the invoke is answered with an `internal_error`
/// wire envelope and the node keeps running) but remain a caller bug.
/// (simplify: the spike dispatches inline on the loop; products should
/// dispatch off-loop — e.g. `tokio::task::spawn_blocking` or an adapter task
/// pool — once handler latency matters.)
pub type InvokeHandler =
    dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, ErrorEnvelope> + Send + Sync;

/// Node configuration.
///
/// All fields are public; [`crate::SpokeConnectNode::start`] validates the
/// combination (see [`ConnectConfig::validate`]).
pub struct ConnectConfig {
    /// libp2p identity keypair (Ed25519). The derived `PeerId` is this node's
    /// network identity and the local hello signer. Rust-only this iteration;
    /// a uniffi binding surface will construct keypairs from byte seeds
    /// instead of exposing `Keypair`.
    pub identity: Keypair,

    /// PeerId allowlist (trust root). Empty allowlist = reject all remote
    /// peers (fail-closed).
    pub peer_allowlist: Vec<PeerId>,

    /// Listen multiaddrs (loopback tests use `127.0.0.1/tcp/0`).
    pub listen_addrs: Vec<Multiaddr>,

    /// Local `HostCapabilityManifest` advertised in the signed hello
    /// (spoke-schemas type).
    pub local_manifest: HostCapabilityManifest,

    /// Dial / handshake timeout. `None` applies
    /// [`DEFAULT_HANDSHAKE_TIMEOUT`].
    pub handshake_timeout: Option<Duration>,

    /// Optional remote op dispatcher for inbound invokes. `None` answers
    /// every inbound invoke with an `op_unsupported` error envelope.
    pub invoke_handler: Option<Arc<InvokeHandler>>,
}

impl fmt::Debug for ConnectConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The handler closure is not Debug; render it as an opaque marker.
        f.debug_struct("ConnectConfig")
            .field("identity", &self.identity)
            .field("peer_allowlist", &self.peer_allowlist)
            .field("listen_addrs", &self.listen_addrs)
            .field("local_manifest", &self.local_manifest)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("invoke_handler", &"<handler>")
            .finish()
    }
}

impl Clone for ConnectConfig {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            peer_allowlist: self.peer_allowlist.clone(),
            listen_addrs: self.listen_addrs.clone(),
            local_manifest: self.local_manifest.clone(),
            handshake_timeout: self.handshake_timeout,
            invoke_handler: self.invoke_handler.clone(),
        }
    }
}

impl ConnectConfig {
    /// Effective handshake timeout (`handshake_timeout` or the default).
    #[must_use]
    pub fn effective_handshake_timeout(&self) -> Duration {
        self.handshake_timeout.unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT)
    }

    /// Validate the configuration.
    ///
    /// Checks: at least one listen address. An empty allowlist is **valid**
    /// (fail-closed semantics); the manifest's `schema_version` is enforced
    /// by its `NonZeroU64` type.
    pub fn validate(&self) -> Result<(), ConnectError> {
        if self.listen_addrs.is_empty() {
            return Err(ConnectError::Config(
                "listen_addrs must not be empty".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    fn manifest() -> HostCapabilityManifest {
        HostCapabilityManifest {
            authority: None,
            capabilities: vec!["spoke-baseline".into()],
            extensions: Default::default(),
            host_id: "test-host".parse().expect("host id parses"),
            namespaces: Vec::new(),
            roles: vec!["data-store".into()],
            schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
        }
    }

    fn config() -> ConnectConfig {
        ConnectConfig {
            identity: Keypair::generate_ed25519(),
            peer_allowlist: Vec::new(),
            listen_addrs: vec!["/ip4/127.0.0.1/tcp/0".parse().expect("multiaddr")],
            local_manifest: manifest(),
            handshake_timeout: None,
            invoke_handler: None,
        }
    }

    #[test]
    fn empty_listen_addrs_rejected() {
        let mut cfg = config();
        cfg.listen_addrs.clear();
        assert!(matches!(cfg.validate(), Err(ConnectError::Config(_))));
    }

    #[test]
    fn empty_allowlist_is_valid_fail_closed_config() {
        // Empty allowlist is a *valid* configuration: it rejects all peers.
        assert!(config().validate().is_ok());
    }

    #[test]
    fn default_handshake_timeout_applied() {
        assert_eq!(
            config().effective_handshake_timeout(),
            DEFAULT_HANDSHAKE_TIMEOUT
        );
        let mut cfg = config();
        cfg.handshake_timeout = Some(Duration::from_secs(30));
        assert_eq!(cfg.effective_handshake_timeout(), Duration::from_secs(30));
    }
}
