import { describe, expect, it } from "vitest";

import {
  CAPABILITY_L2_COMPUTABLE,
  CAPABILITY_SPOKE_BASELINE,
  dispatchAllowed,
  requiredCapability,
} from "../src/core/dispatch.js";

const BASELINE_OPS = ["upsert", "promote", "relate", "check", "assemble"];
const COMPUTABLE_OPS = ["project", "compute"];

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
