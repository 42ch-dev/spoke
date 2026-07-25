//! AssemblePacket builders.

use crate::extensions::ExtensionMap;
use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::util::{extract_snippet_from_body_wire, knowledge_entry_body_to_json};
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

fn validate_assemble_knowledge_entry(knowledge_entry: &KnowledgeEntry) -> SpokeResult<()> {
    if knowledge_entry.canonical_name.as_str().trim().is_empty() {
        let mut details = Map::new();
        details.insert(
            "entry_id".into(),
            Value::String(knowledge_entry.entry_id.clone()),
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
) -> AssemblePacketEntriesItem {
    let mut entry = AssemblePacketEntriesItem {
        canonical_name: AssemblePacketEntriesItemCanonicalName::try_from(
            knowledge_entry.canonical_name.to_string(),
        )
        .unwrap_or_else(|_| {
            AssemblePacketEntriesItemCanonicalName::try_from("placeholder".to_owned())
                .expect("placeholder")
        }),
        entry_id: knowledge_entry.entry_id.clone(),
        entry_type: knowledge_entry.entry_type.clone(),
        snippet: None,
    };

    if let Some(snippet) = extract_snippet_from_body_wire(body_wire) {
        entry.snippet = Some(snippet);
    }

    entry
}

/// Map a KnowledgeEntry to a slim assemble entry per wire rules.
#[must_use]
pub fn knowledge_entry_to_assemble_entry(
    knowledge_entry: &KnowledgeEntry,
) -> AssemblePacketEntriesItem {
    let body_wire = knowledge_entry_body_to_json(&knowledge_entry.body);
    map_to_assemble_entry(knowledge_entry, &body_wire)
}

/// Map using wire JSON for `body` (preserves `summary` and other additionalProperties stripped by typify).
#[cfg(test)]
#[must_use]
pub(crate) fn knowledge_entry_to_assemble_entry_with_body_wire(
    knowledge_entry: &KnowledgeEntry,
    body_wire: &Value,
) -> AssemblePacketEntriesItem {
    map_to_assemble_entry(knowledge_entry, body_wire)
}

/// Input for `build_assemble_packet`.
pub struct BuildAssemblePacketInput<'a> {
    pub packet_id: &'a str,
    pub knowledge_entries: &'a [KnowledgeEntry],
    pub extensions: Option<&'a ExtensionMap>,
    pub max_entries: Option<usize>,
    /// Parallel body wire JSON per entry (typify preserves only known `body` keys on struct deserialize).
    pub knowledge_entry_body_wires: Option<&'a [Value]>,
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

    if let Some(body_wires) = input.knowledge_entry_body_wires {
        if body_wires.len() != input.knowledge_entries.len() {
            return spoke_reject(
                SpokeRejectCode::InvalidPacketInput,
                "knowledge_entry_body_wires length must match knowledge_entries",
                None,
            );
        }
    }

    for knowledge_entry in input.knowledge_entries {
        if let SpokeResult::Reject(reject) = validate_assemble_knowledge_entry(knowledge_entry) {
            return SpokeResult::Reject(reject);
        }
    }

    let entries: Vec<AssemblePacketEntriesItem> = input
        .knowledge_entries
        .iter()
        .enumerate()
        .map(|(index, knowledge_entry)| {
            let body_wire = input
                .knowledge_entry_body_wires
                .and_then(|wires| wires.get(index))
                .map_or_else(
                    || knowledge_entry_body_to_json(&knowledge_entry.body),
                    Clone::clone,
                );
            map_to_assemble_entry(knowledge_entry, &body_wire)
        })
        .collect();

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

    fn entry_from_wire(wire: Value) -> (KnowledgeEntry, Value) {
        let body = wire.get("body").cloned().unwrap_or_else(|| json!({}));
        let entry: KnowledgeEntry = serde_json::from_value(wire).expect("valid KnowledgeEntry");
        (entry, body)
    }

    #[test]
    fn maps_core_fields_with_snippet() {
        let (entry, body) = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": "  Hero  " },
            "extensions": {}
        }));

        let mapped = knowledge_entry_to_assemble_entry_with_body_wire(&entry, &body);

        assert_eq!(mapped.entry_id, "kb_1");
        assert_eq!(mapped.entry_type, "character");
        assert_eq!(mapped.canonical_name.as_str(), "Mira Vale");
        assert_eq!(mapped.snippet.as_deref(), Some("Hero"));
    }

    #[test]
    fn omits_snippet_for_non_string_summary() {
        let (entry, body) = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": 42 },
            "extensions": {}
        }));

        let mapped = knowledge_entry_to_assemble_entry_with_body_wire(&entry, &body);
        assert!(mapped.snippet.is_none());
    }

    #[test]
    fn omits_snippet_for_whitespace_only_summary() {
        let (entry, body) = entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": "   " },
            "extensions": {}
        }));

        let mapped = knowledge_entry_to_assemble_entry_with_body_wire(&entry, &body);
        assert!(mapped.snippet.is_none());
    }

    #[test]
    fn builds_packet_with_empty_list() {
        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_1",
            knowledge_entries: &[],
            extensions: None,
            max_entries: None,
            knowledge_entry_body_wires: None,
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
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_a".into();
                entry.canonical_name =
                    KnowledgeEntryCanonicalName::try_from("A".to_owned()).unwrap();
            }),
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_b".into();
                entry.canonical_name =
                    KnowledgeEntryCanonicalName::try_from("B".to_owned()).unwrap();
            }),
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_c".into();
                entry.canonical_name =
                    KnowledgeEntryCanonicalName::try_from("C".to_owned()).unwrap();
            }),
        ];

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_2",
            knowledge_entries: &entries,
            extensions: None,
            max_entries: Some(2),
            knowledge_entry_body_wires: None,
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
            knowledge_entry_body_wires: None,
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
            knowledge_entry_body_wires: None,
        });

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
        }
    }
}
