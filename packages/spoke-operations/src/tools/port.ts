import type { SpokeResult } from "../result.js";

/**
 * Request to invoke a remote tool by its `tools.<ns>.<tool_id>` capability id.
 */
export type ToolInvokeRequest = {
  capability_id: string;
  arguments: Record<string, unknown>;
};

/**
 * Successful tool invocation result — an opaque JSON value.
 */
export type ToolInvokeResponse = {
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
