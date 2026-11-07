import { describe, expect, it } from "vitest";

import {
  CAPABILITY_L2_COMPUTABLE,
  CAPABILITY_SPOKE_BASELINE,
  CAPABILITY_SPOKE_COGNITION,
  cognitionRequirementField,
  cognitionRequirementPresent,
  dispatchAllowed,
  requiredCapability,
  tokenAuthorizesOp,
} from "../../src/core/dispatch.js";
import { negotiatedCapabilities } from "../../src/core/session.js";

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

describe("tools.* prefix rule (parity golden vector, frozen §3)", () => {
  const TOOL_OP = "tools.math.add";

  it("requiredCapability returns the op string itself for the tools. prefix", () => {
    expect(requiredCapability(TOOL_OP)).toBe(TOOL_OP);
    expect(requiredCapability("tools.any.namespaced.thing")).toBe(
      "tools.any.namespaced.thing",
    );
  });

  it("dispatchAllowed authorizes only when the exact capability is negotiated", () => {
    // Authorized: the tool capability string itself is negotiated.
    expect(dispatchAllowed(TOOL_OP, [TOOL_OP])).toBe(true);
    // Not negotiated / wrong capability: denied (the self-describing tool
    // gate never consults an umbrella flag or the baseline capability).
    expect(dispatchAllowed(TOOL_OP, [CAPABILITY_SPOKE_BASELINE])).toBe(false);
    expect(dispatchAllowed(TOOL_OP, [CAPABILITY_L2_COMPUTABLE])).toBe(false);
    expect(dispatchAllowed(TOOL_OP, [])).toBe(false);
  });

  it("tokenAuthorizesOp authorizes iff the grant contains the exact capability", () => {
    expect(tokenAuthorizesOp(requiredCapability(TOOL_OP), [TOOL_OP])).toBe(true);
    expect(
      tokenAuthorizesOp(requiredCapability(TOOL_OP), [CAPABILITY_SPOKE_BASELINE]),
    ).toBe(false);
    expect(tokenAuthorizesOp(requiredCapability(TOOL_OP), [])).toBe(false);
  });
});

import { CAPABILITY_KE_EXTRACTION } from "../../src/core/dispatch.js";

describe("ke remote", () => {
  it("tokenAuthorizesOp authorizes extract when ke-extraction is granted", () => {
    expect(
      tokenAuthorizesOp(requiredCapability("extract"), ["ke-extraction"]),
    ).toBe(true);
    expect(
      tokenAuthorizesOp(requiredCapability("extract"), [CAPABILITY_SPOKE_BASELINE]),
    ).toBe(false);
  });
});

describe("spoke-cognition family (parity with dispatch.rs)", () => {
  it("has one canonical family constant and no op-table row", () => {
    expect(CAPABILITY_SPOKE_COGNITION).toBe("spoke-cognition");
    // The requirement is payload-only: no op names the family, so an
    // unnegotiated family never denies an ordinary call.
    for (const op of [
      "upsert",
      "check",
      "assemble",
      "extract",
      "port.scope.list_knowledge_entries",
      "memory.records.list",
      "memory.recall",
    ]) {
      expect(requiredCapability(op), `${op} must not require the family`).not.toBe(
        CAPABILITY_SPOKE_COGNITION,
      );
    }
  });

  it("locates the op's declared requirement member on the raw payload", () => {
    // Only the actual carrier of the dispatched operation can demand
    // cognition; every slot below comes from the registered request schema.
    expect(
      cognitionRequirementField("check", {
        scope: { scope_id: "s", cognition: { view: "current" } },
      }),
    ).toBe("scope.cognition");
    expect(
      cognitionRequirementField("port.scope.list_knowledge_entries", {
        scope: { scope_id: "s", cognition: { view: "current" } },
      }),
    ).toBe("scope.cognition");
    expect(
      cognitionRequirementField("memory.records.list", {
        cognition_filter: { state: { eq: "known" } },
      }),
    ).toBe("cognition_filter");
    // A present `null` is a requirement (malformed), never "absent".
    expect(
      cognitionRequirementField("memory.recall", { cognition_filter: null }),
    ).toBe("cognition_filter");
    // The slots are disjoint per op, so unrelated data can never stand in
    // front of the op's real requirement.
    expect(
      cognitionRequirementField("memory.records.list", {
        cognition: {},
        cognition_filter: { q_min: 0.5 },
      }),
    ).toBe("cognition_filter");
    expect(cognitionRequirementPresent("upsert", {})).toBe(false);
    expect(
      cognitionRequirementPresent("port.scope.list_knowledge_entries", {
        scope: { scope_id: "s", viewpoint: "kb_mira" },
      }),
    ).toBe(false);
    expect(
      cognitionRequirementPresent("port.finding.put", { findings: [] }),
    ).toBe(false);
    expect(cognitionRequirementPresent("check", { scope: "s" })).toBe(false);
    // Tool arguments, a Memory filter on a Scope op, and any opaque
    // `cognition` member elsewhere are ordinary payload data.
    expect(
      cognitionRequirementPresent("tools.math.add", {
        arguments: {},
        cognition: "tool metadata",
      }),
    ).toBe(false);
    expect(
      cognitionRequirementPresent("port.scope.list_knowledge_entries", {
        scope: { scope_id: "s" },
        cognition_filter: { state: { eq: "known" } },
      }),
    ).toBe(false);
    expect(
      cognitionRequirementPresent("upsert", { cognition: { view: "recorded_at" } }),
    ).toBe(false);
    expect(
      cognitionRequirementPresent("memory.records.list", {
        scope: { scope_id: "s", cognition: { view: "current" } },
      }),
    ).toBe(false);
    // A Memory content write's `content.cognition` is written data, not a
    // request requirement; its placement `scope` is unrelated payload too.
    expect(
      cognitionRequirementPresent("memory.records.create", {
        content: { cognition: { records: [] } },
        scope: { space_id: "s", subject_id: "sub" },
      }),
    ).toBe(false);
    expect(
      cognitionRequirementPresent("check", {
        scope: { scope_id: "s", cognition: null },
      }),
    ).toBe(true);
  });

  it("never derives the family from the legacy flag", () => {
    // The negotiation is the plain intersection: `l5-mind` on one side grants
    // nothing, and no alias exists on either side.
    expect(negotiatedCapabilities(["l5-mind"], ["spoke-cognition"])).toEqual([]);
    expect(negotiatedCapabilities(["spoke-cognition"], ["l5-mind"])).toEqual([]);
  });
});
