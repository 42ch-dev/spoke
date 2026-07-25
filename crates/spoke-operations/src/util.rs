//! Typify ergonomics helpers — read dynamic `body` fields without parallel wire DTOs.

use crate::result::{spoke_reject, spoke_ok_unit, SpokeRejectCode, SpokeResult};
use serde_json::{Map, Value};
use spoke_schemas::knowledge_entry::KnowledgeEntryBody;

/// Extract a trimmed non-empty string field from a JSON object value.
pub(crate) fn trimmed_string_field(value: &Value, field: &str) -> Option<String> {
    let text = value.get(field)?.as_str()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Read `body.summary` per assemble rules from wire JSON (preserves typify-stripped keys).
pub(crate) fn extract_snippet_from_body_wire(body: &Value) -> Option<String> {
    trimmed_string_field(body, "summary")
}

/// Serialize a generated `KnowledgeEntryBody` to JSON for field reads (known fields only).
pub(crate) fn knowledge_entry_body_to_json(body: &KnowledgeEntryBody) -> Value {
    serde_json::to_value(body).unwrap_or(Value::Object(Map::new()))
}

/// Preserve `body` wire JSON from a KnowledgeEntry wire object before typify deserialize.
pub(crate) fn body_wire_from_entry_wire(entry_wire: &Value) -> Value {
    entry_wire
        .get("body")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// Validate wire `revision` per promote rules (TS `validateRevision`).
pub(crate) fn validate_revision_wire(revision: Option<&Value>) -> SpokeResult<()> {
    let Some(value) = revision else {
        return spoke_ok_unit();
    };

    if value.is_null() {
        return spoke_ok_unit();
    }

    let number = match value {
        Value::Number(number) => number,
        _ => {
            let mut details = Map::new();
            details.insert("revision".into(), value.clone());
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "KnowledgeEntry revision must be a non-negative integer or omitted",
                Some(details),
            );
        }
    };

    if let Some(revision) = number.as_u64() {
        let _ = revision;
        return spoke_ok_unit();
    }

    if let Some(revision) = number.as_i64() {
        if revision < 0 {
            let mut details = Map::new();
            details.insert("revision".into(), value.clone());
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "KnowledgeEntry revision must be a non-negative integer or omitted",
                Some(details),
            );
        }
        return spoke_ok_unit();
    }

    let mut details = Map::new();
    details.insert("revision".into(), value.clone());
    spoke_reject(
        SpokeRejectCode::InvalidInput,
        "KnowledgeEntry revision must be a non-negative integer or omitted",
        Some(details),
    )
}
