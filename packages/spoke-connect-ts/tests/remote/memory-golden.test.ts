import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { MEMORY_OPS } from "@42ch/spoke-connect/remote";

import { CAPABILITY_SPOKE_MEMORY, requiredCapability } from "../../src/core/dispatch.js";

interface Vector {
  op: string;
  method: string;
  request: Record<string, unknown>;
  response: Record<string, unknown>;
}

const FIXTURE = JSON.parse(
  readFileSync(new URL("../fixtures/memory-invoke-golden.json", import.meta.url), "utf8"),
) as { schema_version: number; vectors: Vector[] };

const RUST_FIXTURE_BYTES = readFileSync(
  new URL(
    "../../../../crates/spoke-connect/tests/fixtures/memory-invoke-golden.json",
    import.meta.url,
  ),
);

const TS_FIXTURE_BYTES = readFileSync(
  new URL("../fixtures/memory-invoke-golden.json", import.meta.url),
);

describe("memory invoke golden vectors (shared with the Rust harness)", () => {
  it("is byte-identical to the Rust fixture", () => {
    expect(TS_FIXTURE_BYTES.equals(RUST_FIXTURE_BYTES)).toBe(true);
  });

  it("maps every vector method to its op through MEMORY_OPS", () => {
    expect(FIXTURE.vectors.length).toBeGreaterThanOrEqual(8);
    for (const vector of FIXTURE.vectors) {
      expect(
        (MEMORY_OPS as Record<string, string>)[vector.method],
        `vector method ${vector.method}`,
      ).toBe(vector.op);
    }
  });

  it("carries request and response payload objects only", () => {
    for (const vector of FIXTURE.vectors) {
      expect(typeof vector.request).toBe("object");
      expect(typeof vector.response).toBe("object");
      expect(Array.isArray(vector.request)).toBe(false);
      expect(Array.isArray(vector.response)).toBe(false);
    }
  });

  it("wires the create row to the memory capability and a real dispatch row", () => {
    expect(MEMORY_OPS.createSpace).toBe("memory.spaces.create");
    expect(requiredCapability(MEMORY_OPS.createSpace)).toBe(CAPABILITY_SPOKE_MEMORY);
  });

  it("maps every memory op row to a real dispatch row", () => {
    const ops = Object.values(MEMORY_OPS) as string[];
    expect(new Set(ops).size).toBe(ops.length);
    for (const op of ops) {
      expect(op.startsWith("memory."), `${op} is a memory op`).toBe(true);
      expect(requiredCapability(op), `${op} has a dispatch row`).toBeDefined();
    }
    for (const vector of FIXTURE.vectors) {
      expect(requiredCapability(vector.op), `${vector.op} has a dispatch row`).toBeDefined();
    }
  });
});
