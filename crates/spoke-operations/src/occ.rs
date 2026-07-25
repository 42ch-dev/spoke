//! Optimistic concurrency revision compare helpers.

use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map};

fn is_valid_revision(value: f64) -> bool {
    value.is_finite() && value >= 0.0 && value.fract() == 0.0
}

/// Compare caller-supplied revisions before persist; library performs no storage I/O.
pub fn assert_revision_match(expected_revision: f64, actual_revision: f64) -> SpokeResult<()> {
    if !is_valid_revision(expected_revision) || !is_valid_revision(actual_revision) {
        let mut details = Map::new();
        details.insert("expectedRevision".into(), json!(expected_revision));
        details.insert("actualRevision".into(), json!(actual_revision));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "Revisions must be non-negative integers",
            Some(details),
        );
    }

    let expected = expected_revision as u64;
    let actual = actual_revision as u64;

    if expected == actual {
        return spoke_ok_unit();
    }

    if actual > expected {
        let mut details = Map::new();
        details.insert("expectedRevision".into(), json!(expected_revision));
        details.insert("actualRevision".into(), json!(actual_revision));
        return spoke_reject(
            SpokeRejectCode::StoredRevisionStale,
            format!("Stored revision {actual} is ahead of expected {expected}"),
            Some(details),
        );
    }

    let mut details = Map::new();
    details.insert("expectedRevision".into(), json!(expected_revision));
    details.insert("actualRevision".into(), json!(actual_revision));
    spoke_reject(
        SpokeRejectCode::RevisionConflict,
        format!("Expected revision {expected} is ahead of actual {actual}"),
        Some(details),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;

    #[test]
    fn accepts_equal_non_negative_integer_revisions() {
        assert!(assert_revision_match(0.0, 0.0).is_ok());
        assert!(assert_revision_match(3.0, 3.0).is_ok());
    }

    #[test]
    fn rejects_when_actual_revision_is_greater_than_expected() {
        let result = assert_revision_match(2.0, 5.0);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        }
    }

    #[test]
    fn rejects_when_actual_revision_is_less_than_expected() {
        let result = assert_revision_match(5.0, 2.0);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RevisionConflict);
        }
    }

    #[test]
    fn rejects_invalid_input() {
        for (expected, actual) in [
            (-1.0, 0.0),
            (0.0, -1.0),
            (1.5, 1.0),
            (1.0, 1.5),
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
        ] {
            let result = assert_revision_match(expected, actual);

            assert!(result.is_reject(), "expected reject for ({expected}, {actual})");
            if let SpokeResult::Reject(reject) = result {
                assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            }
        }
    }
}
