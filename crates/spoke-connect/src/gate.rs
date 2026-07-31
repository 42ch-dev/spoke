//! `noise-peerid` accept gate: allowlist check + per-sender nonce dedup.
//!
//! Normative rules (`.mstar/specs/spoke-connect.md` §Auth model, §Nonce):
//! - Trust root is the deployment-configured `PeerId` allowlist; the peer is
//!   the **noise-authenticated** remote id, never the hello's claimed string.
//! - Empty allowlist rejects all remote peers (fail-closed).
//! - Nonce uniqueness is scoped per sender `peer_id`; the receiver must reject
//!   a hello whose `(peer_id, nonce)` pair was already accepted. An in-memory
//!   set for the life of the process is sufficient for protocol v1.

use crate::error::ConnectError;
use crate::hello::verify_hello;
use crate::protocol::PROTOCOL_VERSION;
use libp2p::identity::PublicKey;
use libp2p::PeerId;
use spoke_schemas::connect::ConnectHello;
use std::collections::HashSet;
/// Whether `peer` is on the allowlist. An empty allowlist rejects every peer.
#[must_use]
pub(crate) fn is_allowlisted(allowlist: &[PeerId], peer: &PeerId) -> bool {
    // simplify: linear scan. Switch to a HashSet if allowlists grow past a
    // handful of peers.
    allowlist.contains(peer)
}

/// In-memory `(peer_id, nonce)` store of accepted hellos.
#[derive(Debug, Default)]
pub(crate) struct NonceStore {
    seen: HashSet<(PeerId, String)>,
}

impl NonceStore {
    /// Creates an empty store.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records `(peer, nonce)` unless it was already accepted; returns `false`
    /// on replay.
    pub(crate) fn check_and_record(&mut self, peer: &PeerId, nonce: &str) -> bool {
        self.seen.insert((*peer, nonce.to_owned()))
    }
}

/// Run the full accept gate for an inbound hello.
///
/// Order: protocol version and claimed-peer binding → allowlist → signature →
/// nonce. A returned [`ConnectError`] identifies the exact rejection reason.
pub(crate) fn gate_hello(
    authenticated_peer: &PeerId,
    public_key: &PublicKey,
    allowlist: &[PeerId],
    nonces: &mut NonceStore,
    hello: &ConnectHello,
) -> Result<(), ConnectError> {
    if hello.protocol_version.get() != PROTOCOL_VERSION {
        return Err(ConnectError::HandshakeFailed {
            reason: format!(
                "unsupported protocol_version {} (expected {PROTOCOL_VERSION})",
                hello.protocol_version
            ),
        });
    }
    if !is_allowlisted(allowlist, authenticated_peer) {
        return Err(ConnectError::NotAllowlisted {
            peer_id: *authenticated_peer,
        });
    }
    verify_hello(public_key, authenticated_peer, hello)?;
    if !nonces.check_and_record(authenticated_peer, hello.nonce.as_str()) {
        return Err(ConnectError::NonceReplay);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Deterministic test vectors: the fixed nonce strings in these tests are
    // reproducible fixtures, not production CSPRNG output (production nonces
    // come from `generate_nonce`).
    use super::*;
    use crate::hello::{generate_nonce, sign_hello};
    use libp2p::identity::Keypair;

    fn manifest(host_id: &str) -> spoke_schemas::connect::connect_hello::HostCapabilityManifest {
        spoke_schemas::connect::connect_hello::HostCapabilityManifest {
            authority: None,
            capabilities: vec!["spoke-baseline".into()],
            extensions: Default::default(),
            host_id: host_id.parse().expect("host id parses"),
            namespaces: Vec::new(),
            roles: vec!["data-store".into()],
            schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
        }
    }

    #[test]
    fn allowlist_accepts_listed_peer() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let hello = sign_hello(
            &keypair,
            &generate_nonce().expect("nonce"),
            &manifest("host-a"),
        )
        .expect("sign hello");
        let mut nonces = NonceStore::new();

        gate_hello(&peer_id, &keypair.public(), &[peer_id], &mut nonces, &hello)
            .expect("allowlisted peer accepted");
    }

    #[test]
    fn allowlist_rejects_unlisted_peer() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let other = Keypair::generate_ed25519().public().to_peer_id();
        let hello = sign_hello(
            &keypair,
            &generate_nonce().expect("nonce"),
            &manifest("host-a"),
        )
        .expect("sign hello");
        let mut nonces = NonceStore::new();

        let err = gate_hello(&peer_id, &keypair.public(), &[other], &mut nonces, &hello)
            .expect_err("unlisted peer");
        assert!(matches!(
            err,
            ConnectError::NotAllowlisted { peer_id: p } if p == peer_id
        ));
    }

    #[test]
    fn empty_allowlist_rejects_everyone() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let hello = sign_hello(
            &keypair,
            &generate_nonce().expect("nonce"),
            &manifest("host-a"),
        )
        .expect("sign hello");
        let mut nonces = NonceStore::new();

        let err = gate_hello(&peer_id, &keypair.public(), &[], &mut nonces, &hello)
            .expect_err("fail-closed");
        assert!(matches!(err, ConnectError::NotAllowlisted { .. }));
    }

    #[test]
    fn nonce_replay_rejected() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let hello =
            sign_hello(&keypair, "replay-nonce-12345", &manifest("host-a")).expect("sign hello");
        let mut nonces = NonceStore::new();

        gate_hello(&peer_id, &keypair.public(), &[peer_id], &mut nonces, &hello)
            .expect("first accept");
        let err = gate_hello(&peer_id, &keypair.public(), &[peer_id], &mut nonces, &hello)
            .expect_err("replay");
        assert!(matches!(err, ConnectError::NonceReplay));
    }

    #[test]
    fn same_nonce_from_different_peers_is_allowed() {
        let keypair_a = Keypair::generate_ed25519();
        let keypair_b = Keypair::generate_ed25519();
        let peer_a = keypair_a.public().to_peer_id();
        let peer_b = keypair_b.public().to_peer_id();
        let nonce = "shared-nonce-12345";
        let hello_a = sign_hello(&keypair_a, nonce, &manifest("host-a")).expect("sign a");
        let hello_b = sign_hello(&keypair_b, nonce, &manifest("host-b")).expect("sign b");
        let mut nonces = NonceStore::new();

        gate_hello(
            &peer_a,
            &keypair_a.public(),
            &[peer_a, peer_b],
            &mut nonces,
            &hello_a,
        )
        .expect("a accepted");
        gate_hello(
            &peer_b,
            &keypair_b.public(),
            &[peer_a, peer_b],
            &mut nonces,
            &hello_b,
        )
        .expect("b accepted with same nonce");
    }

    #[test]
    fn rejected_hello_nonce_is_not_recorded() {
        // A peer whose hello fails the allowlist must not burn its nonce:
        // once allowlisted (same process), the same hello must be accepted.
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let hello =
            sign_hello(&keypair, "retry-nonce-12345", &manifest("host-a")).expect("sign hello");
        let mut nonces = NonceStore::new();

        let err = gate_hello(&peer_id, &keypair.public(), &[], &mut nonces, &hello)
            .expect_err("rejected while unlisted");
        assert!(matches!(err, ConnectError::NotAllowlisted { .. }));

        gate_hello(&peer_id, &keypair.public(), &[peer_id], &mut nonces, &hello)
            .expect("same hello accepted once allowlisted");
    }

    #[test]
    fn gate_rejects_key_not_deriving_the_authenticated_peer() {
        // Defense in depth: the verify key must derive the noise-authenticated
        // peer id, otherwise a remote stack could attest a different identity
        // key for an allowlisted peer (spoof of the hello signature as an
        // identity attestation).
        let signer = Keypair::generate_ed25519();
        let other = Keypair::generate_ed25519();
        let peer_id = signer.public().to_peer_id();
        let hello =
            sign_hello(&signer, "bind-nonce-12345", &manifest("host-a")).expect("sign hello");
        let mut nonces = NonceStore::new();

        let err = gate_hello(&peer_id, &other.public(), &[peer_id], &mut nonces, &hello)
            .expect_err("unbound verify key");
        assert!(matches!(err, ConnectError::HandshakeFailed { .. }));
    }
}
