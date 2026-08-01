//! `noise-peerid` allowlist check (fail-closed).
//!
//! Normative rules (`.mstar/specs/spoke-connect.md` §Auth model): the trust
//! root is a deployment-configured peer id allowlist. An empty allowlist
//! rejects every remote peer.

/// Whether `peer_id` is on the allowlist. An empty allowlist rejects every
/// peer (fail-closed).
#[must_use]
pub fn is_allowlisted(allowlist: &[String], peer_id: &str) -> bool {
    // simplify: linear scan. Switch to a HashSet if allowlists grow past a
    // handful of peers.
    allowlist.iter().any(|entry| entry == peer_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_peer_is_accepted() {
        let allowlist = vec!["peer-a".to_owned(), "peer-b".to_owned()];
        assert!(is_allowlisted(&allowlist, "peer-a"));
        assert!(is_allowlisted(&allowlist, "peer-b"));
    }

    #[test]
    fn unlisted_peer_is_rejected() {
        let allowlist = vec!["peer-a".to_owned()];
        assert!(!is_allowlisted(&allowlist, "peer-c"));
    }

    #[test]
    fn empty_allowlist_rejects_everyone() {
        let allowlist: Vec<String> = Vec::new();
        assert!(!is_allowlisted(&allowlist, "peer-a"), "fail-closed");
    }
}
