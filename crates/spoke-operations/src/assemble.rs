//! AssemblePacket builders.

use crate::cognition::{project_cognition_records, CurrentViewPolicy};
use crate::extensions::ExtensionMap;
use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::scope::{assert_supported_cognition_selection, cognition_records_of_authorized_entries};
use crate::util::{
    body_wire_from_entry_wire, extract_snippet_from_body_wire, knowledge_entry_body_to_json,
    validate_assemble_body_wire,
};
use serde_json::{Map, Value};
use spoke_schemas::assemble_packet::{
    AssembleEntry, AssembleEntryCanonicalName, AssemblePacket,
    CognitionProjection as PacketedCognitionProjection, ExtensionKey,
};
use spoke_schemas::data::cognition::{
    CognitionProjection as CognitionProjectionWire, CognitionSelection as CognitionSelectionWire,
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

/// Convert between the generated per-module re-declarations of one wire shape.
fn wire_convert<T, U>(value: &T) -> SpokeResult<U>
where
    T: serde::Serialize,
    U: serde::de::DeserializeOwned,
{
    let converted = serde_json::to_value(value)
        .ok()
        .and_then(|wire| serde_json::from_value(wire).ok());
    match converted {
        Some(converted) => spoke_ok(converted),
        None => spoke_reject(
            SpokeRejectCode::InvalidPacketInput,
            "cognition wire conversion failed",
            None,
        ),
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
) -> SpokeResult<AssembleEntry> {
    let canonical_name = match AssembleEntryCanonicalName::try_from(
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

    let mut entry = AssembleEntry {
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
) -> SpokeResult<AssembleEntry> {
    if let SpokeResult::Reject(reject) = validate_assemble_knowledge_entry(knowledge_entry) {
        return SpokeResult::Reject(reject);
    }

    map_to_assemble_entry(&knowledge_entry.entry, &knowledge_entry.body_wire)
}

/// Input for `build_assemble_packet`.
pub struct BuildAssemblePacketInput<'a> {
    pub packet_id: &'a str,
    /// Already scope-authorized entries: disclosure filtering happens before cognition.
    pub knowledge_entries: &'a [KnowledgeEntryForAssemble],
    pub extensions: Option<&'a ExtensionMap>,
    pub max_entries: Option<usize>,
    /// Optional derivative cognitive selection applied to the authorized set.
    pub cognition_selection: Option<&'a spoke_schemas::CognitionSelection>,
    /// Required exactly for a current assessed projection; never a built-in policy.
    pub current_view_policy: Option<&'a dyn CurrentViewPolicy>,
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

    // Optional derivative cognitive projection over the authorized source set.
    // Computed before `max_entries` truncation: the limit narrows the slim
    // context entries, never which authorized cognition was requested.
    let cognition: Option<PacketedCognitionProjection> = match input.cognition_selection {
        None => None,
        Some(selection) => {
            // This carrier supplies current cognition only; an unavailable view
            // rejects the whole build instead of returning a false empty result.
            if let SpokeResult::Reject(reject) =
                assert_supported_cognition_selection(Some(selection), "cognition_selection")
            {
                return SpokeResult::Reject(reject);
            }

            // Generated Rust re-declares the cognition family per wire module;
            // the established canonical-import rule converts at this boundary.
            let selection: CognitionSelectionWire = match wire_convert(selection) {
                SpokeResult::Ok(selection) => selection,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            let authorized_entries: Vec<&KnowledgeEntry> = input
                .knowledge_entries
                .iter()
                .map(KnowledgeEntryForAssemble::entry)
                .collect();
            let records = match cognition_records_of_authorized_entries(&authorized_entries) {
                SpokeResult::Ok(records) => records,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            let projection: CognitionProjectionWire =
                match project_cognition_records(&records, &selection, input.current_view_policy) {
                    SpokeResult::Ok(projection) => projection,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
            match wire_convert(&projection) {
                SpokeResult::Ok(projection) => Some(projection),
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            }
        }
    };

    let extensions = match extension_map_to_packet(input.extensions.unwrap_or(&HashMap::new())) {
        SpokeResult::Ok(extensions) => extensions,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    spoke_ok(AssemblePacket {
        cognition,
        entries: truncated_entries,
        extensions,
        modules: HashMap::new(),
        packet_id: input.packet_id.into(),
        schema_version: default_schema_version(),
    })
}

fn extension_map_to_packet(
    extensions: &ExtensionMap,
) -> SpokeResult<HashMap<ExtensionKey, Map<String, Value>>> {
    let mut packet_extensions = HashMap::new();

    for (key, value) in extensions {
        let typed_key = match ExtensionKey::try_from(key.as_str()) {
            Ok(typed_key) => typed_key,
            Err(_) => {
                let mut details = Map::new();
                details.insert("field".into(), Value::String("extensions".into()));
                details.insert("namespace".into(), Value::String(key.clone()));
                return spoke_reject(
                    SpokeRejectCode::InvalidPacketInput,
                    format!("AssemblePacket extensions namespace must match ^[a-z][a-z0-9_-]*$"),
                    Some(details),
                );
            }
        };
        packet_extensions.insert(typed_key, value.clone());
    }

    spoke_ok(packet_extensions)
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
            disclosure: None,
            entry_id: "kb_1".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
            modules: HashMap::new(),
            owner: None,
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

    fn unwrap_assemble_entry(entry: &KnowledgeEntryForAssemble) -> AssembleEntry {
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
            cognition_selection: None,
            current_view_policy: None,
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
            cognition_selection: None,
            current_view_policy: None,
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
    fn rejects_wire_json_with_non_string_summary() {
        let result = KnowledgeEntryForAssemble::from_wire_json(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": { "summary": 42 },
            "extensions": {}
        }));

        match result {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
            }
            SpokeResult::Ok(_) => panic!("expected reject for invalid summary type"),
        }
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
            cognition_selection: None,
            current_view_policy: None,
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
            cognition_selection: None,
            current_view_policy: None,
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
            cognition_selection: None,
            current_view_policy: None,
        });

        assert!(result.is_ok());
        if let SpokeResult::Ok(packet) = result {
            let key = ExtensionKey::try_from("nexus").unwrap();
            assert_eq!(
                packet.extensions.get(&key).and_then(|m| m.get("profile")),
                Some(&json!("chat"))
            );
        }
    }

    #[test]
    fn rejects_invalid_extension_namespace_key() {
        let mut extensions = ExtensionMap::new();
        extensions.insert(
            "InvalidNamespace".into(),
            Map::from_iter([("profile".into(), json!("chat"))]),
        );

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_ext",
            knowledge_entries: &[],
            extensions: Some(&extensions),
            max_entries: None,
            cognition_selection: None,
            current_view_policy: None,
        });

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
        }
    }

    #[test]
    fn rejects_empty_packet_id() {
        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "  ",
            knowledge_entries: &[],
            extensions: None,
            max_entries: None,
            cognition_selection: None,
            current_view_policy: None,
        });

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidPacketInput);
        }
    }
    // === P2/T2 cognition availability and projection source set ===

    use crate::cognition::CurrentViewPolicy;
    use spoke_schemas::data::cognition::CognitionRecord as SchemaRecord;

    struct FirstInputPolicy;

    impl CurrentViewPolicy for FirstInputPolicy {
        fn select(&self, assessments: &[&SchemaRecord]) -> Option<String> {
            assessments.first().map(|record| match record {
                SchemaRecord::BeliefRecord(record) => record.record_id.to_string(),
                SchemaRecord::EpistemicAssessment(record) => record.record_id.to_string(),
                SchemaRecord::PerceptionRecord(record) => record.record_id.to_string(),
            })
        }
    }

    fn cognition_record_json(entry_id: &str, record_id: &str) -> Value {
        json!({
            "kind": "belief",
            "record_id": record_id,
            "holder": { "kind": "knowledge_entry", "namespace": "world_a", "id": entry_id },
            "target": {
                "carrier": { "kind": "knowledge_entry", "namespace": "world_a", "id": "kb_box" },
                "target_id": "t_marble",
                "kind": "proposition",
                "text": "Mira hides the marble"
            },
            "assessor": { "kind": "principal", "namespace": "world_a", "id": "assessor_self" },
            "assessor_role": "holder_self_report",
            "basis": {
                "kind": "evidence",
                "evidence": [{ "kind": "knowledge_entry", "namespace": "world_a", "id": "kb_box" }]
            },
            "assessment_context": { "context_id": "ctx_main", "fork_id": "fork_alpha" },
            "effective_at": "2026-07-23T09:00:00Z",
            "assessed_at": "2026-07-23T09:05:00Z",
            "view": "current",
            "order": 1,
            "q": {
                "value": 0.9,
                "assertion": "belief_attribution",
                "method": "direct_report",
                "calibration": "uncalibrated"
            },
            "extensions": {}
        })
    }

    fn cognition_entry(entry_id: &str, record_id: &str) -> KnowledgeEntryForAssemble {
        entry_from_wire(json!({
            "schema_version": 1,
            "entry_id": entry_id,
            "entry_type": "character",
            "canonical_name": entry_id,
            "status": "confirmed",
            "body": {},
            "extensions": {},
            "modules": {
                "cognition": {
                    "records": [cognition_record_json(entry_id, record_id)],
                    "world_facts": [],
                    "extensions": {}
                }
            }
        }))
    }

    fn cognition_selection(view: &str) -> spoke_schemas::CognitionSelection {
        let wire = if view == "current" {
            json!({
                "view": view,
                "filter": {},
                "selection_policy": { "policy_id": "pol_first", "policy_version": "v1" }
            })
        } else {
            json!({ "view": view, "filter": {} })
        };
        serde_json::from_value(wire).expect("cognition selection deserializes")
    }

    #[test]
    fn recorded_at_view_rejects_instead_of_empty_projection() {
        let entry = cognition_entry("kb_a", "rec_a");
        let selection = cognition_selection("recorded_at");

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_recorded",
            knowledge_entries: std::slice::from_ref(&entry),
            extensions: None,
            max_entries: None,
            cognition_selection: Some(&selection),
            current_view_policy: None,
        });

        match result {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
                let details = reject.details.expect("unsupported requirement details");
                assert_eq!(
                    details.get("reason").and_then(Value::as_str),
                    Some("unsupported_cognition_requirement")
                );
                assert_eq!(
                    details.get("field").and_then(Value::as_str),
                    Some("cognition_selection.view")
                );
            }
            SpokeResult::Ok(_) => panic!("unavailable recorded_at view must reject"),
        }
    }

    #[test]
    fn projection_covers_authorized_source_set_before_limit() {
        let entries = vec![
            cognition_entry("kb_a", "rec_a"),
            cognition_entry("kb_b", "rec_b"),
        ];
        let selection = cognition_selection("current");
        let policy = FirstInputPolicy;

        let result = build_assemble_packet(BuildAssemblePacketInput {
            packet_id: "pkt_limit",
            knowledge_entries: &entries,
            extensions: None,
            max_entries: Some(1),
            cognition_selection: Some(&selection),
            current_view_policy: Some(&policy),
        });

        let SpokeResult::Ok(packet) = result else {
            panic!("limited packet must succeed");
        };
        assert_eq!(packet.entries.len(), 1);
        let projection = packet.cognition.expect("projection over the source set");
        assert_eq!(projection.items.len(), 2);
        assert_eq!(
            projection.items[0]
                .selection
                .as_ref()
                .map(|selection| selection.selected_record_id.as_str()),
            Some("rec_a")
        );
        assert_eq!(
            projection.items[1]
                .selection
                .as_ref()
                .map(|selection| selection.selected_record_id.as_str()),
            Some("rec_b")
        );
    }
}
