//! Orchestrated remote tool invocation over an injected `ToolInvokePort`
//! (parity with `packages/spoke-operations/src/tools/orchestrate.ts`).
//!
//! Frozen sequence (`tool-contracts.md` §5; no `requireToolInvokePort`
//! wrapper): (1) `parse_tool_capability_id(request.capability_id)` grammar
//! gate → `INVALID_INPUT`; (2) the port is `&dyn ToolInvokePort`, so absence
//! is unrepresentable at the type level — the negative test pins a
//! `MissingToolInvokePort` double whose `invoke_tool` returns the same
//! `CAPABILITY_PORT_MISSING` reject with `details.capability`, demonstrating
//! code parity with the TS runtime guard; (3) `port.invoke_tool(request)`
//! returned as-is.
//!
//! Argument validation is NOT re-run here — callers run the structural
//! argument gate ([`crate::validate_tool_arguments`]) before orchestrating;
//! this function validates only request grammar.

use crate::result::SpokeResult;
use crate::tools::helpers::parse_tool_capability_id;
use crate::tools::port::{ToolInvokePort, ToolInvokeRequest, ToolInvokeResponse};

pub async fn orchestrate_invoke_tool(
    port: &dyn ToolInvokePort,
    request: ToolInvokeRequest,
) -> SpokeResult<ToolInvokeResponse> {
    match parse_tool_capability_id(&request.capability_id) {
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        SpokeResult::Ok(_) => port.invoke_tool(request).await,
    }
}
