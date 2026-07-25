//! Finding status transition helpers.

use crate::result::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use chrono::Utc;
use serde_json::{Map, Value};
use spoke_schemas::Finding;
use std::collections::{HashMap, HashSet};

const CORE_FINDING_STATUSES: &[&str] = &["open", "resolved", "dismissed"];

fn allowed_transitions() -> HashMap<&'static str, HashSet<&'static str>> {
    HashMap::from([
        (
            "open",
            HashSet::from(["resolved", "dismissed", "open"]),
        ),
        ("resolved", HashSet::from(["open", "resolved"])),
        ("dismissed", HashSet::from(["open", "dismissed"])),
    ])
}

fn is_core_finding_status(status: &str) -> bool {
    CORE_FINDING_STATUSES.contains(&status)
}

/// Returns whether a Finding status transition is allowed by the cross-product table.
#[must_use]
pub fn is_valid_finding_status_transition(from: &str, to: &str) -> bool {
    if !is_core_finding_status(from) || !is_core_finding_status(to) {
        return false;
    }

    allowed_transitions()
        .get(from)
        .is_some_and(|targets| targets.contains(to))
}

/// Apply a Finding status transition; returns updated status and `updated_at` on success.
pub fn transition_finding_status(
    finding: &Finding,
    to: &str,
) -> SpokeResult<Finding> {
    if !is_core_finding_status(to) {
        let mut details = Map::new();
        details.insert("status".into(), Value::String(to.to_owned()));
        return spoke_reject(
            SpokeRejectCode::InvalidStatus,
            format!("Invalid finding status: {to}"),
            Some(details),
        );
    }

    if !is_core_finding_status(&finding.status) {
        let mut details = Map::new();
        details.insert("status".into(), Value::String(finding.status.clone()));
        return spoke_reject(
            SpokeRejectCode::InvalidStatus,
            format!("Invalid current finding status: {}", finding.status),
            Some(details),
        );
    }

    if !is_valid_finding_status_transition(&finding.status, to) {
        let mut details = Map::new();
        details.insert("from".into(), Value::String(finding.status.clone()));
        details.insert("to".into(), Value::String(to.to_owned()));
        return spoke_reject(
            SpokeRejectCode::InvalidStatusTransition,
            format!(
                "Disallowed finding status transition: {} -> {to}",
                finding.status
            ),
            Some(details),
        );
    }

    let mut updated = finding.clone();
    updated.status = to.to_owned();
    updated.updated_at = Some(Utc::now());
    spoke_ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn make_finding(status: &str) -> Finding {
        Finding {
            created_at: None,
            description: "Description".into(),
            extensions: HashMap::new(),
            finding_id: "fnd_1".into(),
            kind: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            severity: "warning".into(),
            source_anchor: None,
            status: status.into(),
            suggested_fix: None,
            target_entry_id: None,
            text_position: Default::default(),
            title: "Title".into(),
            updated_at: None,
        }
    }

    #[test]
    fn allows_documented_transitions() {
        for (from, to) in [
            ("open", "resolved"),
            ("open", "dismissed"),
            ("resolved", "open"),
            ("dismissed", "open"),
            ("open", "open"),
            ("resolved", "resolved"),
            ("dismissed", "dismissed"),
        ] {
            assert!(is_valid_finding_status_transition(from, to));
        }
    }

    #[test]
    fn rejects_disallowed_transitions() {
        for (from, to) in [
            ("resolved", "dismissed"),
            ("dismissed", "resolved"),
            ("open", "invalid"),
            ("invalid", "open"),
        ] {
            assert!(!is_valid_finding_status_transition(from, to));
        }
    }

    #[test]
    fn accepts_allowed_transition_and_sets_updated_at() {
        let finding = make_finding("open");
        let result = transition_finding_status(&finding, "resolved");

        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.status, "resolved");
            assert!(value.updated_at.is_some());
            assert_eq!(finding.status, "open");
        }
    }

    #[test]
    fn accepts_no_op_same_status() {
        let finding = make_finding("dismissed");
        let result = transition_finding_status(&finding, "dismissed");

        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.status, "dismissed");
        }
    }

    #[test]
    fn rejects_invalid_target_status() {
        let result = transition_finding_status(&make_finding("open"), "bogus");

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidStatus);
            assert_eq!(reject.message, "Invalid finding status: bogus");
            assert_eq!(
                reject.details,
                Some(Map::from_iter([(
                    "status".into(),
                    Value::String("bogus".into())
                )]))
            );
        }
    }

    #[test]
    fn rejects_disallowed_transition_with_details() {
        let result = transition_finding_status(&make_finding("resolved"), "dismissed");

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidStatusTransition);
            assert_eq!(
                reject.details,
                Some(Map::from_iter([
                    ("from".into(), json!("resolved")),
                    ("to".into(), json!("dismissed")),
                ]))
            );
        }
    }
}
