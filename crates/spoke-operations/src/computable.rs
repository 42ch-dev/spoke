//! Computable shape validators — no execution engines.

use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map, Value};
use spoke_schemas::{ComputableLogEntry, ComputeRequest, ProjectRequest};

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_plain_object(value: &Value) -> bool {
    value.is_object() && !value.is_null()
}

/// Shape gate for body.state / body.computable and op ComputableFieldMap payloads.
pub fn validate_computable_field_map(value: &Value) -> SpokeResult<()> {
    if !is_plain_object(value) {
        let mut details = Map::new();
        details.insert("field".into(), json!("computable_field_map"));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "ComputableFieldMap must be a non-null plain object",
            Some(details),
        );
    }

    spoke_ok_unit()
}

/// Shape gate for TimelineEvent.computable_logs[] items.
pub fn validate_computable_log_entry(entry: &ComputableLogEntry) -> SpokeResult<()> {
    if !is_non_empty_trimmed_string(&entry.entry_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("entry_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ComputableLogEntry entry_id must be a non-empty string",
            Some(details),
        );
    }

    for (index, change) in entry.changes.iter().enumerate() {
        if change.path.trim().is_empty() {
            let mut details = Map::new();
            details.insert("field".into(), json!("changes"));
            details.insert("index".into(), json!(index));
            return spoke_reject(
                SpokeRejectCode::MissingRequiredField,
                "ComputableLogChange path must be a non-empty string",
                Some(details),
            );
        }
    }

    if let Some(session_id) = &entry.session_id {
        if !is_non_empty_trimmed_string(session_id) {
            let mut details = Map::new();
            details.insert("field".into(), json!("session_id"));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputableLogEntry session_id must be a non-empty string when present",
                Some(details),
            );
        }
    }

    spoke_ok_unit()
}

fn validate_extension_map_value(extensions: &Value, field: &str) -> SpokeResult<()> {
    if !is_plain_object(extensions) {
        let mut details = Map::new();
        details.insert("field".into(), json!(field));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("{field} must be an object"),
            Some(details),
        );
    }

    spoke_ok_unit()
}

/// Required-field gate for project op request wire JSON.
pub fn validate_project_request_wire(value: &Value) -> SpokeResult<()> {
    let Some(request) = value.as_object() else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "ProjectRequest must be an object",
            None,
        );
    };

    let session_id = request
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(session_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("session_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ProjectRequest session_id must be a non-empty string",
            Some(details),
        );
    }

    let entry_id = request.get("entry_id").and_then(Value::as_str).unwrap_or("");
    if !is_non_empty_trimmed_string(entry_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("entry_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ProjectRequest entry_id must be a non-empty string",
            Some(details),
        );
    }

    let state = request.get("state").unwrap_or(&Value::Null);
    if let SpokeResult::Reject(reject) = validate_computable_field_map(state) {
        return SpokeResult::Reject(reject);
    }

    if let Some(extensions) = request.get("extensions") {
        if let SpokeResult::Reject(reject) = validate_extension_map_value(extensions, "extensions") {
            return SpokeResult::Reject(reject);
        }
    }

    spoke_ok_unit()
}

/// Required-field gate for project op request wire shape.
pub fn validate_project_request(request: &ProjectRequest) -> SpokeResult<()> {
    if !is_non_empty_trimmed_string(&request.session_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("session_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ProjectRequest session_id must be a non-empty string",
            Some(details),
        );
    }

    if !is_non_empty_trimmed_string(&request.entry_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("entry_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ProjectRequest entry_id must be a non-empty string",
            Some(details),
        );
    }

    if let SpokeResult::Reject(reject) =
        validate_computable_field_map(&Value::Object(request.state.clone()))
    {
        return SpokeResult::Reject(reject);
    }

    if !request.extensions.is_empty() {
        if let SpokeResult::Reject(reject) =
            validate_extension_map_value(&json!(request.extensions), "extensions")
        {
            return SpokeResult::Reject(reject);
        }
    }

    spoke_ok_unit()
}

/// Required-field gate for compute op request wire shape.
pub fn validate_compute_request_wire(value: &Value) -> SpokeResult<()> {
    let Some(request) = value.as_object() else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "ComputeRequest must be an object",
            None,
        );
    };

    let session_id = request
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(session_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("session_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ComputeRequest session_id must be a non-empty string",
            Some(details),
        );
    }

    let entry_id = request.get("entry_id").and_then(Value::as_str).unwrap_or("");
    if !is_non_empty_trimmed_string(entry_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("entry_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ComputeRequest entry_id must be a non-empty string",
            Some(details),
        );
    }

    let computable = request.get("computable").unwrap_or(&Value::Null);
    if let SpokeResult::Reject(reject) = validate_computable_field_map(computable) {
        return SpokeResult::Reject(reject);
    }

    if let Some(settle) = request.get("settle") {
        if !settle.is_boolean() {
            let mut details = Map::new();
            details.insert("field".into(), json!("settle"));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputeRequest settle must be a boolean when present",
                Some(details),
            );
        }
    }

    if let Some(extensions) = request.get("extensions") {
        if let SpokeResult::Reject(reject) = validate_extension_map_value(extensions, "extensions") {
            return SpokeResult::Reject(reject);
        }
    }

    spoke_ok_unit()
}

/// Required-field gate for compute op request wire shape.
pub fn validate_compute_request(request: &ComputeRequest) -> SpokeResult<()> {
    if !is_non_empty_trimmed_string(&request.session_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("session_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ComputeRequest session_id must be a non-empty string",
            Some(details),
        );
    }

    if !is_non_empty_trimmed_string(&request.entry_id) {
        let mut details = Map::new();
        details.insert("field".into(), json!("entry_id"));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "ComputeRequest entry_id must be a non-empty string",
            Some(details),
        );
    }

    if let SpokeResult::Reject(reject) =
        validate_computable_field_map(&Value::Object(request.computable.clone()))
    {
        return SpokeResult::Reject(reject);
    }

    if !request.extensions.is_empty() {
        if let SpokeResult::Reject(reject) =
            validate_extension_map_value(&json!(request.extensions), "extensions")
        {
            return SpokeResult::Reject(reject);
        }
    }

    spoke_ok_unit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;
    use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
    use serde_json::json;
    use spoke_schemas::ComputableLogEntryChangesItem;
    use std::collections::HashMap;

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

    fn validate_computable_log_change(change: &Value, index: usize) -> SpokeResult<()> {
        if !is_plain_object(change) {
            let mut details = Map::new();
            details.insert("field".into(), json!("changes"));
            details.insert("index".into(), json!(index));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputableLogChange must be a non-null plain object",
                Some(details),
            );
        }

        let path = change.get("path").and_then(Value::as_str).unwrap_or("");
        if !is_non_empty_trimmed_string(path) {
            let mut details = Map::new();
            details.insert("field".into(), json!("changes"));
            details.insert("index".into(), json!(index));
            return spoke_reject(
                SpokeRejectCode::MissingRequiredField,
                "ComputableLogChange path must be a non-empty string",
                Some(details),
            );
        }

        spoke_ok_unit()
    }

    fn validate_computable_log_entry_wire(value: &Value) -> SpokeResult<()> {
        let Some(entry) = value.as_object() else {
            let mut details = Map::new();
            details.insert("field".into(), json!("logged_at"));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputableLogEntry logged_at must be a valid date-time string",
                Some(details),
            );
        };

        let logged_at = entry.get("logged_at").and_then(Value::as_str).unwrap_or("");
        if !is_non_empty_trimmed_string(logged_at) || !is_parseable_date_time(logged_at) {
            let mut details = Map::new();
            details.insert("field".into(), json!("logged_at"));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputableLogEntry logged_at must be a valid date-time string",
                Some(details),
            );
        }

        let entry_id = entry.get("entry_id").and_then(Value::as_str).unwrap_or("");
        if !is_non_empty_trimmed_string(entry_id) {
            let mut details = Map::new();
            details.insert("field".into(), json!("entry_id"));
            return spoke_reject(
                SpokeRejectCode::MissingRequiredField,
                "ComputableLogEntry entry_id must be a non-empty string",
                Some(details),
            );
        }

        let Some(changes) = entry.get("changes") else {
            let mut details = Map::new();
            details.insert("field".into(), json!("changes"));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputableLogEntry changes must be an array",
                Some(details),
            );
        };

        if !changes.is_array() {
            let mut details = Map::new();
            details.insert("field".into(), json!("changes"));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "ComputableLogEntry changes must be an array",
                Some(details),
            );
        }

        for (index, change) in changes.as_array().unwrap().iter().enumerate() {
            if let SpokeResult::Reject(reject) = validate_computable_log_change(change, index) {
                return SpokeResult::Reject(reject);
            }
        }

        if let Some(session_id) = entry.get("session_id") {
            if session_id.is_null() {
                return spoke_ok_unit();
            }
            let session_id = session_id.as_str().unwrap_or("");
            if !is_non_empty_trimmed_string(session_id) {
                let mut details = Map::new();
                details.insert("field".into(), json!("session_id"));
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    "ComputableLogEntry session_id must be a non-empty string when present",
                    Some(details),
                );
            }
        }

        spoke_ok_unit()
    }

    #[test]
    fn validate_computable_field_map_accepts_empty_plain_object() {
        assert!(validate_computable_field_map(&json!({})).is_ok());
    }

    #[test]
    fn validate_computable_field_map_accepts_domain_values() {
        assert!(validate_computable_field_map(&json!({
            "tide_level": 2.4,
            "cargo_tons": 38
        }))
        .is_ok());
    }

    #[test]
    fn validate_computable_field_map_rejects_null() {
        let result = validate_computable_field_map(&Value::Null);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn validate_computable_field_map_rejects_arrays() {
        let result = validate_computable_field_map(&json!([]));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    fn valid_log_entry() -> ComputableLogEntry {
        ComputableLogEntry {
            changes: vec![ComputableLogEntryChangesItem {
                next: Some(json!(2.4)),
                path: "tide_level".into(),
                previous: Some(json!(2.1)),
            }],
            entry_id: "kb_tw_harbor".into(),
            logged_at: Utc.with_ymd_and_hms(2026, 7, 23, 5, 50, 0).unwrap(),
            message: Some("Tide updated.".into()),
            session_id: Some("sess_tw_dawn_arrival".into()),
        }
    }

    #[test]
    fn validate_computable_log_entry_accepts_valid_entry() {
        assert!(validate_computable_log_entry(&valid_log_entry()).is_ok());
    }

    #[test]
    fn validate_computable_log_entry_accepts_datetime_without_timezone_suffix() {
        let result = validate_computable_log_entry_wire(&json!({
            "logged_at": "2026-07-23T05:50:00",
            "entry_id": "kb_tw_harbor",
            "changes": [{ "path": "tide_level", "previous": 2.1, "next": 2.4 }],
            "session_id": "sess_tw_dawn_arrival",
            "message": "Tide updated."
        }));

        assert!(result.is_ok());
    }

    #[test]
    fn validate_computable_log_entry_rejects_invalid_logged_at() {
        let result = validate_computable_log_entry_wire(&json!({
            "logged_at": "not-a-timestamp",
            "entry_id": "kb_tw_harbor",
            "changes": [{ "path": "tide_level" }],
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn validate_computable_log_entry_rejects_empty_entry_id() {
        let mut entry = valid_log_entry();
        entry.entry_id = "   ".into();

        let result = validate_computable_log_entry(&entry);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }

    #[test]
    fn validate_computable_log_entry_rejects_change_without_path() {
        let mut entry = valid_log_entry();
        entry.changes = vec![ComputableLogEntryChangesItem {
            next: None,
            path: String::new(),
            previous: None,
        }];

        let result = validate_computable_log_entry(&entry);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }

    #[test]
    fn validate_computable_log_entry_rejects_null_change_without_panicking() {
        let result = validate_computable_log_entry_wire(&json!({
            "logged_at": "2026-07-23T05:50:00Z",
            "entry_id": "kb_tw_harbor",
            "changes": [null],
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn validate_computable_log_entry_rejects_undefined_change_without_panicking() {
        let mut changes = serde_json::to_value(valid_log_entry())
            .unwrap()
            .as_object()
            .unwrap()["changes"]
            .as_array()
            .unwrap()
            .clone();
        changes.push(Value::Null);

        let result = validate_computable_log_entry_wire(&json!({
            "logged_at": "2026-07-23T05:50:00Z",
            "entry_id": "kb_tw_harbor",
            "changes": changes,
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    fn valid_project_request() -> ProjectRequest {
        ProjectRequest {
            entry_id: "kb_tw_harbor".into(),
            extensions: HashMap::new(),
            session_id: "sess_tw_dawn_arrival".into(),
            state: Map::from_iter([
                ("tide_level".into(), json!(2.1)),
                ("cargo_tons".into(), json!(40)),
            ]),
        }
    }

    #[test]
    fn validate_project_request_accepts_valid_request() {
        assert!(validate_project_request(&valid_project_request()).is_ok());
    }

    #[test]
    fn validate_project_request_rejects_missing_session_id() {
        let mut request = valid_project_request();
        request.session_id = String::new();

        let result = validate_project_request(&request);

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }

    #[test]
    fn validate_project_request_rejects_invalid_state() {
        let result = validate_computable_field_map(&Value::Null);
        assert!(result.is_reject());
    }

    #[test]
    fn validate_project_request_rejects_non_object_extensions_via_wire() {
        let result = validate_project_request_wire(&json!({
            "session_id": "sess_tw_dawn_arrival",
            "entry_id": "kb_tw_harbor",
            "state": { "tide_level": 2.1 },
            "extensions": []
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    fn valid_compute_request() -> ComputeRequest {
        ComputeRequest {
            computable: Map::from_iter([
                ("tide_level".into(), json!(2.5)),
                ("cargo_tons".into(), json!(37)),
            ]),
            entry_id: "kb_tw_harbor".into(),
            extensions: HashMap::new(),
            session_id: "sess_tw_dawn_arrival".into(),
            settle: None,
        }
    }

    #[test]
    fn validate_compute_request_accepts_valid_request() {
        assert!(validate_compute_request(&valid_compute_request()).is_ok());
    }

    #[test]
    fn validate_compute_request_accepts_settle_true() {
        let mut request = valid_compute_request();
        request.settle = Some(true);

        assert!(validate_compute_request(&request).is_ok());
    }

    #[test]
    fn validate_compute_request_rejects_non_boolean_settle_via_wire() {
        let result = validate_compute_request_wire(&json!({
            "session_id": "sess_tw_dawn_arrival",
            "entry_id": "kb_tw_harbor",
            "computable": { "tide_level": 2.5 },
            "settle": "true"
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn validate_compute_request_rejects_invalid_computable_map() {
        let result = validate_compute_request_wire(&json!({
            "session_id": "sess_tw_dawn_arrival",
            "entry_id": "kb_tw_harbor",
            "computable": []
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }
}
