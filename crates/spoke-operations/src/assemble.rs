//! AssemblePacket builders.

use crate::extensions::ExtensionMap;
use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::util::{
    body_wire_from_entry_wire, extract_snippet_from_body_wire, knowledge_entry_body_to_json,
    validate_assemble_body_wire,
};
use serde_json::{Map, Value};
use spoke_schemas::assemble_packet::{
    AssemblePacket, AssemblePacketEntriesItem, AssemblePacketEntriesItemCanonicalName,
    AssemblePacketExtensionsKey,
};
use spoke_schemas::knowledge_entry::KnowledgeEntry;
use std::collections::HashMap;
use std::num::NonZeroU64;

fn default_schema_version() -> NonZeroU64 {
    NonZeroU64::new(1).expect("schema version 1 is non-zero")
}

/// KnowledgeEntry with preserved body wire JSON for dynamic field reads (e.g. `body.summary`).
///
/// Integrators deserializing from wire JSON should use [`KnowledgeEntryForAssemble::from_wire_json`]
/// so typify-stripped `body` additionalProperties remain available to assemble helpers.
#[derive(Debug, Clone)]
pub struct KnowledgeEntryForAssemble {
    pub entry: KnowledgeEntry,
    body_wire: Value,
}

impl KnowledgeEntryForAssemble {
    /// Deserialize from wire JSON via `spoke-schemas`, preserving body additionalProperties.
    pub fn from_wire_json(wire: Value) -> SpokeResult<Self> {
        let body_wire = body_wire_from_entry_wire(&wire);
        let entry_id = wire
            .get("entry_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");

        if let SpokeResult::Reject(reject) = validate_assemble_body_wire(&body_wire, entry_id) {
            return SpokeResult::Reject(reject);
        }

        let entry: KnowledgeEntry = match serde_json::from_value(wire) {
            Ok(entry) => entry,
            Err(error) => {
                return spoke_reject(
                    SpokeRejectCode::InvalidPacketInput,
                    format!("Invalid KnowledgeEntry wire JSON: {error}"),
                    None,
                );
            }
        };

        spoke_ok(Self { entry, body_wire })
    }

    /// Wrap a programmatically constructed entry (typify-known body fields only).
    #[must_use]
    pub fn from_entry(entry: KnowledgeEntry) -> Self {
        let body_wire = knowledge_entry_body_to_json(&entry.body);
        Self { entry, body_wire }
    }

    /// Typed KnowledgeEntry view.
    #[must_use]
    pub fn entry(&self) -> &KnowledgeEntry {
        &self.entry
    }
}

fn validate_assemble_knowledge_entry(
    knowledge_entry: &KnowledgeEntryForAssemble,
) -> SpokeResult<()> {
    if let SpokeResult::Reject(reject) =
        validate_assemble_body_wire(&knowledge_entry.body_wire, &knowledge_entry.entry.entry_id)
    {
        return SpokeResult::Reject(reject);
    }

    if knowledge_entry.entry.canonical_name.as_str().trim().is_empty() {
        let mut details = Map::new();
        details.insert(
            "entry_id".into(),
            Value::String(knowledge_entry.entry.entry_id.clone()),
        );
        details.insert("field".into(), Value::String("canonical_name".into()));
        return spoke_reject(
            SpokeRejectCode::InvalidPacketInput,
            "KnowledgeEntry canonical_name must be non-empty",
            Some(details),
        );
    }

    spoke_ok_unit()
}

fn map_to_assemble_entry(
    knowledge_entry: &KnowledgeEntry,
    body_wire: &Value,
) -> SpokeResult<AssemblePacketEntriesItem> {
    let canonical_name = match AssemblePacketEntriesItemCanonicalName::try_from(
        knowledge_entry.canonical_name.to_string(),
    ) {
        Ok(name) => name,
        Err(_) => {
            let mut details = Map::new();
            details.insert(
                "entry_id".into(),
                Value::String(knowledge_entry.entry_id.clone()),
            );
            details.insert("field".into(), Value::String("canonical_name".into()));
            return spoke_reject(
                SpokeRejectCode::InvalidPacketInput,
                "KnowledgeEntry canonical_name must be a string",
                Some(details),
            );
        }
    };

    let mut entry = AssemblePacketEntriesItem {
        canonical_name,
        entry_id: knowledge_entry.entry_id.clone(),
        entry_type: knowledge_entry.entry_type.clone(),
        snippet: None,
    };

    if let Some(snippet) = extract_snippet_from_body_wire(body_wire) {
        entry.snippet = Some(snippet);
    }

    spoke_ok(entry)
}

/// Map a KnowledgeEntry to a slim assemble entry per wire rules.
pub fn knowledge_entry_to_assemble_entry(
    knowledge_entry: &KnowledgeEntryForAssemble,
) -> SpokeResult<AssemblePacketEntriesItem> {
    if let SpokeResult::Reject(reject) = validate_assemble_knowledge_entry(knowledge_entry) {
        return SpokeResult::Reject(reject);
    }

    map_to_assemble_entry(&knowledge_entry.entry, &knowledge_entry.body_wire)
}

/// Input for `build_assemble_packet`.
pub struct BuildAssemblePacketInput<'a> {
    pub packet_id: &'a str,
    pub knowledge_entries: &'a [KnowledgeEntryForAssemble],
    pub extensions: Option<&'a ExtensionMap>,
    pub max_entries: Option<usize>,
}

/// Build a wire-valid `AssemblePacket` from KnowledgeEntries (order-preserving truncate only).
pub fn build_assemble_packet(
    input: BuildAssemblePacketInput<'_>,
) -> SpokeResult<AssemblePacket> {
    if input.packet_id.trim().is_empty() {
        let mut details = Map::new();
        details.insert("packetId".into(), Value::String(input.packet_id.into()));
        return spoke_reject(
            SpokeRejectCode::InvalidPacketInput,
            "packetId must be a non-empty string",
            Some(details),
        );
    }

    for knowledge_entry in input.knowledge_entries {
        if let SpokeResult::Reject(reject) = validate_assemble_knowledge_entry(knowledge_entry) {
            return SpokeResult::Reject(reject);
        }
    }

    let mut entries = Vec::with_capacity(input.knowledge_entries.len());
    for knowledge_entry in input.knowledge_entries {
        match map_to_assemble_entry(&knowledge_entry.entry, &knowledge_entry.body_wire) {
            SpokeResult::Ok(entry) => entries.push(entry),
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
    }

    let truncated_entries = match input.max_entries {
        Some(max) => entries.into_iter().take(max).collect(),
        None => entries,
    };

    let extensions = extension_map_to_packet(input.extensions.unwrap_or(&HashMap::new()));

    spoke_ok(AssemblePacket {
        entries: truncated_entries,
        extensions,
        packet_id: input.packet_id.into(),
        schema_version: default_schema_version(),
    })
}

fn extension_map_to_packet(
    extensions: &ExtensionMap,
) -> HashMap<AssemblePacketExtensionsKey, Map<String, Value>> {
    extensions
        .iter()
        .filter_map(|(key, value)| {
            AssemblePacketExtensionsKey::try_from(key.as_str())
                .ok()
                .map(|typed_key| (typed_key, value.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use spoke_schemas::knowledge_entry::{KnowledgeEntryBody, KnowledgeEntryCanonicalName};

    fn make_knowledge_entry(overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
        let mut entry = KnowledgeEntry {
            body: KnowledgeEntryBody::default(),
            canonical_name: KnowledgeEntryCanonicalName::try_from("Mira Vale".to_owned()).unwrap(),
            created_at: None,
            entry_id: "kb_1".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
            revision: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            source_anchor: None,
            status: "confirmed".into(),
            updated_at: None,
        };
        overrides(&mut entry);
        entry
    }

    fn entry_from_wire(wire: Value) -> KnowledgeEntryForAssemble {
        match KnowledgeEntryForAssemble::from_wire_json(wire) {
            SpokeResult::Ok(entry) => entry,
            SpokeResult::Reject(reject) => panic!("valid KnowledgeEntry wire JSON: {reject:?}"),
        }
    }

    fn unwrap_assemble_entry(entry: &KnowledgeEntryForAssemble) -> AssemblePacketEntriesItem {
        match knowledge_entry_to_assemble_entry(entry) {
            SpokeResult::Ok(mapped) => mapped,
            SpokeResult::Reject(reject) => panic!("assemble entry: {reject:?}"),
        }
    }

    #[test]
    fn maps_core_fields_with_snippet_from_wire_deserialize() {
        let entry = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": "  Hero  " },
            "extensions": {}
        }));

        let mapped = unwrap_assemble_entry(&entry);

        assert_eq!(mapped.entry_id, "kb_1");
        assert_eq!(mapped.entry_type, "character");
        assert_eq!(mapped.canonical_name.as_str(), "Mira Vale");
        assert_eq!(mapped.snippet.as_deref(), Some("Hero"));
    }

    #[test]
    fn build_packet_snippet_from_wire_deserialize() {
        let entry = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": "Hero" },
            "extensions": {}
        }));

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_snippet",
            knowledge_entries: std::slice::from_ref(&entry),
            extensions: None,
            max_entries: None,
        });

        assert!(result.is_ok());
        if let SpokeResult::Ok(packet) = result {
            assert_eq!(packet.entries.len(), 1);
            assert_eq!(packet.entries[0].snippet.as_deref(), Some("Hero"));
        }
    }

    #[test]
    fn rejects_null_body_in_build_packet_f002() {
        let entry = KnowledgeEntryForAssemble {
            entry: make_knowledge_entry(|_| {}),
            body_wire: Value::Null,
        };

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_5",
            knowledge_entries: std::slice::from_ref(&entry),
            extensions: None,
            max_entries: None,
        });

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
            assert_eq!(reject.message, "KnowledgeEntry body must be an object");
        }
    }

    #[test]
    fn rejects_null_body_from_wire_json_f002() {
        let result = KnowledgeEntryForAssemble::from_wire_json(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": null,
            "extensions": {}
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
            assert_eq!(reject.message, "KnowledgeEntry body must be an object");
        }
    }

    #[test]
    fn omits_snippet_for_non_string_summary() {
        let entry = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": 42 },
            "extensions": {}
        }));

        let mapped = unwrap_assemble_entry(&entry);
        assert!(mapped.snippet.is_none());
    }

    #[test]
    fn omits_snippet_for_whitespace_only_summary() {
        let entry = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": "   " },
            "extensions": {}
        }));

        let mapped = unwrap_assemble_entry(&entry);
        assert!(mapped.snippet.is_none());
    }

    #[test]
    fn builds_packet_with_empty_list() {
        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_1",
            knowledge_entries: &[],
            extensions: None,
            max_entries: None,
        });

        assert!(result.is_ok());
        if let SpokeResult::Ok(packet) = result {
            assert_eq!(packet.packet_id, "pkt_1");
            assert_eq!(packet.schema_version.get(), 1);
            assert!(packet.entries.is_empty());
            assert!(packet.extensions.is_empty());
        }
    }

    #[test]
    fn truncates_entries_in_input_order() {
        let entries = vec![
            KnowledgeEntryForAssemble::from_entry(make_knowledge_entry(|entry| {
                entry.entry_id = "kb_a".into();
                entry.canonical_name =
                    KnowledgeEntryCanonicalName::try_from("A".to_owned()).unwrap();
            })),
            KnowledgeEntryForAssemble::from_entry(make_knowledge_entry(|entry| {
                entry.entry_id = "kb_b".into();
                entry.canonical_name =
                    KnowledgeEntryCanonicalName::try_from("B".to_owned()).unwrap();
            })),
            KnowledgeEntryForAssemble::from_entry(make_knowledge_entry(|entry| {
                entry.entry_id = "kb_c".into();
                entry.canonical_name =
                    KnowledgeEntryCanonicalName::try_from("C".to_owned()).unwrap();
            })),
        ];

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_2",
            knowledge_entries: &entries,
            extensions: None,
            max_entries: Some(2),
        });

        assert!(result.is_ok());
        if let SpokeResult::Ok(packet) = result {
            let ids: Vec<_> = packet.entries.iter().map(|entry| entry.entry_id.as_str()).collect();
            assert_eq!(ids, vec!["kb_a", "kb_b"]);
        }
    }

    #[test]
    fn passes_extensions_through() {
        let mut extensions = ExtensionMap::new();
        extensions.insert(
            "nexus".into(),
            Map::from_iter([("profile".into(), json!("chat"))]),
        );

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_3",
            knowledge_entries: &[],
            extensions: Some(&extensions),
            max_entries: None,
        });

        assert!(result.is_ok());
        if let SpokeResult::Ok(packet) = result {
            let key = AssemblePacketExtensionsKey::try_from("nexus").unwrap();
            assert_eq!(
                packet.extensions.get(&key).and_then(|m| m.get("profile")),
                Some(&json!("chat"))
            );
        }
    }

    #[test]
    fn rejects_empty_packet_id() {
        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "  ",
            knowledge_entries: &[],
            extensions: None,
            max_entries: None,
        });

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
        }
    }
}
