/**
 * Optional `ToolInvokePort` family — wire-adjacent invoke request/response
 * types and the async port contract for injecting remote tool invocation.
 *
 * The family is standalone: it is NOT folded into `BaselinePorts`, and no
 * `ToolsPort` / `ToolsAdapter` composed alias exists (capability gating is
 * per-tool — the capability string itself — not per-composed-type).
 *
 * Purity: the library stays I/O-free; port methods return awaitable results
 * and the library only awaits injected ports. The port itself does NOT
 * re-validate request arguments — callers run the structural argument gate
 * (`validateToolArguments`, see `tools/helpers.ts`) before invoking.
 */

import type { SpokeResult } from "../result.js";

/**
 * Request to invoke a remote tool by its `tools.<ns>.<tool_id>` capability id.
 */
export type ToolInvokeRequest = {
  /** Tool capability string `tools.<ns>.<tool_id>`. */
  capability_id: string;
  /** Opaque JSON arguments object for the tool. */
  arguments: Record<string, unknown>;
};

/**
 * Successful tool invocation result — an opaque JSON value.
 */
export type ToolInvokeResponse = {
  /** Opaque JSON result value produced by the tool. */
  result: unknown;
};

/**
 * Optional port family for invoking remote tools by capability id.
 *
 * The structural argument gate is a caller-side step (`validateToolArguments`);
 * the port itself does not re-validate.
 */
export interface ToolInvokePort {
  invokeTool(request: ToolInvokeRequest): Promise<SpokeResult<ToolInvokeResponse>>;
}
