//! MindState shape validator — no execution engines.
//!
//! Mirrors `validateMindState` (TypeScript): required fields, closed envelope,
//! snapshot / deltas types. Wire shape only — mental transition and ToM
//! inference stay product-owned.

use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Map, Value};

const MIND_STATE_KEYS: [&str; 12] = [
    "schema_version",
    "mind_state_id",
    "holder_entry_id",
    "canonical_name",
    "occurred_at",
    "sort_key",
    "snapshot",
    "deltas",
    "source_anchor",
    "created_at",
    "updated_at",
    "extensions",
];

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_plain_object(value: &Value) -> bool {
    value.is_object() && !value.is_null()
}

fn is_parseable_date_time(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    DateTime::parse_from_rfc3339(trimmed).is_ok()
        || trimmed.parse::<DateTime<Utc>>().is_ok()
        || NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
        || NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").is_ok()
}

fn reject(field: &str, code: SpokeRejectCode, message: &str) -> SpokeResult<()> {
    let mut details = Map::new();
    details.insert("field".into(), json!(field));
    spoke_reject(code, message, Some(details))
}

/// Wire-shape gate for MindState (optional capability l5-mind).
///
/// Checks required fields (`schema_version`, `mind_state_id`,
/// `holder_entry_id`, `extensions`), the closed envelope (no unknown
/// properties), and `snapshot` / `deltas` types when present.
pub fn validate_mind_state(value: &Value) -> SpokeResult<()> {
    let Some(state) = value.as_object() else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "MindState must be a non-null plain object",
            None,
        );
    };

    for key in state.keys() {
        if !MIND_STATE_KEYS.contains(&key.as_str()) {
            return reject(
                key,
                SpokeRejectCode::InvalidInput,
                "MindState has unknown property",
            );
        }
    }

    let schema_version = state.get("schema_version").unwrap_or(&Value::Null);
    let version_ok = match schema_version.as_f64() {
        Some(version) => version >= 1.0 && version.fract() == 0.0,
        None => false,
    };
    if !version_ok {
        return reject(
            "schema_version",
            SpokeRejectCode::InvalidInput,
            "MindState schema_version must be an integer >= 1",
        );
    }

    let mind_state_id = state
        .get("mind_state_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(mind_state_id) {
        return reject(
            "mind_state_id",
            SpokeRejectCode::MissingRequiredField,
            "MindState mind_state_id must be a non-empty string",
        );
    }

    let holder_entry_id = state
        .get("holder_entry_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(holder_entry_id) {
        return reject(
            "holder_entry_id",
            SpokeRejectCode::MissingRequiredField,
            "MindState holder_entry_id must be a non-empty string",
        );
    }

    if let Some(extensions) = state.get("extensions") {
        if !is_plain_object(extensions) {
            return reject(
                "extensions",
                SpokeRejectCode::InvalidInput,
                "MindState extensions must be an object",
            );
        }
    } else {
        return reject(
            "extensions",
            SpokeRejectCode::MissingRequiredField,
            "MindState extensions is required",
        );
    }

    if let Some(canonical_name) = state.get("canonical_name") {
        let canonical_name = canonical_name.as_str().unwrap_or("");
        if !is_non_empty_trimmed_string(canonical_name) {
            return reject(
                "canonical_name",
                SpokeRejectCode::InvalidInput,
                "MindState canonical_name must be a non-empty string when present",
            );
        }
    }

    if let Some(sort_key) = state.get("sort_key") {
        if !sort_key.is_string() {
            return reject(
                "sort_key",
                SpokeRejectCode::InvalidInput,
                "MindState sort_key must be a string when present",
            );
        }
    }

    for field in ["occurred_at", "created_at", "updated_at"] {
        if let Some(value) = state.get(field) {
            if !value.is_string() || !is_parseable_date_time(value.as_str().unwrap_or("")) {
                return reject(
                    field,
                    SpokeRejectCode::InvalidInput,
                    "MindState timestamp fields must be valid date-time strings when present",
                );
            }
        }
    }

    if let Some(snapshot) = state.get("snapshot") {
        if !is_plain_object(snapshot) {
            return reject(
                "snapshot",
                SpokeRejectCode::InvalidInput,
                "MindState snapshot must be an object when present",
            );
        }
    }

    if let Some(source_anchor) = state.get("source_anchor") {
        if !is_plain_object(source_anchor) {
            return reject(
                "source_anchor",
                SpokeRejectCode::InvalidInput,
                "MindState source_anchor must be an object when present",
            );
        }
    }

    if let Some(deltas) = state.get("deltas") {
        let Some(items) = deltas.as_array() else {
            return reject(
                "deltas",
                SpokeRejectCode::InvalidInput,
                "MindState deltas must be an array when present",
            );
        };
        for (index, delta) in items.iter().enumerate() {
            if !is_plain_object(delta) {
                return reject(
                    "deltas",
                    SpokeRejectCode::InvalidInput,
                    "MindDelta must be a non-null plain object",
                );
            }
            let path = delta.get("path").and_then(Value::as_str).unwrap_or("");
            if !is_non_empty_trimmed_string(path) {
                let mut details = Map::new();
                details.insert("field".into(), json!("deltas"));
                details.insert("index".into(), json!(index));
                return spoke_reject(
                    SpokeRejectCode::MissingRequiredField,
                    "MindDelta path must be a non-empty string",
                    Some(details),
                );
            }
        }
    }

    spoke_ok_unit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;
    use std::path::Path;

    fn valid_mind_state() -> Value {
        json!({
            "schema_version": 1,
            "mind_state_id": "mind_tw_bo_pre_transfer",
            "holder_entry_id": "kb_tw_bo",
            "canonical_name": "Bo before the hidden transfer",
            "occurred_at": "2026-07-23T09:00:00Z",
            "sort_key": "fb-001",
            "snapshot": {
                "beliefs": { "ref": "kb_tw_bo", "count": 2 },
                "attention": { "target": "kb_tw_room", "modality": "visual" },
                "emotions": [{ "emotion": "calm", "intensity": 0.4 }]
            },
            "deltas": [],
            "created_at": "2026-07-23T09:00:00Z",
            "updated_at": "2026-07-23T09:00:00Z",
            "extensions": {
                "toy": {
                    "story": "false-belief-box-basket",
                    "phase": "pre-transfer"
                }
            }
        })
    }

    #[test]
    fn validate_mind_state_accepts_valid_record() {
        assert!(validate_mind_state(&valid_mind_state()).is_ok());
    }

    #[test]
    fn validate_mind_state_accepts_minimal_required_only_record() {
        let state = json!({
            "schema_version": 1,
            "mind_state_id": "mind_tw_min",
            "holder_entry_id": "kb_tw_bo",
            "extensions": {}
        });

        assert!(validate_mind_state(&state).is_ok());
    }

    #[test]
    fn validate_mind_state_rejects_missing_holder_entry_id() {
        let mut state = valid_mind_state();
        state.as_object_mut().unwrap().remove("holder_entry_id");

        let result = validate_mind_state(&state);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }

    #[test]
    fn validate_mind_state_rejects_missing_mind_state_id() {
        let mut state = valid_mind_state();
        state.as_object_mut().unwrap().remove("mind_state_id");

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_empty_holder_entry_id() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("holder_entry_id".into(), json!("  "));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_extra_property() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("mind_engine".into(), json!({}));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_delta_without_path() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("deltas".into(), json!([{ "previous": 1, "next": 2 }]));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_accepts_delta_with_path_only() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("deltas".into(), json!([{ "path": "beliefs" }]));

        assert!(validate_mind_state(&state).is_ok());
    }

    #[test]
    fn validate_mind_state_rejects_non_array_deltas() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("deltas".into(), json!({ "path": "beliefs" }));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_non_object_snapshot() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("snapshot".into(), json!([]));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_non_object_extensions() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("extensions".into(), json!([]));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_invalid_occurred_at() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("occurred_at".into(), json!("not-a-timestamp"));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_timestamp_parity_with_typescript() {
        // Shared cross-language parity contract (TS validateMindState ↔ this
        // validator): the same fixture is asserted by both suites, so the
        // timestamp accept set can never drift between languages again.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/timestamp-parity-cases.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let cases: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));

        for value in cases["accept"].as_array().expect("accept list") {
            let mut state = valid_mind_state();
            state
                .as_object_mut()
                .unwrap()
                .insert("occurred_at".into(), value.clone());
            assert!(
                validate_mind_state(&state).is_ok(),
                "expected {} to be accepted",
                value
            );
        }
        for value in cases["reject"].as_array().expect("reject list") {
            let mut state = valid_mind_state();
            state
                .as_object_mut()
                .unwrap()
                .insert("occurred_at".into(), value.clone());
            assert!(
                validate_mind_state(&state).is_reject(),
                "expected {} to be rejected",
                value
            );
        }
    }

    #[test]
    fn validate_mind_state_rejects_invalid_schema_version() {
        let mut state = valid_mind_state();
        state
            .as_object_mut()
            .unwrap()
            .insert("schema_version".into(), json!(0));

        assert!(validate_mind_state(&state).is_reject());
    }

    #[test]
    fn validate_mind_state_rejects_null_and_non_object_state() {
        assert!(validate_mind_state(&Value::Null).is_reject());
        assert!(validate_mind_state(&json!([])).is_reject());
        assert!(validate_mind_state(&json!("mind")).is_reject());
    }
}
