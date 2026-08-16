import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  compileSchemaValidator,
  createSchemaValidator,
  FIXTURES_ROOT,
} from "./schema-validator.js";

const TOOL_DESCRIPTOR_ID =
  "https://spoke42.invalid/schemas/data/tool-descriptor.schema.json";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

function baseDescriptor(): Record<string, unknown> {
  return {
    schema_version: 1,
    capability_id: "tools.tool_demo.lookup",
    op: "tools.tool_demo.lookup",
    description: "Look up a knowledge entry by id.",
    input: {},
    output: {},
  };
}

describe("ToolDescriptor wire contract", () => {
  const ajv = createSchemaValidator();
  const validateDescriptor = compileSchemaValidator(ajv, TOOL_DESCRIPTOR_ID);

  it("accepts an unconstrained ABI (input/output = {})", () => {
    const descriptor = baseDescriptor();
    expect(validateDescriptor(descriptor)).toBe(true);
    expect(validateDescriptor.errors).toBeNull();
  });

  it("accepts object-typed input/output subschemas", () => {
    const descriptor = {
      ...baseDescriptor(),
      input: {
        type: "object",
        properties: { entry_id: { type: "string" } },
        required: ["entry_id"],
      },
      output: {
        type: "object",
        properties: { entry: { type: "object" } },
      },
      idempotent: true,
    };
    expect(validateDescriptor(descriptor)).toBe(true);
    expect(validateDescriptor.errors).toBeNull();
  });

  it("rejects bad namespace grammar in capability_id", () => {
    const descriptor = {
      ...baseDescriptor(),
      capability_id: "tools.BadNs.lookup",
      op: "tools.BadNs.lookup",
    };
    expect(validateDescriptor(descriptor)).toBe(false);
  });

  it("rejects missing output", () => {
    const { output: _output, ...descriptor } = baseDescriptor();
    expect(validateDescriptor(descriptor)).toBe(false);
  });

  it("rejects boolean-schema input (input MUST be an object subschema)", () => {
    const descriptor = { ...baseDescriptor(), input: true };
    expect(validateDescriptor(descriptor)).toBe(false);
  });

  it("documents the op === capability_id boundary (draft-07 cannot express cross-field equality)", () => {
    // Both fields are pattern-constrained; equality is a spec-level MUST
    // enforced by the validateToolDescriptor helper (plan 2), not the schema.
    const mismatched = { ...baseDescriptor(), op: "tools.tool_demo.rank" };
    expect(mismatched.op).not.toBe(mismatched.capability_id);
    expect(validateDescriptor(mismatched)).toBe(true);
  });
});

describe("host_tw_tools manifest tools consistency (spec MUSTs)", () => {
  it("lists every tool capability in capabilities[] and its namespace in namespaces[]", () => {
    const manifest = loadFixture<{
      capabilities: string[];
      namespaces: string[];
      tools: Array<{ capability_id: string; op: string }>;
    }>("host_tw_tools.json");

    const capabilities = new Set(manifest.capabilities);
    const namespaces = new Set(manifest.namespaces);

    expect(manifest.tools.length).toBeGreaterThan(0);
    for (const tool of manifest.tools) {
      expect(capabilities.has(tool.capability_id)).toBe(true);
      expect(tool.op).toBe(tool.capability_id);
      const ns = tool.capability_id.split(".")[1];
      expect(ns).toBeDefined();
      expect(namespaces.has(ns!)).toBe(true);
    }
  });
});
