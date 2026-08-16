//! Integration tests for `orchestrate_invoke_tool`.
//!
//! Parity with `packages/spoke-operations/tests/tools/orchestrate.test.ts`.
//! The Rust `&dyn ToolInvokePort` parameter makes port absence
//! unrepresentable at the type level; the `MissingToolInvokePort` negative
//! double pins the same `CAPABILITY_PORT_MISSING` reject the TS runtime
//! guard produces (demonstrating code parity).

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use spoke_operations::{
    orchestrate_invoke_tool, spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult,
    ToolInvokePort, ToolInvokeRequest, ToolInvokeResponse,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn make_request(capability_id: &str, arguments: Value) -> ToolInvokeRequest {
    ToolInvokeRequest {
        capability_id: capability_id.to_string(),
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
fn returns_ports_response_for_valid_capability_id() {
    let port = EchoToolInvokePort;

    let result = pollster::block_on(orchestrate_invoke_tool(
        &port,
        make_request("tools.tool_demo.lookup", json!({ "query": "ke-1" })),
    ));

    match result {
        SpokeResult::Ok(response) => {
            assert_eq!(response.result, json!({ "echo": "tools.tool_demo.lookup" }));
        }
        SpokeResult::Reject(reject) => panic!("expected ok, got: {}", reject.message),
    }
}

#[test]
fn passes_through_port_reject_result_as_is() {
    struct ExplodingPort;

    #[async_trait]
    impl ToolInvokePort for ExplodingPort {
        async fn invoke_tool(&self, _request: ToolInvokeRequest) -> SpokeResult<ToolInvokeResponse> {
            spoke_reject(SpokeRejectCode::InternalError, "tool exploded", None)
        }
    }

    let result = pollster::block_on(orchestrate_invoke_tool(
        &ExplodingPort,
        make_request("tools.tool_demo.lookup", json!({})),
    ));

    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject.message, "tool exploded");
        }
        SpokeResult::Ok(_) => panic!("expected reject"),
    }
}

#[test]
fn rejects_bad_request_grammar_before_touching_port() {
    struct SpyPort {
        invoked: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ToolInvokePort for SpyPort {
        async fn invoke_tool(&self, _request: ToolInvokeRequest) -> SpokeResult<ToolInvokeResponse> {
            self.invoked.store(true, Ordering::SeqCst);
            spoke_ok(ToolInvokeResponse { result: Value::Null })
        }
    }

    let invoked = Arc::new(AtomicBool::new(false));
    let port = SpyPort {
        invoked: Arc::clone(&invoked),
    };

    let result = pollster::block_on(orchestrate_invoke_tool(
        &port,
        make_request("tools.Bad.lookup", json!({})),
    ));

    // Grammar gate runs first: the port must never be reached.
    assert!(!invoked.load(Ordering::SeqCst));
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            let details = reject.details.expect("details present");
            assert_eq!(
                details.get("capability_id").and_then(Value::as_str),
                Some("tools.Bad.lookup")
            );
        }
        SpokeResult::Ok(_) => panic!("expected reject"),
    }
}

#[test]
fn missing_port_double_rejects_capability_port_missing_with_per_tool_blame() {
    // Rust `&dyn ToolInvokePort` makes absence unrepresentable at the type
    // level; the MissingToolInvokePort double returns the same reject the TS
    // runtime guard produces, pinning code parity for the per-tool blame.
    struct MissingToolInvokePort;

    #[async_trait]
    impl ToolInvokePort for MissingToolInvokePort {
        async fn invoke_tool(&self, request: ToolInvokeRequest) -> SpokeResult<ToolInvokeResponse> {
            let mut details = Map::new();
            details.insert("capability".into(), Value::String(request.capability_id));
            spoke_reject(
                SpokeRejectCode::CapabilityPortMissing,
                "no port",
                Some(details),
            )
        }
    }

    let result = pollster::block_on(orchestrate_invoke_tool(
        &MissingToolInvokePort,
        make_request("tools.tool_demo.rank", json!({})),
    ));

    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            let details = reject.details.expect("details present");
            assert_eq!(
                details.get("capability").and_then(Value::as_str),
                Some("tools.tool_demo.rank")
            );
        }
        SpokeResult::Ok(_) => panic!("expected reject"),
    }
}

#[test]
fn does_not_revalidate_arguments_grammar_only() {
    let port = EchoToolInvokePort;

    // Arguments that would fail the structural argument gate still reach the
    // port — argument validation is the caller's job
    // (validate_tool_arguments).
    let result = pollster::block_on(orchestrate_invoke_tool(
        &port,
        make_request("tools.tool_demo.lookup", Value::Null),
    ));

    assert!(result.is_ok());
}
