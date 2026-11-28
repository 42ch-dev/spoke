import {
  SpokeRejectCode,
  spokeReject,
  type SpokeResult,
} from "../result.js";
import { parseToolCapabilityId } from "./helpers.js";
import type {
  ToolInvokePort,
  ToolInvokeRequest,
  ToolInvokeResponse,
} from "./port.js";

export async function orchestrateInvokeTool(
  port: ToolInvokePort,
  request: ToolInvokeRequest,
): Promise<SpokeResult<ToolInvokeResponse>> {
  const grammar = parseToolCapabilityId(request.capability_id);
  if (!grammar.ok) {
    return grammar;
  }
  if (port == null || typeof port.invokeTool !== "function") {
    return spokeReject(
      SpokeRejectCode.CAPABILITY_PORT_MISSING,
      `Missing ToolInvokePort for capability "${request.capability_id}" (port is null/undefined or invokeTool is not a function)`,
      { capability: request.capability_id },
    );
  }
  return port.invokeTool(request);
}
