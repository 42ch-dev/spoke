//! `spoke-history` family helpers: the port result type and the
//! canonical-JSON snapshot comparison (spoke-history contract: two providers
//! returning the same snapshot at the same revision MUST produce identical
//! bytes under RFC 8785 canonical JSON).

use crate::result::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use serde::{Deserialize, Serialize};
use spoke_schemas::RevisionSnapshot;

/// `history.revisions.prune` success value (spoke-history contract
/// §Checkpoints, pruning, and `floor_revision`): a plain object — the
/// schema carries no named `HistoryPruneResult` type.
///
/// `deleted` counts checkpoints deleted by this call; it excludes the live
/// row and does not count derived revisions. `floor_revision` is
/// `newest_deleted_checkpoint_revision + 1` when at least one checkpoint was
/// deleted; `None` (wire `null`) on a no-op, including `keep` >= the
/// available checkpoint count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPruneResult {
    /// Number of checkpoints deleted by this call.
    pub deleted: u64,
    /// Newest deleted checkpoint revision + 1, or `None` on a no-op. The
    /// current retained floor for subsequent replay.
    pub floor_revision: Option<u64>,
}

/// Whether two [`RevisionSnapshot`]s are byte-identical under RFC 8785
/// canonical JSON — the contract's provider-equality rule.
///
/// Rejects with `INVALID_INPUT` when either snapshot cannot be serialized
/// to canonical bytes.
pub fn revision_snapshots_equal_canonical(
    left: &RevisionSnapshot,
    right: &RevisionSnapshot,
) -> SpokeResult<bool> {
    let left = match serde_jcs::to_vec(left) {
        Ok(bytes) => bytes,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("JCS canonicalization failed: {error}"),
                None,
            );
        }
    };
    let right = match serde_jcs::to_vec(right) {
        Ok(bytes) => bytes,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("JCS canonicalization failed: {error}"),
                None,
            );
        }
    };
    spoke_ok(left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> RevisionSnapshot {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "entity_kind": "knowledge_entry",
            "entity_id": "ke_mira_harbor",
            "revision": 3,
            "snapshot": {
                "schema_version": 1,
                "entry_id": "ke_mira_harbor",
                "entry_type": "character",
                "canonical_name": "Mira",
                "status": "confirmed",
                "body": {},
                "extensions": {}
            },
            "extensions": {}
        }))
        .expect("snapshot fixture decodes")
    }

    #[test]
    fn identical_snapshots_compare_equal() {
        match revision_snapshots_equal_canonical(&snapshot(), &snapshot()) {
            SpokeResult::Ok(equal) => assert!(equal),
            SpokeResult::Reject(reject) => panic!("fixture must canonicalize: {reject:?}"),
        }
    }

    #[test]
    fn different_revisions_compare_unequal() {
        let mut other = snapshot();
        other.revision = 4;
        match revision_snapshots_equal_canonical(&snapshot(), &other) {
            SpokeResult::Ok(equal) => assert!(!equal),
            SpokeResult::Reject(reject) => panic!("fixture must canonicalize: {reject:?}"),
        }
    }

    #[test]
    fn member_order_does_not_change_the_canonical_bytes() {
        // The same document written with members in a different order has
        // the same RFC 8785 bytes — this is the property the helper exists
        // to assert.
        let reordered: RevisionSnapshot = serde_json::from_value(serde_json::json!({
            "extensions": {},
            "snapshot": {
                "extensions": {},
                "body": {},
                "status": "confirmed",
                "canonical_name": "Mira",
                "entry_type": "character",
                "entry_id": "ke_mira_harbor",
                "schema_version": 1
            },
            "revision": 3,
            "entity_id": "ke_mira_harbor",
            "entity_kind": "knowledge_entry",
            "schema_version": 1
        }))
        .expect("reordered fixture decodes");
        match revision_snapshots_equal_canonical(&snapshot(), &reordered) {
            SpokeResult::Ok(equal) => assert!(equal),
            SpokeResult::Reject(reject) => panic!("fixture must canonicalize: {reject:?}"),
        }
    }

    #[test]
    fn prune_result_wire_shape() {
        let value = serde_json::to_value(HistoryPruneResult {
            deleted: 1,
            floor_revision: Some(2),
        })
        .expect("serialize");
        assert_eq!(value, serde_json::json!({"deleted": 1, "floor_revision": 2}));
        let value = serde_json::to_value(HistoryPruneResult {
            deleted: 0,
            floor_revision: None,
        })
        .expect("serialize");
        assert_eq!(value, serde_json::json!({"deleted": 0, "floor_revision": null}));
    }
}
