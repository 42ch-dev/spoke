import { describe, expect, it } from "vitest";

import {
  orchestrateInvokeTool,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
  type ToolInvokePort,
  type ToolInvokeRequest,
  type ToolInvokeResponse,
} from "@42ch/spoke-operations";

function makeRequest(
  overrides: Partial<ToolInvokeRequest> = {},
): ToolInvokeRequest {
  return {
    capability_id: "tools.tool_demo.lookup",
    arguments: {},
    ...overrides,
  };
}

function createEchoPort(): ToolInvokePort {
  return {
    async invokeTool(request): Promise<SpokeResult<ToolInvokeResponse>> {
      return spokeOk({ result: { echo: request.capability_id } });
    },
  };
}

describe("orchestrateInvokeTool", () => {
  it("returns the port's response for a valid capability id", async () => {
    const result = await orchestrateInvokeTool(
      createEchoPort(),
      makeRequest({ arguments: { query: "ke-1" } }),
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({
        result: { echo: "tools.tool_demo.lookup" },
      });
    }
  });

  it("passes through a port reject result as-is", async () => {
    const port: ToolInvokePort = {
      async invokeTool(): Promise<SpokeResult<ToolInvokeResponse>> {
        return spokeReject(
          SpokeRejectCode.INTERNAL_ERROR,
          "tool exploded",
        );
      },
    };

    const result = await orchestrateInvokeTool(port, makeRequest());

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
      expect(result.message).toBe("tool exploded");
    }
  });

  it("rejects bad request grammar with INVALID_INPUT before touching the port", async () => {
    let invoked = false;
    const port: ToolInvokePort = {
      async invokeTool(): Promise<SpokeResult<ToolInvokeResponse>> {
        invoked = true;
        return spokeOk({ result: null });
      },
    };

    const result = await orchestrateInvokeTool(
      port,
      makeRequest({ capability_id: "tools.Bad.lookup" }),
    );

    // Grammar gate runs first: the port must never be reached.
    expect(invoked).toBe(false);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({ capability_id: "tools.Bad.lookup" });
    }
  });

  it("rejects a structurally-missing port with CAPABILITY_PORT_MISSING per-tool blame", async () => {
    // JS boundary: a plain object that is not a ToolInvokePort must reject
    // structurally instead of throwing a TypeError.
    const notAPort = {} as unknown as ToolInvokePort;

    const result = await orchestrateInvokeTool(
      notAPort,
      makeRequest({ capability_id: "tools.tool_demo.rank" }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details).toEqual({ capability: "tools.tool_demo.rank" });
    }
  });

  it("rejects a null/undefined port with CAPABILITY_PORT_MISSING per-tool blame", async () => {
    // JS boundary: `null`/`undefined` must reject structurally (same code +
    // per-tool blame) instead of throwing a TypeError on `port.invokeTool`.
    for (const notAPort of [null, undefined] as const) {
      const result = await orchestrateInvokeTool(
        notAPort as unknown as ToolInvokePort,
        makeRequest({ capability_id: "tools.tool_demo.rank" }),
      );

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
        expect(result.details).toEqual({ capability: "tools.tool_demo.rank" });
      }
    }
  });

  it("does not re-run argument validation (grammar-only; arbitrary arguments reach the port)", async () => {
    const port = createEchoPort();

    // Arguments that would fail the structural argument gate still reach the
    // port — argument validation is the caller's job (validateToolArguments).
    const result = await orchestrateInvokeTool(
      port,
      makeRequest({
        arguments: null as unknown as Record<string, unknown>,
      }),
    );

    expect(result.ok).toBe(true);
  });
});
