//! Optional `ToolInvokePort` family — wire-adjacent invoke request/response
//! types and the async port contract for injecting remote tool invocation.
//!
//! The family is standalone: it is NOT folded into `BaselinePorts`, and no
//! `ToolsPort` / `ToolsAdapter` composed alias exists (capability gating is
//! per-tool — the capability string itself — not per-composed-type).
//!
//! Purity: the library stays I/O-free; port methods return awaitable results
//! (Send futures) and the library only awaits injected ports. The port itself
//! does NOT re-validate request arguments — callers run the structural
//! argument gate ([`crate::validate_tool_arguments`]) before invoking.

use crate::result::SpokeResult;
use async_trait::async_trait;
use serde_json::Value;

/// Request to invoke a remote tool by its `tools.<ns>.<tool_id>` capability id.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvokeRequest {
    /// Tool capability string `tools.<ns>.<tool_id>`.
    pub capability_id: String,
    /// Opaque JSON arguments object for the tool.
    pub arguments: Value,
}

/// Successful tool invocation result — an opaque JSON value.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvokeResponse {
    /// Opaque JSON result value produced by the tool.
    pub result: Value,
}

/// Optional port family for invoking remote tools by capability id.
///
/// The structural argument gate is a caller-side step
/// ([`crate::validate_tool_arguments`]); the port itself does not
/// re-validate.
#[async_trait]
pub trait ToolInvokePort {
    async fn invoke_tool(&self, request: ToolInvokeRequest) -> SpokeResult<ToolInvokeResponse>;
}
