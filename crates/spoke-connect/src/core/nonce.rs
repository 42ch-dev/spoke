//! Single-use `(peer_id, nonce)` replay store.
//!
//! Normative rules (`.mstar/specs/spoke-connect.md` §Nonce / replay
//! protection): nonce uniqueness is scoped per **sender** `peer_id`; a
//! receiver MUST reject a hello whose `(peer_id, nonce)` pair was already
//! accepted. An in-memory set for the life of the process is sufficient for
//! protocol v1 (products MAY persist nonces with TTL).
//!
//! The store only records **accepted** hellos — the caller must not call
//! `check_and_record` for a hello that failed an earlier gate, so a rejected
//! hello stays retry-safe.

use std::collections::HashSet;

/// In-memory `(peer_id, nonce)` store of accepted hellos.
#[derive(Debug, Default)]
pub struct NonceStore {
    seen: HashSet<(String, String)>,
}

impl NonceStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `(peer_id, nonce)` unless it was already accepted; returns
    /// `false` on replay. Call only after the hello passed every earlier
    /// gate (allowlist, signature) so a rejected hello is not burned.
    pub fn check_and_record(&mut self, peer_id: &str, nonce: &str) -> bool {
        self.seen.insert((peer_id.to_owned(), nonce.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_single_use_per_peer() {
        let mut store = NonceStore::new();
        assert!(store.check_and_record("peer-a", "nonce-1"));
        assert!(
            !store.check_and_record("peer-a", "nonce-1"),
            "replay rejected"
        );
        // A different nonce from the same peer is fresh.
        assert!(store.check_and_record("peer-a", "nonce-2"));
    }

    #[test]
    fn same_nonce_from_different_peers_is_allowed() {
        let mut store = NonceStore::new();
        assert!(store.check_and_record("peer-a", "shared-nonce"));
        assert!(
            store.check_and_record("peer-b", "shared-nonce"),
            "nonce scoping is per sender peer_id"
        );
    }

    #[test]
    fn fresh_store_accepts_everything() {
        let mut store = NonceStore::new();
        assert!(store.check_and_record("peer-a", "n-1"));
        assert!(store.check_and_record("peer-b", "n-1"));
        assert!(store.check_and_record("peer-b", "n-2"));
    }
}
