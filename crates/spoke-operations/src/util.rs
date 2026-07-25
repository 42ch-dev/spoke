//! Typify ergonomics helpers — read dynamic `body` fields without parallel wire DTOs.

use serde_json::Value;
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
    serde_json::to_value(body).unwrap_or(Value::Object(Default::default()))
}
