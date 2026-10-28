import { describe, expect, it } from "vitest";

import {
  CAPABILITY_KE_EXTRACTION,
  CAPABILITY_L2_COMPUTABLE,
  CAPABILITY_SPOKE_BASELINE,
  CAPABILITY_SPOKE_MEMORY,
  CAPABILITY_SPOKE_MEMORY_REFLECTION,
  CAPABILITY_SPOKE_MEMORY_TRANSFER,
  dispatchAllowed,
  requiredCapability,
  tokenAuthorizesOp,
} from "../src/core/dispatch.js";

const BASELINE_OPS = ["upsert", "promote", "relate", "check", "assemble"];
const COMPUTABLE_OPS = ["project", "compute"];

const MEMORY_CORE_OPS = [
  "memory.describe",
  "memory.spaces.list",
  "memory.spaces.freeze",
  "memory.spaces.unfreeze",
  "memory.subjects.ensure",
  "memory.subjects.get",
  "memory.subjects.fence",
  "memory.grants.list",
  "memory.grants.grant",
  "memory.grants.revoke",
  "memory.capture",
  "memory.candidates.list",
  "memory.candidates.get",
  "memory.candidates.dismiss",
  "memory.candidates.review",
  "memory.records.create",
  "memory.records.get",
  "memory.records.update",
  "memory.records.delete",
  "memory.records.list",
  "memory.records.promote",
  "memory.recall",
  "memory.profile.get",
  "memory.profile.update",
  "memory.receipts.get",
];
const MEMORY_REFLECTION_OPS = [
  "memory.reflections.state",
  "memory.reflections.list",
  "memory.reflections.generate",
];
const MEMORY_TRANSFER_OPS = [
  "memory.exports.create",
  "memory.exports.get",
  "memory.imports.stage",
  "memory.imports.activate",
  "memory.imports.discard",
];

describe("dispatch gate (port of dispatch.rs)", () => {
  it("baseline ops require spoke-baseline", () => {
    for (const op of BASELINE_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_SPOKE_BASELINE);
      expect(dispatchAllowed(op, ["spoke-baseline"])).toBe(true);
      expect(dispatchAllowed(op, ["spoke-connect", "spoke-baseline"])).toBe(true);
      expect(dispatchAllowed(op, ["l2-computable"])).toBe(false);
      expect(dispatchAllowed(op, [])).toBe(false);
    }
  });

  it("compute ops require l2-computable", () => {
    for (const op of COMPUTABLE_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_L2_COMPUTABLE);
      expect(dispatchAllowed(op, ["l2-computable"])).toBe(true);
      expect(dispatchAllowed(op, ["spoke-baseline", "l2-computable"])).toBe(true);
      expect(dispatchAllowed(op, ["spoke-baseline"])).toBe(false);
    }
  });

  it("unknown ops fail closed", () => {
    expect(requiredCapability("custom-op")).toBeUndefined();
    expect(dispatchAllowed("custom-op", ["spoke-baseline"])).toBe(false);
    expect(dispatchAllowed("custom-op", ["spoke-connect"])).toBe(false);
    expect(dispatchAllowed("", ["spoke-baseline"])).toBe(false);
  });
});

describe("ke remote", () => {
  it("extract requires ke-extraction in the core dispatch table", () => {
    expect(requiredCapability("extract")).toBe(CAPABILITY_KE_EXTRACTION);
    expect(dispatchAllowed("extract", ["ke-extraction"])).toBe(true);
    expect(dispatchAllowed("extract", ["spoke-baseline"])).toBe(false);
    expect(dispatchAllowed("extract", [])).toBe(false);
  });
});

// Memory family (SPOKE memory protocol §4.1/§13): parity twin of the Rust
// `core::dispatch::tests` memory block. Three flags gate disjoint op groups;
// the gate evaluates negotiated capabilities only.
describe("memory family", () => {
  const ALL_MEMORY = [
    CAPABILITY_SPOKE_MEMORY,
    CAPABILITY_SPOKE_MEMORY_REFLECTION,
    CAPABILITY_SPOKE_MEMORY_TRANSFER,
  ];

  it("covers all 33 protocol ops with the expected flag", () => {
    expect(
      MEMORY_CORE_OPS.length + MEMORY_REFLECTION_OPS.length + MEMORY_TRANSFER_OPS.length,
    ).toBe(33);
    for (const op of MEMORY_CORE_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_SPOKE_MEMORY);
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY])).toBe(true);
    }
    for (const op of MEMORY_REFLECTION_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_SPOKE_MEMORY_REFLECTION);
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY_REFLECTION])).toBe(true);
    }
    for (const op of MEMORY_TRANSFER_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_SPOKE_MEMORY_TRANSFER);
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY_TRANSFER])).toBe(true);
    }
  });

  it("unnegotiated memory op denies while unknown memory op stays unsupported", () => {
    // `memory.capture` has a row, so the boundary answers `capability_missing`.
    expect(requiredCapability("memory.capture")).toBe(CAPABILITY_SPOKE_MEMORY);
    expect(dispatchAllowed("memory.capture", [])).toBe(false);
    expect(
      dispatchAllowed("memory.capture", ["spoke-baseline", "l2-computable", "ke-extraction"]),
    ).toBe(false);
    // No row ⇒ no requirement ⇒ the host answers `op_unsupported`.
    expect(requiredCapability("memory.archive.purge")).toBeUndefined();
    expect(dispatchAllowed("memory.archive.purge", ALL_MEMORY)).toBe(false);
  });

  it("memory flags never cross-authorize other families", () => {
    for (const op of MEMORY_CORE_OPS) {
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY_REFLECTION])).toBe(false);
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY_TRANSFER])).toBe(false);
    }
    for (const op of MEMORY_REFLECTION_OPS) {
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY])).toBe(false);
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY_TRANSFER])).toBe(false);
    }
    for (const op of MEMORY_TRANSFER_OPS) {
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY])).toBe(false);
      expect(dispatchAllowed(op, [CAPABILITY_SPOKE_MEMORY_REFLECTION])).toBe(false);
    }
    expect(
      dispatchAllowed("memory.recall", [
        CAPABILITY_SPOKE_BASELINE,
        CAPABILITY_L2_COMPUTABLE,
        CAPABILITY_KE_EXTRACTION,
      ]),
    ).toBe(false);
  });

  it("sub-flag without core leaves core ops unnegotiated", () => {
    // Protocol §4.1: a provider declaring a sub-flag MUST also declare
    // `spoke-memory`; a session with only the sub-flag admits the sub-family
    // and still fails core ops with `capability_missing`.
    expect(dispatchAllowed("memory.reflections.list", [CAPABILITY_SPOKE_MEMORY_REFLECTION])).toBe(
      true,
    );
    expect(dispatchAllowed("memory.capture", [CAPABILITY_SPOKE_MEMORY_REFLECTION])).toBe(false);
  });

  it("fully negotiated session dispatches the whole family", () => {
    for (const op of [...MEMORY_CORE_OPS, ...MEMORY_REFLECTION_OPS, ...MEMORY_TRANSFER_OPS]) {
      expect(dispatchAllowed(op, ALL_MEMORY)).toBe(true);
      expect(tokenAuthorizesOp(requiredCapability(op), ALL_MEMORY)).toBe(true);
    }
  });
});
