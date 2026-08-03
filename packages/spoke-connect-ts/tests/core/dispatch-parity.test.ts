import { describe, expect, it } from "vitest";

import {
  CAPABILITY_L2_COMPUTABLE,
  CAPABILITY_SPOKE_BASELINE,
  dispatchAllowed,
  requiredCapability,
  tokenAuthorizesOp,
} from "../../src/core/dispatch.js";

/**
 * `tokenAuthorizesOp` parity tests — mirror
 * `crates/spoke-connect/src/core/dispatch.rs` unit tests
 * (`token_grant_authorizes_by_membership`,
 * `token_grant_missing_requirement_denies`,
 * `token_grant_unknown_ops_fail_closed`).
 *
 * The core-op table spot-check re-asserts `requiredCapability` /
 * `dispatchAllowed` against the Rust table (contract says parity; this is
 * the re-diff guard requested by the brief).
 */

const BASELINE_OPS = ["upsert", "promote", "relate", "check", "assemble"];
const COMPUTABLE_OPS = ["project", "compute"];

describe("tokenAuthorizesOp (port of dispatch.rs token_authorizes_op)", () => {
  it("authorizes by membership of the required capability (subset-of-grant)", () => {
    // Extra capabilities on the token are ignored when unused.
    expect(
      tokenAuthorizesOp(CAPABILITY_SPOKE_BASELINE, [
        "spoke-baseline",
        "l2-computable",
        "unused-extra",
      ]),
    ).toBe(true);
    expect(tokenAuthorizesOp(CAPABILITY_L2_COMPUTABLE, ["l2-computable"])).toBe(true);
    // The grant is NOT an exact-list match: a subset suffices.
    expect(
      tokenAuthorizesOp(CAPABILITY_SPOKE_BASELINE, [
        "spoke-baseline",
        "l2-computable",
      ]),
    ).toBe(true);
  });

  it("denies when the grant misses the required capability", () => {
    expect(tokenAuthorizesOp(CAPABILITY_L2_COMPUTABLE, ["spoke-baseline"])).toBe(
      false,
    );
    expect(tokenAuthorizesOp(CAPABILITY_SPOKE_BASELINE, [])).toBe(false);
    expect(tokenAuthorizesOp(CAPABILITY_SPOKE_BASELINE, ["l2-computable"])).toBe(
      false,
    );
  });

  it("fails closed when the op has no required capability (undefined required)", () => {
    expect(tokenAuthorizesOp(undefined, ["spoke-baseline"])).toBe(false);
    expect(tokenAuthorizesOp(undefined, [])).toBe(false);
  });

  it("composes with requiredCapability like the Rust dispatch gate", () => {
    for (const op of BASELINE_OPS) {
      expect(tokenAuthorizesOp(requiredCapability(op), ["spoke-baseline"])).toBe(
        true,
      );
      expect(tokenAuthorizesOp(requiredCapability(op), ["l2-computable"])).toBe(
        false,
      );
    }
    for (const op of COMPUTABLE_OPS) {
      expect(tokenAuthorizesOp(requiredCapability(op), ["l2-computable"])).toBe(
        true,
      );
      expect(tokenAuthorizesOp(requiredCapability(op), ["spoke-baseline"])).toBe(
        false,
      );
    }
    // Unknown op: no requirement → never authorized by the token gate.
    expect(
      tokenAuthorizesOp(requiredCapability("custom-op"), ["spoke-baseline"]),
    ).toBe(false);
  });
});

describe("core-op dispatch table spot-check (parity with dispatch.rs)", () => {
  it("baseline ops require spoke-baseline; compute ops require l2-computable", () => {
    for (const op of BASELINE_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_SPOKE_BASELINE);
      expect(dispatchAllowed(op, ["spoke-baseline"])).toBe(true);
      expect(dispatchAllowed(op, ["l2-computable"])).toBe(false);
    }
    for (const op of COMPUTABLE_OPS) {
      expect(requiredCapability(op)).toBe(CAPABILITY_L2_COMPUTABLE);
      expect(dispatchAllowed(op, ["l2-computable"])).toBe(true);
      expect(dispatchAllowed(op, ["spoke-baseline"])).toBe(false);
    }
  });
});
