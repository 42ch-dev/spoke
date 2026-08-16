//! Integration tests for the optional `ToolInvokePort` family.
//!
//! Parity with `packages/spoke-operations/tests/tools/port.test.ts`.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use spoke_operations::{
    spoke_ok, spoke_reject, validate_tool_arguments, SpokeRejectCode, SpokeResult,
    ToolInvokePort, ToolInvokeRequest, ToolInvokeResponse,
};
use spoke_schemas::host_capability_manifest::ToolDescriptor;
use std::num::NonZeroU64;

fn make_descriptor(input: Value) -> ToolDescriptor {
    ToolDescriptor {
        schema_version: NonZeroU64::new(1).unwrap(),
        capability_id: "tools.tool_demo.lookup".parse().unwrap(),
        op: "tools.tool_demo.lookup".parse().unwrap(),
        description: "Look up a knowledge entry by id.".parse().unwrap(),
        input: input.as_object().cloned().unwrap_or_default(),
        output: Map::new(),
        idempotent: false,
    }
}

fn make_request(arguments: Value) -> ToolInvokeRequest {
    ToolInvokeRequest {
        capability_id: "tools.tool_demo.lookup".to_string(),
        arguments,
    }
}

struct EchoToolInvokePort;

#[async_trait]
impl ToolInvokePort for EchoToolInvokePort {
    async fn invoke_tool(&self, request: ToolInvokeRequest) -> SpokeResult<ToolInvokeResponse> {
        spoke_ok(ToolInvokeResponse {
            result: json!({ "echo": request.capability_id }),
        })
    }
}

#[test]
fn request_carries_capability_id_and_arguments() {
    let request = make_request(json!({ "query": "ke-1", "limit": 10 }));

    assert_eq!(request.capability_id, "tools.tool_demo.lookup");
    assert_eq!(request.arguments, json!({ "query": "ke-1", "limit": 10 }));
}

#[test]
fn response_carries_opaque_result_value() {
    let response = ToolInvokeResponse {
        result: json!({ "entry": { "id": "ke-1" }, "tags": ["a", "b"], "ok": true }),
    };

    assert_eq!(
        response.result,
        json!({ "entry": { "id": "ke-1" }, "tags": ["a", "b"], "ok": true })
    );
}

#[test]
fn invokes_mock_port_and_returns_its_response() {
    let port = EchoToolInvokePort;

    let result = pollster::block_on(port.invoke_tool(make_request(json!({ "query": "ke-1" }))));

    match result {
        SpokeResult::Ok(response) => {
            assert_eq!(response.result, json!({ "echo": "tools.tool_demo.lookup" }));
        }
        SpokeResult::Reject(_) => panic!("expected ok"),
    }
}

#[test]
fn passes_through_reject_result_as_is() {
    struct MissingToolInvokePort;

    #[async_trait]
    impl ToolInvokePort for MissingToolInvokePort {
        async fn invoke_tool(&self, _request: ToolInvokeRequest) -> SpokeResult<ToolInvokeResponse> {
            let mut details = Map::new();
            details.insert("capability".into(), Value::String("tools.tool_demo.lookup".into()));
            spoke_reject(
                SpokeRejectCode::CapabilityPortMissing,
                "no port",
                Some(details),
            )
        }
    }

    let port = MissingToolInvokePort;

    let result = pollster::block_on(port.invoke_tool(make_request(json!({}))));

    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            let details = reject.details.expect("details present");
            assert_eq!(
                details.get("capability").and_then(Value::as_str),
                Some("tools.tool_demo.lookup")
            );
        }
        SpokeResult::Ok(_) => panic!("expected reject"),
    }
}

#[test]
fn does_not_revalidate_arguments_inside_port() {
    let port = EchoToolInvokePort;

    // Direct port invocation with arguments that would fail the structural
    // gate still reaches the port — validation happens before, not inside.
    let result = pollster::block_on(port.invoke_tool(make_request(json!({}))));

    assert!(result.is_ok());
}

#[test]
fn argument_gate_admits_well_shaped_arguments_before_invoking() {
    let descriptor = make_descriptor(json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"]
    }));
    let request = make_request(json!({ "query": "ke-1" }));

    let gate = validate_tool_arguments(&descriptor, &request.arguments);
    assert!(gate.is_ok());

    let result = pollster::block_on(EchoToolInvokePort.invoke_tool(request));
    assert!(result.is_ok());
}

#[test]
fn argument_gate_rejects_missing_required_keys() {
    let descriptor = make_descriptor(json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"]
    }));
    let request = make_request(json!({}));

    let gate = validate_tool_arguments(&descriptor, &request.arguments);

    match gate {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            let details = reject.details.expect("details present");
            assert_eq!(details.get("field").and_then(Value::as_str), Some("arguments"));
            assert_eq!(
                details.get("missing").and_then(Value::as_array),
                Some(&vec![Value::String("query".into())])
            );
        }
        SpokeResult::Ok(_) => panic!("expected reject"),
    }
}

#[test]
fn argument_gate_rejects_non_object_arguments() {
    let descriptor = make_descriptor(json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"]
    }));

    for bad in [Value::Null, Value::from(42), Value::String("nope".into()), json!(["a"])] {
        let gate = validate_tool_arguments(&descriptor, &bad);

        match gate {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
                let details = reject.details.expect("details present");
                assert_eq!(details.get("field").and_then(Value::as_str), Some("arguments"));
            }
            SpokeResult::Ok(_) => panic!("expected reject"),
        }
    }
}
