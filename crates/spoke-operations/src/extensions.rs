//! Extension map merge and round-trip preserve.

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Product extension namespace map (`ExtensionMap` wire shape).
pub type ExtensionMap = HashMap<String, Map<String, Value>>;

fn is_plain_object(value: &Value) -> bool {
    value.is_object() && !value.is_null()
}

fn clone_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(clone_value).collect()),
        Value::Object(map) => {
            let mut cloned = Map::new();
            for (key, nested) in map {
                cloned.insert(key.clone(), clone_value(nested));
            }
            Value::Object(cloned)
        }
        _ => value.clone(),
    }
}

fn clone_namespace(namespace: Option<&Map<String, Value>>) -> Map<String, Value> {
    namespace.map_or_else(Map::new, |map| {
        map.iter()
            .map(|(key, value)| (key.clone(), clone_value(value)))
            .collect()
    })
}

fn deep_merge_records(
    base: Option<&Map<String, Value>>,
    overlay: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut result = clone_namespace(base);

    let Some(overlay) = overlay else {
        return result;
    };

    for (key, overlay_value) in overlay {
        if let Some(base_value) = result.get(key) {
            if is_plain_object(base_value) && is_plain_object(overlay_value) {
                let merged = deep_merge_records(base_value.as_object(), overlay_value.as_object());
                result.insert(key.clone(), Value::Object(merged));
                continue;
            }
        }

        result.insert(key.clone(), clone_value(overlay_value));
    }

    result
}

fn merge_extension_maps_internal(base: &ExtensionMap, overlay: &ExtensionMap) -> ExtensionMap {
    let namespaces: HashSet<&String> = base.keys().chain(overlay.keys()).collect();
    let mut result = ExtensionMap::new();

    for namespace in namespaces {
        let merged = deep_merge_records(
            base.get(namespace).map(|m| m as &Map<_, _>),
            overlay.get(namespace).map(|m| m as &Map<_, _>),
        );
        result.insert(namespace.clone(), merged);
    }

    result
}

/// Deep-merge two extension maps; overlay wins on scalar conflicts.
#[must_use]
pub fn merge_extension_maps(base: &ExtensionMap, overlay: &ExtensionMap) -> ExtensionMap {
    merge_extension_maps_internal(base, overlay)
}

/// Merge maps for round-trip preserve: target wins on known keys; unknown namespaces/keys from source are retained.
#[must_use]
pub fn preserve_extension_maps(source: &ExtensionMap, target: &ExtensionMap) -> ExtensionMap {
    merge_extension_maps_internal(source, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map_from_json(value: serde_json::Value) -> ExtensionMap {
        value
            .as_object()
            .expect("extension map object")
            .iter()
            .map(|(key, value)| {
                let namespace = value
                    .as_object()
                    .expect("namespace object")
                    .iter()
                    .map(|(field, field_value)| (field.clone(), field_value.clone()))
                    .collect();
                (key.clone(), namespace)
            })
            .collect()
    }

    #[test]
    fn preserves_unknown_namespaces_from_both_inputs() {
        let base = map_from_json(json!({
            "nexus": { "world_id": "w1" },
            "creader": { "book_id": "b1" }
        }));
        let overlay = map_from_json(json!({
            "nexus": { "editor": "v2" }
        }));

        let result = merge_extension_maps(&base, &overlay);

        assert_eq!(
            result.get("nexus").unwrap().get("world_id").unwrap(),
            &json!("w1")
        );
        assert_eq!(
            result.get("nexus").unwrap().get("editor").unwrap(),
            &json!("v2")
        );
        assert_eq!(
            result.get("creader").unwrap().get("book_id").unwrap(),
            &json!("b1")
        );
    }

    #[test]
    fn overlay_wins_on_scalar_conflicts() {
        let base = map_from_json(json!({
            "nexus": { "mode": "draft", "keep": true }
        }));
        let overlay = map_from_json(json!({
            "nexus": { "mode": "published" }
        }));

        let result = merge_extension_maps(&base, &overlay);

        assert_eq!(result.get("nexus").unwrap().get("mode").unwrap(), &json!("published"));
        assert_eq!(result.get("nexus").unwrap().get("keep").unwrap(), &json!(true));
    }

    #[test]
    fn keeps_empty_namespace_objects() {
        let base = map_from_json(json!({ "nexus": {} }));
        let overlay = map_from_json(json!({ "creader": { "flag": true } }));

        let result = merge_extension_maps(&base, &overlay);

        assert_eq!(result.get("nexus").unwrap(), &Map::new());
        assert_eq!(
            result.get("creader").unwrap().get("flag").unwrap(),
            &json!(true)
        );
    }

    #[test]
    fn does_not_mutate_inputs() {
        let base = map_from_json(json!({ "nexus": { "a": 1 } }));
        let overlay = map_from_json(json!({ "nexus": { "b": 2 } }));
        let base_copy = base.clone();
        let overlay_copy = overlay.clone();

        let _ = merge_extension_maps(&base, &overlay);

        assert_eq!(base, base_copy);
        assert_eq!(overlay, overlay_copy);
    }

    #[test]
    fn does_not_alias_nested_objects_from_inputs() {
        let base = map_from_json(json!({ "nexus": { "nested": { "count": 1 } } }));
        let overlay = map_from_json(json!({ "nexus": { "tags": ["draft"] } }));

        let mut result = merge_extension_maps(&base, &overlay);

        result
            .get_mut("nexus")
            .unwrap()
            .get_mut("nested")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("count".into(), json!(99));
        result
            .get_mut("nexus")
            .unwrap()
            .get_mut("tags")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!("published"));

        assert_eq!(
            base.get("nexus").unwrap().get("nested").unwrap().get("count").unwrap(),
            &json!(1)
        );
        assert!(!base.get("nexus").unwrap().contains_key("tags"));
        assert_eq!(
            overlay.get("nexus").unwrap().get("tags").unwrap(),
            &json!(["draft"])
        );
    }

    #[test]
    fn preserve_retains_unknown_keys_from_source() {
        let source = map_from_json(json!({
            "nexus": { "legacy": "keep", "mode": "old" },
            "creader": { "only_source": true }
        }));
        let target = map_from_json(json!({
            "nexus": { "mode": "new" }
        }));

        let result = preserve_extension_maps(&source, &target);

        assert_eq!(
            result.get("nexus").unwrap(),
            &map_from_json(json!({ "nexus": { "legacy": "keep", "mode": "new" } }))
                .get("nexus")
                .unwrap()
                .clone()
        );
        assert_eq!(
            result.get("creader").unwrap().get("only_source").unwrap(),
            &json!(true)
        );
    }

    #[test]
    fn preserve_does_not_delete_sibling_namespaces() {
        let source = map_from_json(json!({
            "nexus": { "a": 1 },
            "creader": { "b": 2 }
        }));
        let target = map_from_json(json!({
            "nexus": { "c": 3 }
        }));

        let result = preserve_extension_maps(&source, &target);

        assert_eq!(
            result.get("nexus").unwrap(),
            &map_from_json(json!({ "nexus": { "a": 1, "c": 3 } }))
                .get("nexus")
                .unwrap()
                .clone()
        );
        assert_eq!(
            result.get("creader").unwrap().get("b").unwrap(),
            &json!(2)
        );
    }
}
