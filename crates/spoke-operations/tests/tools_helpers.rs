//! Integration tests for the pure tool manifest and ABI helpers.
//!
//! Parity with `packages/spoke-operations/tests/tools/helpers.test.ts`.
//! Divergences (documented in `src/tools/helpers.rs`): the TS runtime guards
//! for capability_id/op pattern and input/output object-ness are
//! type-guaranteed in Rust (pattern-validated newtypes, `serde_json::Map`),
//! so those negative cases have no Rust mirror.

use serde_json::{json, Map, Value};
use spoke_operations::{
    find_tool, list_tools, parse_tool_capability_id, tool_capability_id,
    validate_manifest_tools, validate_tool_arguments, validate_tool_descriptor,
    SpokeRejectCode, SpokeResult, ToolCapabilityId,
};
use spoke_schemas::host_capability_manifest::ToolDescriptor;
use spoke_schemas::HostCapabilityManifest;
use std::collections::HashMap;
use std::num::NonZeroU64;

fn make_descriptor(capability_id: &str, op: &str) -> ToolDescriptor {
    ToolDescriptor {
        schema_version: NonZeroU64::new(1).unwrap(),
        capability_id: capability_id.parse().unwrap(),
        op: op.parse().unwrap(),
        description: "Look up a knowledge entry by id.".parse().unwrap(),
        input: Map::new(),
        output: Map::new(),
        idempotent: false,
    }
}

fn make_manifest(
    tools: Vec<ToolDescriptor>,
    namespaces: &[&str],
    capabilities: &[&str],
) -> HostCapabilityManifest {
    HostCapabilityManifest {
        authority: None,
        capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
        extensions: HashMap::new(),
        host_id: "host-1".parse().unwrap(),
        namespaces: namespaces.iter().map(|s| s.parse().unwrap()).collect(),
        roles: vec!["data-store".to_string()],
        schema_version: NonZeroU64::new(1).unwrap(),
        tools,
    }
}

fn reject_details(result: SpokeResult<()>) -> (SpokeRejectCode, Map<String, Value>) {
    match result {
        SpokeResult::Reject(reject) => (
            reject.code,
            reject.details.unwrap_or_else(|| {
                panic!("expected structured details, got message: {}", reject.message)
            }),
        ),
        SpokeResult::Ok(_) => panic!("expected reject, got ok"),
    }
}

fn detail_str<'a>(details: &'a Map<String, Value>, key: &str) -> &'a str {
    details
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string detail {key:?}"))
}

#[test]
fn composes_namespace_and_tool_id() {
    assert_eq!(tool_capability_id("tool_demo", "lookup"), "tools.tool_demo.lookup");
}

#[test]
fn accepts_digits_and_separators_in_both_segments() {
    assert_eq!(tool_capability_id("ns_2", "tool-1"), "tools.ns_2.tool-1");
    assert_eq!(tool_capability_id("ns", "0abc"), "tools.ns.0abc");
}

#[test]
#[should_panic(expected = "invalid tool capability grammar")]
fn panics_on_invalid_namespace_grammar() {
    tool_capability_id("Tool Demo", "lookup");
}

#[test]
#[should_panic(expected = "invalid tool capability grammar")]
fn panics_on_invalid_tool_id_grammar() {
    tool_capability_id("tool_demo", "-tool");
}

#[test]
fn parses_valid_capability_id() {
    let parsed = parse_tool_capability_id("tools.tool_demo.lookup");
    match parsed {
        SpokeResult::Ok(ToolCapabilityId { namespace, tool_id }) => {
            assert_eq!(namespace, "tool_demo");
            assert_eq!(tool_id, "lookup");
        }
        SpokeResult::Reject(reject) => panic!("unexpected reject: {}", reject.message),
    }
}

#[test]
fn rejects_trailing_line_terminator() {
    // Schema-pattern identity (F-007): the pattern string is byte-identical
    // to the schema. ECMA-262 `$` without the `m` flag anchors at the
    // absolute end of input — a trailing line terminator does NOT match
    // (unlike PCRE/Python `re`; verified against V8 and regress). "Fixing"
    // this to accept trailing terminators would diverge from the schema
    // pattern, which is forbidden.
    assert!(parse_tool_capability_id("tools.tool_demo.lookup\n").is_reject());
    assert!(parse_tool_capability_id("tools.tool_demo.lookup\r").is_reject());
    assert!(parse_tool_capability_id("tools.tool_demo.lookup\r\n").is_reject());
}

#[test]
fn rejects_id_without_tools_prefix() {
    let result = parse_tool_capability_id("nottools.tool_demo.lookup");
    assert!(result.is_reject());
    if let SpokeResult::Reject(reject) = result {
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        let details = reject.details.unwrap();
        assert_eq!(detail_str(&details, "capability_id"), "nottools.tool_demo.lookup");
    }
}

#[test]
fn rejects_bad_grammar() {
    for bad in [
        "tools.Tool_demo.lookup",
        "tools..lookup",
        "tools.tool_demo.",
        "tools.tool_demo.too long",
        "tools.9ns.lookup",
        "",
    ] {
        let result = parse_tool_capability_id(bad);
        assert!(result.is_reject(), "expected reject for {bad:?}");
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }
}

#[test]
fn type_layer_enforces_pattern_and_objectness_guarantees() {
    // Parity documentation for the TS-only runtime guards in
    // validateToolDescriptor: in Rust the generated types enforce the schema
    // pattern (newtype FromStr) and input/output object-ness
    // (serde_json::Map) at construction/deserialization time, so a
    // ToolDescriptor with a bad pattern or a non-object ABI cannot exist to
    // reach the helper.
    use spoke_schemas::host_capability_manifest::ToolDescriptorCapabilityId;

    // Pattern is enforced when the newtype is parsed.
    assert!("tools.Bad.lookup".parse::<ToolDescriptorCapabilityId>().is_err());
    assert!("tools.tool_demo.lookup"
        .parse::<ToolDescriptorCapabilityId>()
        .is_ok());

    // Non-object input is rejected at deserialization (input MUST be a JSON
    // object subschema, per the wire contract §2).
    let bad_abi = json!({
        "schema_version": 1,
        "capability_id": "tools.tool_demo.lookup",
        "op": "tools.tool_demo.lookup",
        "description": "x",
        "input": "not-an-object",
        "output": {}
    });
    assert!(serde_json::from_value::<ToolDescriptor>(bad_abi).is_err());
}

#[test]
fn validates_descriptor_with_unconstrained_abi() {
    assert!(validate_tool_descriptor(&make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup")).is_ok());
}

#[test]
fn validates_descriptor_with_object_typed_subschemas() {
    let mut descriptor = make_descriptor("tools.tool_demo.rank", "tools.tool_demo.rank");
    descriptor.input = serde_json::from_value(json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"]
    }))
    .unwrap();
    descriptor.output = serde_json::from_value(json!({
        "type": "object",
        "properties": { "ranked_ids": { "type": "array", "items": { "type": "string" } } },
        "required": ["ranked_ids"]
    }))
    .unwrap();
    descriptor.idempotent = true;
    assert!(validate_tool_descriptor(&descriptor).is_ok());
}

#[test]
fn rejects_op_mismatching_capability_id() {
    let descriptor = make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.rank");
    let result = validate_tool_descriptor(&descriptor);
    assert!(result.is_reject());
    if let SpokeResult::Reject(reject) = result {
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        let details = reject.details.unwrap();
        assert_eq!(detail_str(&details, "field"), "op");
        assert_eq!(detail_str(&details, "op"), "tools.tool_demo.rank");
        assert_eq!(detail_str(&details, "capability_id"), "tools.tool_demo.lookup");
    }
}

#[test]
fn manifest_accepts_well_formed_tools() {
    let manifest = make_manifest(
        vec![make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup")],
        &["tool_demo"],
        &["spoke-baseline", "tools.tool_demo.lookup"],
    );
    assert!(validate_manifest_tools(&manifest).is_ok());
}

#[test]
fn manifest_accepts_without_tools() {
    let manifest = make_manifest(vec![], &["tool_demo"], &["spoke-baseline"]);
    assert!(validate_manifest_tools(&manifest).is_ok());
}

#[test]
fn manifest_rejects_descriptor_invalid_tool_with_field_and_index() {
    let manifest = make_manifest(
        vec![make_descriptor("tools.tool_demo.lookup", "tools.other.rank")],
        &["tool_demo"],
        &["spoke-baseline", "tools.tool_demo.lookup"],
    );
    let (code, details) = reject_details(validate_manifest_tools(&manifest));
    assert_eq!(code, SpokeRejectCode::InvalidInput);
    assert_eq!(detail_str(&details, "field"), "tools");
    assert_eq!(details.get("index").and_then(Value::as_u64), Some(0));
    assert_eq!(detail_str(&details, "capability_id"), "tools.tool_demo.lookup");
}

#[test]
fn manifest_rejects_capability_id_missing_from_capabilities() {
    let manifest = make_manifest(
        vec![make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup")],
        &["tool_demo"],
        &["spoke-baseline"],
    );
    let (code, details) = reject_details(validate_manifest_tools(&manifest));
    assert_eq!(code, SpokeRejectCode::InvalidInput);
    assert_eq!(detail_str(&details, "field"), "tools");
    assert_eq!(details.get("index").and_then(Value::as_u64), Some(0));
    assert_eq!(detail_str(&details, "capability_id"), "tools.tool_demo.lookup");
}

#[test]
fn manifest_rejects_unowned_namespace() {
    let manifest = make_manifest(
        vec![make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup")],
        &["other_ns"],
        &["spoke-baseline", "tools.tool_demo.lookup"],
    );
    let (code, details) = reject_details(validate_manifest_tools(&manifest));
    assert_eq!(code, SpokeRejectCode::InvalidInput);
    assert_eq!(detail_str(&details, "field"), "tools");
    assert_eq!(details.get("index").and_then(Value::as_u64), Some(0));
    assert_eq!(detail_str(&details, "namespace"), "tool_demo");
    assert_eq!(detail_str(&details, "capability_id"), "tools.tool_demo.lookup");
}

#[test]
fn manifest_rejects_duplicate_capability_ids() {
    let manifest = make_manifest(
        vec![
            make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup"),
            make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup"),
        ],
        &["tool_demo"],
        &["spoke-baseline", "tools.tool_demo.lookup"],
    );
    let (code, details) = reject_details(validate_manifest_tools(&manifest));
    assert_eq!(code, SpokeRejectCode::InvalidInput);
    assert_eq!(detail_str(&details, "field"), "tools");
    assert_eq!(details.get("index").and_then(Value::as_u64), Some(1));
    assert_eq!(detail_str(&details, "capability_id"), "tools.tool_demo.lookup");
    assert_eq!(details.get("duplicate_of").and_then(Value::as_u64), Some(0));
}

#[test]
fn arguments_pass_when_input_unconstrained() {
    let descriptor = make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup");
    assert!(validate_tool_arguments(&descriptor, &json!({})).is_ok());
    assert!(validate_tool_arguments(&descriptor, &json!({ "any": "thing" })).is_ok());
}

#[test]
fn arguments_reject_non_object() {
    let descriptor = make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup");
    for bad in [json!(null), json!(42), json!("nope"), json!(["a"])] {
        let (code, details) = reject_details(validate_tool_arguments(&descriptor, &bad));
        assert_eq!(code, SpokeRejectCode::InvalidInput);
        assert_eq!(detail_str(&details, "field"), "arguments");
    }
}

#[test]
fn arguments_accept_satisfied_required_keys() {
    let mut descriptor = make_descriptor("tools.tool_demo.rank", "tools.tool_demo.rank");
    descriptor.input = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "limit": { "type": "integer" }
        },
        "required": ["query"]
    }))
    .unwrap();
    assert!(validate_tool_arguments(&descriptor, &json!({ "query": "x", "limit": 3 })).is_ok());
    assert!(validate_tool_arguments(&descriptor, &json!({ "query": "x" })).is_ok());
}

#[test]
fn arguments_reject_missing_required_keys() {
    let mut descriptor = make_descriptor("tools.tool_demo.rank", "tools.tool_demo.rank");
    descriptor.input = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "limit": { "type": "integer" }
        },
        "required": ["query", "limit"]
    }))
    .unwrap();

    let (code, details) = reject_details(validate_tool_arguments(&descriptor, &json!({ "query": "x" })));
    assert_eq!(code, SpokeRejectCode::InvalidInput);
    assert_eq!(detail_str(&details, "field"), "arguments");
    let missing = details.get("missing").and_then(Value::as_array).unwrap();
    assert_eq!(missing, &vec![json!("limit")]);

    let (_, all_missing) = reject_details(validate_tool_arguments(&descriptor, &json!({})));
    let missing_all = all_missing.get("missing").and_then(Value::as_array).unwrap();
    assert_eq!(missing_all, &vec![json!("query"), json!("limit")]);
}

#[test]
fn arguments_skip_gate_when_input_type_not_object() {
    let mut descriptor = make_descriptor("tools.tool_demo.rank", "tools.tool_demo.rank");
    descriptor.input = serde_json::from_value(json!({ "type": "string" })).unwrap();
    assert!(validate_tool_arguments(&descriptor, &json!({})).is_ok());
}

#[test]
fn arguments_skip_gate_when_required_not_array() {
    let mut descriptor = make_descriptor("tools.tool_demo.rank", "tools.tool_demo.rank");
    descriptor.input = serde_json::from_value(json!({ "type": "object", "required": "query" })).unwrap();
    assert!(validate_tool_arguments(&descriptor, &json!({})).is_ok());
}

#[test]
fn list_tools_returns_manifest_order() {
    let tools = vec![
        make_descriptor("tools.tool_demo.alpha", "tools.tool_demo.alpha"),
        make_descriptor("tools.tool_demo.beta", "tools.tool_demo.beta"),
    ];
    let manifest = make_manifest(tools.clone(), &["tool_demo"], &["spoke-baseline"]);
    let listed = list_tools(&manifest);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].capability_id.as_str(), "tools.tool_demo.alpha");
    assert_eq!(listed[1].capability_id.as_str(), "tools.tool_demo.beta");
}

#[test]
fn list_tools_empty_when_absent() {
    let manifest = make_manifest(vec![], &["tool_demo"], &["spoke-baseline"]);
    assert!(list_tools(&manifest).is_empty());
}

#[test]
fn find_tool_returns_matching_descriptor_with_abi_intact() {
    let mut descriptor = make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup");
    descriptor.input = serde_json::from_value(json!({ "type": "object", "required": ["query"] })).unwrap();
    descriptor.output = Map::new();
    let manifest = make_manifest(vec![descriptor.clone()], &["tool_demo"], &["spoke-baseline"]);
    let found = find_tool(&manifest, "tools.tool_demo.lookup");
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.capability_id.as_str(), descriptor.capability_id.as_str());
    assert_eq!(found.op.as_str(), descriptor.op.as_str());
    assert_eq!(found.input, descriptor.input);
    assert_eq!(found.output, descriptor.output);
}

#[test]
fn find_tool_returns_none_when_no_match() {
    let manifest = make_manifest(
        vec![make_descriptor("tools.tool_demo.lookup", "tools.tool_demo.lookup")],
        &["tool_demo"],
        &["spoke-baseline"],
    );
    assert!(find_tool(&manifest, "tools.other.nope").is_none());
}
