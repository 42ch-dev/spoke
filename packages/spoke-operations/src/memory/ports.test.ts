import { describe, expect, it } from "vitest";

import type { Principal } from "@42ch/spoke-schemas";

import {
  SpokeRejectCode,
  type MemoryPort,
  type MemoryPorts,
} from "../index.js";

/** Every method name of the port contract, type-checked as `keyof MemoryPort`. */
const MEMORY_PORT_METHODS: (keyof MemoryPort)[] = [
  "describe",
  "listSpaces",
  "freezeSpace",
  "unfreezeSpace",
  "ensureSubject",
  "getSubject",
  "fenceSubject",
  "listGrants",
  "grant",
  "revokeGrant",
  "capture",
  "listCandidates",
  "getCandidate",
  "dismissCandidate",
  "reviewCandidate",
  "createRecord",
  "getRecord",
  "updateRecord",
  "deleteRecord",
  "listRecords",
  "promoteRecord",
  "recall",
  "getProfile",
  "updateProfile",
  "getReflectionState",
  "listReflections",
  "generateReflection",
  "getReceipt",
  "createExport",
  "getExport",
  "stageImport",
  "activateImport",
  "discardImport",
];

describe("MemoryPort / MemoryPorts (TS parity with the Rust trait)", () => {
  it("declares exactly the 33 op methods (§7.1–7.33)", () => {
    expect(MEMORY_PORT_METHODS).toHaveLength(33);
    expect(new Set(MEMORY_PORT_METHODS).size).toBe(33);
  });

  it("takes Principal as the explicit first parameter (type-level probe)", () => {
    // Assignability probe: each representative method must accept a
    // Principal-first signature and return an awaitable SpokeResult.
    const describe: MemoryPort["describe"] = (
      _principal: Principal,
      _request: Parameters<MemoryPort["describe"]>[1],
    ) => Promise.reject(new Error("unused"));
    const capture: MemoryPort["capture"] = (
      _principal: Principal,
      _request: Parameters<MemoryPort["capture"]>[1],
    ) => Promise.reject(new Error("unused"));
    const discardImport: MemoryPort["discardImport"] = (
      _principal: Principal,
      _request: Parameters<MemoryPort["discardImport"]>[1],
    ) => Promise.reject(new Error("unused"));

    expect([describe, capture, discardImport]).toHaveLength(3);
  });

  it("probe defaults to undefined for adapters that do not serve memory", () => {
    const adapter: MemoryPorts = {};
    const probe: MemoryPort | undefined = adapter.asMemory?.();
    expect(probe).toBeUndefined();
  });

  it("carries the 14 §14.1 memory reject codes", () => {
    const codes = [
      "NOT_FOUND",
      "NOT_AUTHORIZED",
      "GRANT_REVOKED",
      "SPACE_FROZEN",
      "SUBJECT_FENCED",
      "CANDIDATE_CONSUMED",
      "PROFILE_INCOMPLETE",
      "INSUFFICIENT_DATA",
      "IDEMPOTENCY_CONFLICT",
      "OUTCOME_UNCERTAIN",
      "EXPORT_EXPIRED",
      "IMPORT_CONFLICT",
      "RECEIPT_EXPIRED",
      "CURSOR_INVALID",
    ] as const;
    expect(codes).toHaveLength(14);
    for (const wire of codes) {
      expect(Object.values(SpokeRejectCode)).toContain(wire);
    }
  });
});
