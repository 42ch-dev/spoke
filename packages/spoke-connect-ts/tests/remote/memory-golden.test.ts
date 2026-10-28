/**
 * Cross-language memory invoke vectors (memory protocol §13).
 *
 * The fixture is shared byte-for-byte with the Rust harness
 * (`crates/spoke-connect/tests/fixtures/memory-invoke-golden.json`), where the
 * vectors are additionally round-tripped through the generated Rust request /
 * response types. Here they pin the TypeScript side of the contract:
 *
 * - the adapter's `MEMORY_OPS` table is the complete 33-row op catalogue and
 *   matches each vector's `method` → `op` row;
 * - every vector op has a dispatch row in the core table (Task 1 parity);
 * - each payload is a plain request/response object (no wrapper), which is the
 *   shape `RemoteAdapter` sends and `connectResponder`'s memory seam receives.
 */

import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { MEMORY_OPS } from "@42ch/spoke-connect/remote";

import { requiredCapability } from "../../src/core/dispatch.js";

/** Ops in the memory family (protocol §7) — 25 core + 3 reflection + 5 transfer. */
const MEMORY_OP_COUNT = 33;

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

  it("covers the whole memory op catalogue with real dispatch rows", () => {
    const ops = Object.values(MEMORY_OPS) as string[];
    expect(ops).toHaveLength(MEMORY_OP_COUNT);
    expect(new Set(ops).size).toBe(MEMORY_OP_COUNT);
    for (const op of ops) {
      expect(op.startsWith("memory."), `${op} is a memory op`).toBe(true);
      expect(requiredCapability(op), `${op} has a dispatch row`).toBeDefined();
    }
    for (const vector of FIXTURE.vectors) {
      expect(requiredCapability(vector.op), `${vector.op} has a dispatch row`).toBeDefined();
    }
  });
});
