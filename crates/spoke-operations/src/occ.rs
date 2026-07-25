//! Optimistic concurrency revision compare helpers.

use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map};

/// Compare caller-supplied revisions before persist; library performs no storage I/O.
pub fn assert_revision_match(expected_revision: u64, actual_revision: u64) -> SpokeResult<()> {
    if expected_revision == actual_revision {
        return spoke_ok_unit();
    }

    if actual_revision > expected_revision {
        let mut details = Map::new();
        details.insert("expectedRevision".into(), json!(expected_revision));
        details.insert("actualRevision".into(), json!(actual_revision));
        return spoke_reject(
            SpokeRejectCode::StoredRevisionStale,
            format!(
                "Stored revision {actual_revision} is ahead of expected {expected_revision}"
            ),
            Some(details),
        );
    }

    let mut details = Map::new();
    details.insert("expectedRevision".into(), json!(expected_revision));
    details.insert("actualRevision".into(), json!(actual_revision));
    spoke_reject(
        SpokeRejectCode::RevisionConflict,
        format!(
            "Expected revision {expected_revision} is ahead of actual {actual_revision}"
        ),
        Some(details),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;

    #[test]
    fn accepts_equal_non_negative_integer_revisions() {
        assert!(assert_revision_match(0, 0).is_ok());
        assert!(assert_revision_match(3, 3).is_ok());
    }

    #[test]
    fn accepts_revisions_above_js_safe_integer_range() {
        let large = 9_007_199_254_740_993_u64;
        assert!(assert_revision_match(large, large).is_ok());
    }

    #[test]
    fn rejects_when_actual_revision_is_greater_than_expected() {
        let result = assert_revision_match(2, 5);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        }
    }

    #[test]
    fn rejects_when_actual_revision_is_less_than_expected() {
        let result = assert_revision_match(5, 2);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RevisionConflict);
        }
    }

    #[test]
    fn detects_stale_revision_above_js_safe_integer_range() {
        let expected = 9_007_199_254_740_992_u64;
        let actual = 9_007_199_254_740_993_u64;
        let result = assert_revision_match(expected, actual);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        }
    }
}
