//! Pure tool manifest and ABI helpers (parity with
//! `packages/spoke-operations/src/tools/helpers.ts`).
//!
//! Purity: no I/O, storage, LLM, HTTP, MCP, or JSON-Schema-validator
//! dependency — the descriptor's `input`/`output` are data, not executed.
//!
//! Type-level guarantees (documented divergence from TS): the generated
//! `ToolDescriptorCapabilityId` / `ToolDescriptorOp` newtypes validate the
//! schema pattern in `FromStr` (and serde deserializes through `FromStr`),
//! and `input`/`output` are `serde_json::Map` (always a JSON object). The TS
//! runtime guards for pattern and object-ness are therefore unrepresentable
//! here; the only runtime-checkable cross-field rule is `op === capability_id`.

use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{Map, Value};
use spoke_schemas::host_capability_manifest::ToolDescriptor;
use spoke_schemas::HostCapabilityManifest;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Tool capability grammar: `tools.<ns>.<tool_id>`.
///
/// The pattern string is byte-identical to the schema pattern in
/// `schemas/data/tool-descriptor.schema.json`. It is compiled with `regress`,
/// an ECMA-262 engine, so it inherits native ECMA-262 semantics: `$` without
/// the `m` flag anchors at the absolute end of input — a trailing line
/// terminator does NOT match (unlike PCRE/Python `re`; verified against V8
/// and regress). Do not diverge from the schema pattern string in either
/// direction.
const TOOL_CAPABILITY_PATTERN: &str = r"^tools\.[a-z][a-z0-9_-]*\.[a-z0-9][a-z0-9_-]*$";

static TOOL_CAPABILITY_REGEX: LazyLock<regress::Regex> = LazyLock::new(|| {
    regress::Regex::new(TOOL_CAPABILITY_PATTERN)
        .expect("tool capability pattern is a valid ECMA-262 regex")
});

/// Parsed tool capability id (`tools.<ns>.<tool_id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCapabilityId {
    pub namespace: String,
    pub tool_id: String,
}

/// Compose a tool capability id `tools.<ns>.<tool_id>` and assert its
/// grammar. Panics on bad grammar — programmer misuse, consistent with
/// generated-type ergonomics. Never returns an id that
/// `parse_tool_capability_id` would reject.
pub fn tool_capability_id(namespace: &str, tool_id: &str) -> String {
    let id = format!("tools.{namespace}.{tool_id}");
    if TOOL_CAPABILITY_REGEX.find(&id).is_none() {
        panic!(
            "invalid tool capability grammar: {id:?} (expected tools.<ns>.<tool_id> with ns ^[a-z][a-z0-9_-]*$ and tool_id ^[a-z0-9][a-z0-9_-]*$)"
        );
    }
    id
}

/// Parse a tool capability id into its namespace and tool id.
///
/// Rejects with `INVALID_INPUT` for a missing `tools.` prefix and for bad
/// grammar. The pattern is the schema pattern (see `TOOL_CAPABILITY_PATTERN`),
/// so `$` is a strict end-of-input anchor (no trailing line terminator).
pub fn parse_tool_capability_id(id: &str) -> SpokeResult<ToolCapabilityId> {
    if !id.starts_with("tools.") {
        let mut details = Map::new();
        details.insert("capability_id".into(), Value::String(id.to_string()));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("Tool capability id must start with \"tools.\" prefix: {id:?}"),
            Some(details),
        );
    }
    let Some(found) = TOOL_CAPABILITY_REGEX.find(id) else {
        let mut details = Map::new();
        details.insert("capability_id".into(), Value::String(id.to_string()));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!(
                "Invalid tool capability grammar: {id:?} (expected tools.<ns>.<tool_id> with ns ^[a-z][a-z0-9_-]*$ and tool_id ^[a-z0-9][a-z0-9_-]*$)"
            ),
            Some(details),
        );
    };
    // The pattern guarantees exactly three dot-separated segments.
    let matched = &id[found.start()..found.end()];
    let mut parts = matched.split('.');
    let _prefix = parts.next();
    let namespace = parts
        .next()
        .expect("matched grammar guarantees a namespace segment")
        .to_string();
    let tool_id = parts
        .next()
        .expect("matched grammar guarantees a tool id segment")
        .to_string();
    spoke_ok(ToolCapabilityId { namespace, tool_id })
}

/// Validate a `ToolDescriptor` with `INVALID_INPUT`.
///
/// Pattern and `input`/`output` object-ness are type-guaranteed by the
/// generated types (pattern-validated newtypes, `serde_json::Map`); the
/// runtime-checkable rule is `op === capability_id`.
pub fn validate_tool_descriptor(descriptor: &ToolDescriptor) -> SpokeResult<()> {
    if descriptor.op.as_str() != descriptor.capability_id.as_str() {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("op".into()));
        details.insert("op".into(), Value::String(descriptor.op.to_string()));
        details.insert(
            "capability_id".into(),
            Value::String(descriptor.capability_id.to_string()),
        );
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!(
                "ToolDescriptor op must equal capability_id: op=\"{}\" capability_id=\"{}\"",
                descriptor.op.as_str(),
                descriptor.capability_id.as_str()
            ),
            Some(details),
        );
    }
    spoke_ok_unit()
}

/// Validate every tool in a manifest: each descriptor is valid, its
/// `capability_id` appears in `capabilities[]`, its derived namespace is
/// owned (`namespaces[]` membership), and capability ids are unique within
/// `tools[]`. Rejects with `INVALID_INPUT` and structured `details`
/// (`field: "tools"` + `index`).
pub fn validate_manifest_tools(manifest: &HostCapabilityManifest) -> SpokeResult<()> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (index, descriptor) in manifest.tools.iter().enumerate() {
        if let SpokeResult::Reject(reject) = validate_tool_descriptor(descriptor) {
            let mut details = Map::new();
            details.insert("field".into(), Value::String("tools".into()));
            details.insert("index".into(), Value::from(index));
            details.insert(
                "capability_id".into(),
                Value::String(descriptor.capability_id.to_string()),
            );
            return spoke_reject(SpokeRejectCode::InvalidInput, reject.message, Some(details));
        }
        let capability_id = descriptor.capability_id.as_str();
        if !manifest.capabilities.iter().any(|cap| cap.as_str() == capability_id) {
            let mut details = Map::new();
            details.insert("field".into(), Value::String("tools".into()));
            details.insert("index".into(), Value::from(index));
            details.insert("capability_id".into(), Value::String(capability_id.to_string()));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!(
                    "Tool capability \"{capability_id}\" at tools[{index}] is missing from manifest capabilities[]"
                ),
                Some(details),
            );
        }
        // validate_tool_descriptor passed, so the grammar guarantees exactly
        // three dot-separated segments; the split below is total.
        let namespace = descriptor
            .capability_id
            .split('.')
            .nth(1)
            .expect("descriptor grammar was pre-validated");
        if !manifest.namespaces.iter().any(|ns| ns.as_str() == namespace) {
            let mut details = Map::new();
            details.insert("field".into(), Value::String("tools".into()));
            details.insert("index".into(), Value::from(index));
            details.insert("namespace".into(), Value::String(namespace.to_string()));
            details.insert("capability_id".into(), Value::String(capability_id.to_string()));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!(
                    "Tool namespace \"{namespace}\" at tools[{index}] is not owned by this manifest (namespaces[])"
                ),
                Some(details),
            );
        }
        if let Some(&first) = seen.get(capability_id) {
            let mut details = Map::new();
            details.insert("field".into(), Value::String("tools".into()));
            details.insert("index".into(), Value::from(index));
            details.insert("capability_id".into(), Value::String(capability_id.to_string()));
            details.insert("duplicate_of".into(), Value::from(first));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!(
                    "Duplicate tool capability_id \"{capability_id}\" at tools[{index}] (first declared at tools[{first}])"
                ),
                Some(details),
            );
        }
        seen.insert(capability_id, index);
    }
    spoke_ok_unit()
}

/// Structural argument gate (frozen granularity): `arguments` must be a JSON
/// object; when `descriptor.input` declares top-level `"type": "object"` with
/// `"required": [...]`, each listed string key must be present in
/// `arguments`. No deeper JSON-Schema checking — full validation stays
/// consumer/fixture-side. `input: {}` ⇒ vacuous (unconstrained).
pub fn validate_tool_arguments(
    descriptor: &ToolDescriptor,
    arguments: &Value,
) -> SpokeResult<()> {
    let Some(object) = arguments.as_object() else {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("arguments".into()));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "Tool arguments must be a JSON object",
            Some(details),
        );
    };
    let input = &descriptor.input;
    if input.get("type") != Some(&Value::String("object".to_string())) {
        return spoke_ok_unit();
    }
    let Some(Value::Array(required)) = input.get("required") else {
        return spoke_ok_unit();
    };
    let missing: Vec<&str> = required
        .iter()
        .filter_map(Value::as_str)
        .filter(|key| !object.contains_key(*key))
        .collect();
    if missing.is_empty() {
        return spoke_ok_unit();
    }
    let mut details = Map::new();
    details.insert("field".into(), Value::String("arguments".into()));
    details.insert("missing".into(), Value::from(missing.clone()));
    spoke_reject(
        SpokeRejectCode::InvalidInput,
        format!("Missing required tool arguments: {}", missing.join(", ")),
        Some(details),
    )
}

/// List the manifest's tools in declaration order (empty when `tools` absent).
pub fn list_tools(manifest: &HostCapabilityManifest) -> Vec<ToolDescriptor> {
    manifest.tools.clone()
}

/// Find the tool descriptor whose `capability_id` exactly matches, or `None`.
pub fn find_tool(
    manifest: &HostCapabilityManifest,
    capability_id: &str,
) -> Option<ToolDescriptor> {
    manifest
        .tools
        .iter()
        .find(|descriptor| descriptor.capability_id.as_str() == capability_id)
        .cloned()
}
