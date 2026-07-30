//! Extension map merge and round-trip preserve.

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Product extension namespace map (`ExtensionMap` wire shape).
pub type ExtensionMap = HashMap<String, Map<String, Value>>;

/// Cross-product functional-dialect module map (`ModuleMap` wire shape).
/// Namespace values are structured JSON — object or array — so they map to
/// `serde_json::Value` (mirrors the generated `ModuleMap` resolving to `Value`).
pub type ModuleMap = HashMap<String, Value>;

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

/// Merge two structured JSON values: deep-merge when both are plain objects;
/// otherwise the overlay replaces the base (arrays/scalars are not element-merged).
/// When the overlay is absent, the base is retained. Shared primitive for the
/// module namespace merge — the object deep-merge itself is `deep_merge_records`,
/// single-sourced with the extension helpers.
fn merge_json_values(base: Option<&Value>, overlay: Option<&Value>) -> Value {
    match (base, overlay) {
        (Some(b), Some(o)) if is_plain_object(b) && is_plain_object(o) => {
            Value::Object(deep_merge_records(b.as_object(), o.as_object()))
        }
        (_, Some(o)) => clone_value(o),
        (Some(b), None) => clone_value(b),
        // Unreachable: namespace keys come from the union of both maps.
        (None, None) => Value::Null,
    }
}

fn merge_module_maps_internal(base: &ModuleMap, overlay: &ModuleMap) -> ModuleMap {
    let namespaces: HashSet<&String> = base.keys().chain(overlay.keys()).collect();
    let mut result = ModuleMap::new();

    for namespace in namespaces {
        let merged = merge_json_values(base.get(namespace), overlay.get(namespace));
        result.insert(namespace.clone(), merged);
    }

    result
}

/// Deep-merge two module maps; object-valued namespaces are deep-merged while
/// array-valued namespaces are replaced by the overlay. Round-trip only — no
/// matching, activation, or scoring.
#[must_use]
pub fn merge_module_maps(base: &ModuleMap, overlay: &ModuleMap) -> ModuleMap {
    merge_module_maps_internal(base, overlay)
}

/// Merge module maps for round-trip preserve: target wins on known keys; unknown
/// namespaces/keys from source are retained. Round-trip only — no matching,
/// activation, or scoring.
#[must_use]
pub fn preserve_module_maps(source: &ModuleMap, target: &ModuleMap) -> ModuleMap {
    merge_module_maps_internal(source, target)
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

    fn module_map_from_json(value: serde_json::Value) -> ModuleMap {
        value
            .as_object()
            .expect("module map object")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
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

    #[test]
    fn module_deep_merges_object_valued_namespaces() {
        let base = module_map_from_json(json!({ "activation": { "state": "idle", "fuel": 10 } }));
        let overlay = module_map_from_json(json!({ "activation": { "state": "active" } }));

        let result = merge_module_maps(&base, &overlay);

        assert_eq!(
            result.get("activation").unwrap(),
            &json!({ "state": "active", "fuel": 10 })
        );
    }

    #[test]
    fn module_replaces_array_valued_namespaces() {
        let base = module_map_from_json(json!({ "placement": [{ "entry_id": "a", "position_hint": 0 }] }));
        let overlay = module_map_from_json(json!({ "placement": [{ "entry_id": "b", "position_hint": 1 }] }));

        let result = merge_module_maps(&base, &overlay);

        assert_eq!(
            result.get("placement").unwrap(),
            &json!([{ "entry_id": "b", "position_hint": 1 }])
        );
    }

    #[test]
    fn module_preserves_unknown_namespaces_object_and_array() {
        let base = module_map_from_json(json!({
            "activation": { "state": "idle" },
            "custom_obj": { "k": 1 }
        }));
        let overlay = module_map_from_json(json!({
            "placement": [{ "p": 1 }],
            "custom_arr": [9]
        }));

        let result = merge_module_maps(&base, &overlay);

        assert_eq!(result.get("activation").unwrap(), &json!({ "state": "idle" }));
        assert_eq!(result.get("custom_obj").unwrap(), &json!({ "k": 1 }));
        assert_eq!(result.get("placement").unwrap(), &json!([{ "p": 1 }]));
        assert_eq!(result.get("custom_arr").unwrap(), &json!([9]));
    }

    #[test]
    fn module_treats_empty_maps_and_namespaces_as_valid() {
        assert!(merge_module_maps(&ModuleMap::new(), &ModuleMap::new()).is_empty());

        let base = module_map_from_json(json!({ "activation": {} }));
        let overlay = module_map_from_json(json!({ "placement": [] }));

        let result = merge_module_maps(&base, &overlay);

        assert_eq!(result.get("activation").unwrap(), &json!({}));
        assert_eq!(result.get("placement").unwrap(), &json!([]));
    }

    #[test]
    fn module_does_not_alias_arrays_from_inputs() {
        let base = module_map_from_json(json!({ "placement": [{ "entry_id": "a" }] }));
        let overlay = module_map_from_json(json!({ "activation": { "state": "x" } }));

        let mut result = merge_module_maps(&base, &overlay);

        result
            .get_mut("placement")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!({ "entry_id": "z" }));

        assert_eq!(base.get("placement").unwrap(), &json!([{ "entry_id": "a" }]));
    }

    #[test]
    fn module_preserve_retains_unknown_namespaces_from_source() {
        let source = module_map_from_json(json!({
            "activation": { "legacy": true, "mode": "old" },
            "placement": [{ "entry_id": "old" }],
            "custom": { "only_source": 1 }
        }));
        let target = module_map_from_json(json!({ "activation": { "mode": "new" } }));

        let result = preserve_module_maps(&source, &target);

        assert_eq!(
            result.get("activation").unwrap(),
            &json!({ "legacy": true, "mode": "new" })
        );
        assert_eq!(
            result.get("placement").unwrap(),
            &json!([{ "entry_id": "old" }])
        );
        assert_eq!(result.get("custom").unwrap(), &json!({ "only_source": 1 }));
    }

    #[test]
    fn module_preserve_does_not_delete_sibling_namespaces() {
        let source = module_map_from_json(json!({
            "activation": { "a": 1 },
            "placement": [{ "p": 1 }]
        }));
        let target = module_map_from_json(json!({ "activation": { "c": 3 } }));

        let result = preserve_module_maps(&source, &target);

        assert_eq!(result.get("activation").unwrap(), &json!({ "a": 1, "c": 3 }));
        assert_eq!(result.get("placement").unwrap(), &json!([{ "p": 1 }]));
    }

    #[test]
    fn module_preserve_lets_target_replace_array_namespace() {
        let source = module_map_from_json(json!({ "placement": [{ "entry_id": "old" }] }));
        let target = module_map_from_json(json!({ "placement": [{ "entry_id": "new" }] }));

        let result = preserve_module_maps(&source, &target);

        assert_eq!(
            result.get("placement").unwrap(),
            &json!([{ "entry_id": "new" }])
        );
    }
}
