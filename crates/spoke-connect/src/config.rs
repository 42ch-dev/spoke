//! [`ConnectConfig`] — node configuration and validation.

use crate::error::ConnectError;
use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;
use std::collections::HashMap;
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

/// Capability-token proof supplier for challenge responses.
///
/// Called with the **audience** peer id (the challenger's `peer_id`, which
/// becomes the token's `aud`) and returns the wire `proof` object (`{ v,
/// claims, sig }` — see `core::capability_token`). The returned proof must
/// be issued by a trusted issuer with `sub` = this node's `peer_id`.
///
/// Execution contract: like [`InvokeHandler`], the provider runs
/// **synchronously on the node's network event loop** — it must return
/// promptly and must not block on I/O (every handshake, invoke response,
/// and timeout sweep on the node stalls behind it).
/// (simplify: spike answers challenges from a supplier hook; products may
/// hold a token cache or mint on demand.)
pub type CapabilityTokenProvider = dyn Fn(&str) -> Result<serde_json::Value, String> + Send + Sync;

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

    /// Product-defined op → required capability map, consulted by the
    /// inbound dispatch gate for ops outside the core-op table (the pure
    /// core answers `None` for them — see `core::required_capability`).
    /// Default empty: a product op is not dispatchable until its required
    /// capability is configured here AND that capability is part of the
    /// session's `negotiated_capabilities` (normative §Op dispatch gate).
    pub op_capability_requirements: HashMap<String, String>,

    /// Trusted capability-token issuers: `peer_id` strings whose signed
    /// tokens this node accepts (parallel to `peer_allowlist`). **Empty ⇒
    /// the capability-token method is disabled**: challenges are not
    /// offered and any presented proof is rejected (fail closed).
    pub trusted_issuers: Vec<String>,

    /// Whether every session must complete the capability-token challenge
    /// before invokes are accepted (normative §Challenge / response and
    /// invoke `auth`). Effective only with a non-empty `trusted_issuers`;
    /// default `false` keeps the `noise-peerid`-only behavior.
    pub require_capability_token: bool,

    /// Supplies the capability-token proof this node presents when a peer
    /// challenges it. `None` ⇒ the node cannot answer challenges: sessions
    /// it dials stay unauthorized for invokes when the peer's policy
    /// requires a token.
    pub capability_token_provider: Option<Arc<CapabilityTokenProvider>>,

    /// Whether mDNS-discovered peers are dialed automatically (builds with
    /// the `mdns` feature only). Default `true` — same-LAN dev convenience.
    /// Discovery never grants trust: only allowlisted discoveries are
    /// dialed, and the dial passes the same `ConnectionEstablished`
    /// allowlist gate as an explicit `connect(addr)`.
    #[cfg(feature = "mdns")]
    pub mdns_autodial: bool,
}

impl fmt::Debug for ConnectConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The handler closure is not Debug; render it as an opaque marker.
        let mut builder = f.debug_struct("ConnectConfig");
        builder
            .field("identity", &self.identity)
            .field("peer_allowlist", &self.peer_allowlist)
            .field("listen_addrs", &self.listen_addrs)
            .field("local_manifest", &self.local_manifest)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("invoke_handler", &"<handler>")
            .field(
                "op_capability_requirements",
                &self.op_capability_requirements,
            )
            .field("trusted_issuers", &self.trusted_issuers)
            .field("require_capability_token", &self.require_capability_token)
            .field("capability_token_provider", &"<provider>");
        #[cfg(feature = "mdns")]
        builder.field("mdns_autodial", &self.mdns_autodial);
        builder.finish()
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
            op_capability_requirements: self.op_capability_requirements.clone(),
            trusted_issuers: self.trusted_issuers.clone(),
            require_capability_token: self.require_capability_token,
            capability_token_provider: self.capability_token_provider.clone(),
            #[cfg(feature = "mdns")]
            mdns_autodial: self.mdns_autodial,
        }
    }
}

impl ConnectConfig {
    /// Effective handshake timeout (`handshake_timeout` or the default).
    #[must_use]
    pub fn effective_handshake_timeout(&self) -> Duration {
        self.handshake_timeout.unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT)
    }

    /// Whether the capability-token challenge policy is active on this node:
    /// non-empty `trusted_issuers` **and** `require_capability_token`. When
    /// active, sessions must complete the challenge before invokes are
    /// accepted (and the node challenges every new session). An empty
    /// `trusted_issuers` disables the method regardless of the require flag.
    #[must_use]
    pub fn token_policy_active(&self) -> bool {
        self.require_capability_token && !self.trusted_issuers.is_empty()
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
            op_capability_requirements: HashMap::new(),
            trusted_issuers: Vec::new(),
            require_capability_token: false,
            capability_token_provider: None,
            #[cfg(feature = "mdns")]
            mdns_autodial: true,
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

    #[test]
    fn token_policy_is_disabled_by_default() {
        // Defaults (empty trusted_issuers, require false) keep the
        // noise-peerid-only behavior: no challenge policy.
        assert!(!config().token_policy_active());
    }

    #[test]
    fn token_policy_requires_both_trusted_issuers_and_require_flag() {
        let mut cfg = config();
        // trusted_issuers alone does not activate the policy (no automatic
        // challenge; tokens are still validated when `auth` is attached).
        cfg.trusted_issuers = vec!["issuer-a".into()];
        assert!(!cfg.token_policy_active());
        // require flag alone does not activate it either (empty issuer list
        // means the method is disabled and proofs are rejected).
        cfg.trusted_issuers.clear();
        cfg.require_capability_token = true;
        assert!(!cfg.token_policy_active());
        // Both together activate the challenge policy.
        cfg.trusted_issuers = vec!["issuer-a".into()];
        assert!(cfg.token_policy_active());
    }
}
