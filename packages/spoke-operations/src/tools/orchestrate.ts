/**
 * Orchestrated remote tool invocation over an injected `ToolInvokePort`.
 *
 * Frozen sequence (`tool-contracts.md` §5; no `requireToolInvokePort`
 * wrapper): (1) `parseToolCapabilityId(request.capability_id)` grammar gate
 * → `INVALID_INPUT`; (2) runtime guard `typeof port.invokeTool ===
 * "function"` → else `CAPABILITY_PORT_MISSING` with
 * `details.capability = request.capability_id` (per-tool blame — observable
 * at the JS boundary where structural typing cannot protect dynamic
 * composition: plain objects reject structurally instead of throwing a
 * `TypeError`); (3) `port.invokeTool(request)` returned as-is.
 *
 * Argument validation is NOT re-run here — callers run the structural
 * argument gate (`validateToolArguments`, see `tools/helpers.ts`) before
 * orchestrating; this function validates only request grammar.
 */

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
  if (typeof port.invokeTool !== "function") {
    return spokeReject(
      SpokeRejectCode.CAPABILITY_PORT_MISSING,
      `Missing ToolInvokePort for capability "${request.capability_id}" (invokeTool is not a function)`,
      { capability: request.capability_id },
    );
  }
  return port.invokeTool(request);
}
