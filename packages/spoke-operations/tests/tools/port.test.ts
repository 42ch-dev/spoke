import type { ToolDescriptor } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  validateToolArguments,
  type SpokeResult,
  type ToolInvokePort,
  type ToolInvokeRequest,
  type ToolInvokeResponse,
} from "@42ch/spoke-operations";

function makeDescriptor(
  overrides: Partial<ToolDescriptor> = {},
): ToolDescriptor {
  return {
    schema_version: 1,
    capability_id: "tools.tool_demo.lookup",
    op: "tools.tool_demo.lookup",
    description: "Look up a knowledge entry by id.",
    input: {},
    output: {},
    ...overrides,
  };
}

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

describe("ToolInvokeRequest / ToolInvokeResponse types", () => {
  it("carry capability_id and arguments fields", () => {
    const request: ToolInvokeRequest = makeRequest({
      arguments: { query: "ke-1", limit: 10 },
    });

    expect(request.capability_id).toBe("tools.tool_demo.lookup");
    expect(request.arguments).toEqual({ query: "ke-1", limit: 10 });
  });

  it("carry an opaque result value of any JSON shape", () => {
    const response: ToolInvokeResponse = {
      result: { entry: { id: "ke-1" }, tags: ["a", "b"], ok: true },
    };

    expect(response.result).toEqual({
      entry: { id: "ke-1" },
      tags: ["a", "b"],
      ok: true,
    });
  });
});

describe("ToolInvokePort contract", () => {
  it("invokes a mock port and returns its response", async () => {
    const port = createEchoPort();

    const result = await port.invokeTool(
      makeRequest({ arguments: { query: "ke-1" } }),
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({
        result: { echo: "tools.tool_demo.lookup" },
      });
    }
  });

  it("passes through a reject result as-is", async () => {
    const port: ToolInvokePort = {
      async invokeTool(): Promise<SpokeResult<ToolInvokeResponse>> {
        return spokeReject(
          SpokeRejectCode.CAPABILITY_PORT_MISSING,
          "no port",
          { capability: "tools.tool_demo.lookup" },
        );
      },
    };

    const result = await port.invokeTool(makeRequest());

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details).toEqual({
        capability: "tools.tool_demo.lookup",
      });
    }
  });

  it("does not re-validate arguments (gate is the caller's job)", async () => {
    const port = createEchoPort();

    // Direct port invocation with arguments that would fail the structural
    // gate still reaches the port — validation happens before, not inside.
    const result = await port.invokeTool(makeRequest({ arguments: {} }));

    expect(result.ok).toBe(true);
  });
});

describe("argument-shape gate against mock ports", () => {
  const requiredDescriptor = makeDescriptor({
    input: {
      type: "object",
      properties: { query: { type: "string" } },
      required: ["query"],
    },
  });

  it("admits well-shaped arguments before invoking the port", async () => {
    const request = makeRequest({ arguments: { query: "ke-1" } });

    const gate = validateToolArguments(requiredDescriptor, request.arguments);
    expect(gate.ok).toBe(true);

    const result = await createEchoPort().invokeTool(request);
    expect(result.ok).toBe(true);
  });

  it("rejects missing required keys with INVALID_INPUT details", () => {
    const request = makeRequest({ arguments: {} });

    const gate = validateToolArguments(requiredDescriptor, request.arguments);

    expect(gate.ok).toBe(false);
    if (!gate.ok) {
      expect(gate.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(gate.details).toEqual({
        field: "arguments",
        missing: ["query"],
      });
    }
  });

  it("rejects non-object arguments with field details", () => {
    for (const bad of [null, 42, "nope", ["a"]]) {
      const gate = validateToolArguments(
        requiredDescriptor,
        bad as unknown as Record<string, unknown>,
      );
      expect(gate.ok).toBe(false);
      if (!gate.ok) {
        expect(gate.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(gate.details).toEqual({ field: "arguments" });
      }
    }
  });
});
