//! Typify ergonomics helpers — read dynamic `body` fields without parallel wire DTOs.

use crate::result::{spoke_reject, spoke_ok_unit, SpokeRejectCode, SpokeResult};
use serde_json::{Map, Value};
use spoke_schemas::knowledge_entry::KnowledgeEntry;
use spoke_schemas::knowledge_entry::KnowledgeEntryBody;

/// SchemaVersion domain (`[1, 2^53-1]`), exact in the two stacks.
pub(crate) const MAX_SCHEMA_VERSION: u64 = (1 << 53) - 1;

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

/// Validate wire `body` is a JSON object (TS assemble `validateAssembleKnowledgeEntry`).
pub(crate) fn validate_assemble_body_wire(body_wire: &Value, entry_id: &str) -> SpokeResult<()> {
    if body_wire.is_object() {
        return spoke_ok_unit();
    }

    let mut details = Map::new();
    details.insert("entry_id".into(), Value::String(entry_id.to_owned()));
    details.insert("field".into(), Value::String("body".into()));
    spoke_reject(
        SpokeRejectCode::InvalidPacketInput,
        "KnowledgeEntry body must be an object",
        Some(details),
    )
}

/// Preserve `body` wire JSON from a KnowledgeEntry wire object before typify deserialize.
pub(crate) fn body_wire_from_entry_wire(entry_wire: &Value) -> Value {
    entry_wire
        .get("body")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// The nested provenance-anchor `schema_version`s obey the same
/// `[1, 2^53-1]` SchemaVersion domain as the entry's own member: the
/// generated `NonZeroU64` carriers have no ceiling, so the typed decode
/// alone cannot enforce it.
pub(crate) fn validate_anchor_schema_versions(entry: &KnowledgeEntry) -> SpokeResult<()> {
    let anchors = [
        (
            "source_anchor",
            entry.source_anchor.as_ref().map(|a| a.schema_version.0.get()),
        ),
        (
            "adoption_anchor",
            entry
                .adoption_anchor
                .as_ref()
                .map(|a| a.schema_version.0.get()),
        ),
    ];
    for (field, version) in anchors {
        let Some(version) = version else { continue };
        if version > MAX_SCHEMA_VERSION {
            let mut details = Map::new();
            details.insert("field".into(), Value::String(field.to_owned()));
            details.insert("schema_version".into(), Value::from(version));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("KnowledgeEntry {field} schema_version must be an integer in [1, 2^53-1]"),
                Some(details),
            );
        }
    }
    spoke_ok_unit()
}

/// Inclusive revision domain (`[0, 2^53]`), exact in the two stacks.
pub(crate) const MAX_REVISION: u64 = 1 << 53;

/// Validate wire `revision` per promote rules (TS `validateRevision`):
/// omitted or `null` is fine, otherwise a non-negative integer in the
/// inclusive `[0, 2^53]` revision domain. An exact integer token is
/// compared as itself (an exact `2^53 + 1` refuses); only an integral float
/// spelling falls back to its parsed value.
pub(crate) fn validate_revision_wire(revision: Option<&Value>) -> SpokeResult<()> {
    let Some(value) = revision else {
        return spoke_ok_unit();
    };

    if value.is_null() {
        return spoke_ok_unit();
    }

    let in_domain = match value.as_u64() {
        Some(revision) => revision <= MAX_REVISION,
        None => value.as_f64().is_some_and(|revision| {
            revision.fract() == 0.0 && (0.0..=MAX_REVISION as f64).contains(&revision)
        }),
    };

    if in_domain {
        return spoke_ok_unit();
    }

    let mut details = Map::new();
    details.insert("revision".into(), value.clone());
    spoke_reject(
        SpokeRejectCode::InvalidInput,
        "KnowledgeEntry revision must be an integer in [0, 2^53] or omitted",
        Some(details),
    )
}
