//! KnowledgeEntry `body.attributes` read helpers.

use serde_json::Value;
use spoke_schemas::knowledge_entry::{
    KnowledgeEntry, KnowledgeEntryBody, KnowledgeEntryBodyAttributesItem,
    KnowledgeEntryBodyAttributesItemValue,
};
use spoke_schemas::{BodyAttribute, BodyAttributeTraitType, BodyAttributeValue};

/// Input for body attribute read helpers: entry, body, or wire JSON (`None` = absent).
#[derive(Debug, Clone, Copy)]
pub enum BodyAttributesInput<'a> {
    Entry(&'a KnowledgeEntry),
    Body(&'a KnowledgeEntryBody),
    Wire(Option<&'a Value>),
}

fn is_plain_object(value: &Value) -> bool {
    value.is_object() && !value.is_null()
}

fn is_body_attribute_value(value: &Value) -> Option<BodyAttributeValue> {
    match value {
        Value::String(text) => Some(BodyAttributeValue::Variant0(text.clone())),
        Value::Number(number) => number.as_f64().map(BodyAttributeValue::Variant1),
        Value::Bool(flag) => Some(BodyAttributeValue::Variant2(*flag)),
        _ => None,
    }
}

fn parse_body_attribute_wire(value: &Value) -> Option<BodyAttribute> {
    let object = value.as_object()?;
    let trait_type = object.get("trait_type")?.as_str()?;
    if trait_type.is_empty() {
        return None;
    }

    let attribute_value = is_body_attribute_value(object.get("value")?)?;
    let trait_type = BodyAttributeTraitType::try_from(trait_type.to_owned()).ok()?;

    Some(BodyAttribute {
        trait_type,
        value: attribute_value,
        display_type: object
            .get("display_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        max_value: object.get("max_value").and_then(Value::as_f64),
    })
}

fn extract_attributes_array_wire(input: Option<&Value>) -> Vec<&Value> {
    let Some(value) = input else {
        return Vec::new();
    };

    if !is_plain_object(value) {
        return Vec::new();
    }

    let object = value.as_object().expect("plain object");
    let body = if object.contains_key("entry_id") && object.contains_key("body") {
        object.get("body")
    } else {
        Some(value)
    };

    let Some(body) = body else {
        return Vec::new();
    };

    if !is_plain_object(body) {
        return Vec::new();
    }

    let attributes = body
        .as_object()
        .expect("plain object")
        .get("attributes")
        .unwrap_or(&Value::Null);

    attributes
        .as_array()
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn typed_item_to_body_attribute(item: &KnowledgeEntryBodyAttributesItem) -> BodyAttribute {
    BodyAttribute {
        trait_type: BodyAttributeTraitType::try_from(item.trait_type.as_str().to_owned())
            .expect("typed attribute trait_type is valid"),
        value: match &item.value {
            KnowledgeEntryBodyAttributesItemValue::Variant0(text) => {
                BodyAttributeValue::Variant0(text.clone())
            }
            KnowledgeEntryBodyAttributesItemValue::Variant1(number) => {
                BodyAttributeValue::Variant1(*number)
            }
            KnowledgeEntryBodyAttributesItemValue::Variant2(flag) => {
                BodyAttributeValue::Variant2(*flag)
            }
        },
        display_type: item.display_type.clone(),
        max_value: item.max_value,
    }
}

fn list_body_attributes_from_wire(input: Option<&Value>) -> Vec<BodyAttribute> {
    let mut result = Vec::new();

    for element in extract_attributes_array_wire(input) {
        if let Some(attribute) = parse_body_attribute_wire(element) {
            result.push(attribute);
        }
    }

    result
}

/// Lists valid `body.attributes` traits in array order.
///
/// Accepts a [`KnowledgeEntry`], its body, or wire JSON. Absent wire input yields `[]`.
#[must_use]
pub fn list_body_attributes(input: BodyAttributesInput<'_>) -> Vec<BodyAttribute> {
    match input {
        BodyAttributesInput::Entry(entry) => list_body_attributes(BodyAttributesInput::Body(
            &entry.body,
        )),
        BodyAttributesInput::Body(body) => body
            .attributes
            .iter()
            .map(typed_item_to_body_attribute)
            .collect(),
        BodyAttributesInput::Wire(value) => list_body_attributes_from_wire(value),
    }
}

/// Returns all traits with the given `trait_type` in original array order.
#[must_use]
pub fn filter_body_attributes_by_trait_type(
    input: BodyAttributesInput<'_>,
    trait_type: &str,
) -> Vec<BodyAttribute> {
    list_body_attributes(input)
        .into_iter()
        .filter(|attribute| attribute.trait_type.as_str() == trait_type)
        .collect()
}

/// Returns the first trait with the given `trait_type`, if any.
#[must_use]
pub fn find_body_attribute(
    input: BodyAttributesInput<'_>,
    trait_type: &str,
) -> Option<BodyAttribute> {
    filter_body_attributes_by_trait_type(input, trait_type)
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use spoke_schemas::knowledge_entry::{KnowledgeEntryBody, KnowledgeEntryCanonicalName};
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn assert_body_attribute_eq(left: &BodyAttribute, right: &BodyAttribute) {
        assert_eq!(left.trait_type.as_str(), right.trait_type.as_str());
        assert_eq!(format!("{:?}", left.value), format!("{:?}", right.value));
        assert_eq!(left.display_type, right.display_type);
        assert_eq!(left.max_value, right.max_value);
    }

    fn affiliation_guild() -> BodyAttribute {
        BodyAttribute {
            trait_type: BodyAttributeTraitType::try_from("affiliation".to_owned()).unwrap(),
            value: BodyAttributeValue::Variant0("Guild".into()),
            display_type: None,
            max_value: None,
        }
    }

    fn affiliation_crown() -> BodyAttribute {
        BodyAttribute {
            trait_type: BodyAttributeTraitType::try_from("affiliation".to_owned()).unwrap(),
            value: BodyAttributeValue::Variant0("Crown".into()),
            display_type: None,
            max_value: None,
        }
    }

    fn role_protagonist() -> BodyAttribute {
        BodyAttribute {
            trait_type: BodyAttributeTraitType::try_from("role".to_owned()).unwrap(),
            value: BodyAttributeValue::Variant0("protagonist".into()),
            display_type: None,
            max_value: None,
        }
    }

    fn make_knowledge_entry(overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
        let mut entry = KnowledgeEntry {
            body: KnowledgeEntryBody::default(),
            canonical_name: KnowledgeEntryCanonicalName::try_from("Mira Vale".to_owned()).unwrap(),
            created_at: None,
            entry_id: "kb_1".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
            modules: HashMap::new(),
            revision: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            source_anchor: None,
            status: "confirmed".into(),
            updated_at: None,
        };
        overrides(&mut entry);
        entry
    }

    #[test]
    fn list_returns_empty_when_wire_input_is_absent() {
        assert!(list_body_attributes(BodyAttributesInput::Wire(None)).is_empty());
    }

    #[test]
    fn list_returns_empty_when_attributes_are_omitted_or_empty() {
        let summary_only = json!({ "summary": "Only summary" });
        assert!(list_body_attributes(BodyAttributesInput::Wire(Some(&summary_only))).is_empty());

        let empty_attributes = json!({ "attributes": [] });
        assert!(list_body_attributes(BodyAttributesInput::Wire(Some(&empty_attributes))).is_empty());
    }

    #[test]
    fn list_returns_empty_when_entry_body_or_attributes_are_absent() {
        let entry = make_knowledge_entry(|_| {});
        assert!(list_body_attributes(BodyAttributesInput::Entry(&entry)).is_empty());
    }

    #[test]
    fn list_reads_attributes_from_full_knowledge_entry() {
        let entry: KnowledgeEntry = serde_json::from_value(json!({
            "schema_version": 1,
            "entry_id": "kb_1",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": {
                "attributes": [
                    { "trait_type": "affiliation", "value": "Guild" },
                    { "trait_type": "role", "value": "protagonist" }
                ]
            },
            "extensions": {}
        }))
        .expect("knowledge entry json");

        let listed = list_body_attributes(BodyAttributesInput::Entry(&entry));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].trait_type.as_str(), "affiliation");
        assert_eq!(listed[1].trait_type.as_str(), "role");
    }

    #[test]
    fn list_returns_valid_traits_in_order_and_skips_malformed_elements() {
        let body = json!({
            "attributes": [
                { "trait_type": "affiliation", "value": "Guild" },
                { "trait_type": "affiliation", "value": "Crown" },
                { "trait_type": "", "value": "empty-type" },
                { "trait_type": "role", "value": { "nested": true } },
                null,
                "not-an-object",
                { "trait_type": "level", "value": 3, "display_type": "number" },
                { "value": "missing-type" }
            ]
        });

        let listed = list_body_attributes(BodyAttributesInput::Wire(Some(&body)));
        assert_eq!(listed.len(), 3);
        assert_body_attribute_eq(&listed[0], &affiliation_guild());
        assert_body_attribute_eq(&listed[1], &affiliation_crown());
        assert_eq!(listed[2].trait_type.as_str(), "level");
        assert!(matches!(listed[2].value, BodyAttributeValue::Variant1(3.0)));
        assert_eq!(listed[2].display_type.as_deref(), Some("number"));
    }

    #[test]
    fn list_returns_empty_when_attributes_is_not_an_array() {
        let body = json!({ "attributes": "invalid" });
        assert!(list_body_attributes(BodyAttributesInput::Wire(Some(&body))).is_empty());
    }

    #[test]
    fn filter_returns_all_matches_in_order_for_duplicate_trait_types() {
        let body = json!({
            "attributes": [
                { "trait_type": "affiliation", "value": "Guild" },
                { "trait_type": "affiliation", "value": "Crown" },
                { "trait_type": "role", "value": "protagonist" }
            ]
        });
        let input = BodyAttributesInput::Wire(Some(&body));

        let filtered = filter_body_attributes_by_trait_type(input, "affiliation");
        assert_eq!(filtered.len(), 2);
        assert_body_attribute_eq(&filtered[0], &affiliation_guild());
        assert_body_attribute_eq(&filtered[1], &affiliation_crown());
    }

    #[test]
    fn filter_returns_empty_when_trait_type_has_no_matches() {
        let body = json!({
            "attributes": [
                { "trait_type": "affiliation", "value": "Guild" },
                { "trait_type": "affiliation", "value": "Crown" },
                { "trait_type": "role", "value": "protagonist" }
            ]
        });
        let input = BodyAttributesInput::Wire(Some(&body));

        assert!(filter_body_attributes_by_trait_type(input, "missing").is_empty());
        assert!(filter_body_attributes_by_trait_type(BodyAttributesInput::Wire(None), "affiliation")
            .is_empty());
    }

    #[test]
    fn filter_matches_trait_type_with_exact_case_sensitive_equality() {
        let body = json!({
            "attributes": [
                { "trait_type": "affiliation", "value": "Guild" },
                { "trait_type": "affiliation", "value": "Crown" },
                { "trait_type": "role", "value": "protagonist" }
            ]
        });
        let input = BodyAttributesInput::Wire(Some(&body));

        assert!(filter_body_attributes_by_trait_type(input, "Affiliation").is_empty());
        let filtered = filter_body_attributes_by_trait_type(input, "affiliation");
        assert_eq!(filtered.len(), 2);
        assert_body_attribute_eq(&filtered[0], &affiliation_guild());
        assert_body_attribute_eq(&filtered[1], &affiliation_crown());
    }

    #[test]
    fn find_returns_first_matching_trait_in_array_order() {
        let body = json!({
            "attributes": [
                { "trait_type": "affiliation", "value": "Guild" },
                { "trait_type": "affiliation", "value": "Crown" },
                { "trait_type": "role", "value": "protagonist" }
            ]
        });
        let input = BodyAttributesInput::Wire(Some(&body));

        let found = find_body_attribute(input, "affiliation").expect("affiliation");
        assert_body_attribute_eq(&found, &affiliation_guild());
        let found = find_body_attribute(input, "role").expect("role");
        assert_body_attribute_eq(&found, &role_protagonist());
    }

    #[test]
    fn find_returns_none_when_no_match_or_input_is_absent() {
        let body = json!({
            "attributes": [
                { "trait_type": "affiliation", "value": "Guild" },
                { "trait_type": "affiliation", "value": "Crown" },
                { "trait_type": "role", "value": "protagonist" }
            ]
        });
        let input = BodyAttributesInput::Wire(Some(&body));

        assert!(find_body_attribute(input, "missing").is_none());
        assert!(find_body_attribute(BodyAttributesInput::Wire(None), "role").is_none());

        let empty = json!({ "attributes": [] });
        assert!(
            find_body_attribute(BodyAttributesInput::Wire(Some(&empty)), "role").is_none()
        );
    }
}
