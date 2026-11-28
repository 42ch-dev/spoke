import { describe, expect, it } from "vitest";

import {
  CAPABILITY_L2_COMPUTABLE,
  CAPABILITY_SPOKE_BASELINE,
  CAPABILITY_SPOKE_COGNITION,
  CAPABILITY_SPOKE_MEMORY,
  CAPABILITY_SPOKE_MEMORY_REFLECTION,
  CAPABILITY_SPOKE_MEMORY_TRANSFER,
  cognitionRequirementField,
  cognitionRequirementPresent,
  dispatchAllowed,
  requiredCapability,
  tokenAuthorizesOp,
} from "../../src/core/dispatch.js";
import { negotiatedCapabilities } from "../../src/core/session.js";

const BASELINE_OPS = ["upsert", "promote", "relate", "check", "assemble"];
const COMPUTABLE_OPS = ["project", "compute"];

describe("tokenAuthorizesOp (port of dispatch.rs token_authorizes_op)", () => {
  it("authorizes by membership of the required capability (subset-of-grant)", () => {
    expect(
      tokenAuthorizesOp(CAPABILITY_SPOKE_BASELINE, [
        "spoke-baseline",
        "l2-computable",
        "unused-extra",
      ]),
    ).toBe(true);
    expect(tokenAuthorizesOp(CAPABILITY_L2_COMPUTABLE, ["l2-computable"])).toBe(true);
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
    expect(
      tokenAuthorizesOp(requiredCapability("custom-op"), ["spoke-baseline"]),
    ).toBe(false);
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
    expect(dispatchAllowed(TOOL_OP, [TOOL_OP])).toBe(true);
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

describe("memory family token grants (parity with dispatch.rs)", () => {
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

  const GROUPS = [
    {
      ops: MEMORY_CORE_OPS,
      flag: CAPABILITY_SPOKE_MEMORY,
      siblings: [
        CAPABILITY_SPOKE_MEMORY_REFLECTION,
        CAPABILITY_SPOKE_MEMORY_TRANSFER,
      ],
    },
    {
      ops: MEMORY_REFLECTION_OPS,
      flag: CAPABILITY_SPOKE_MEMORY_REFLECTION,
      siblings: [
        CAPABILITY_SPOKE_MEMORY,
        CAPABILITY_SPOKE_MEMORY_TRANSFER,
      ],
    },
    {
      ops: MEMORY_TRANSFER_OPS,
      flag: CAPABILITY_SPOKE_MEMORY_TRANSFER,
      siblings: [
        CAPABILITY_SPOKE_MEMORY,
        CAPABILITY_SPOKE_MEMORY_REFLECTION,
      ],
    },
  ];

  it("authorizes each memory op only for its own gating flag", () => {
    for (const group of GROUPS) {
      for (const op of group.ops) {
        expect(tokenAuthorizesOp(requiredCapability(op), [group.flag]), op).toBe(
          true,
        );
        for (const sibling of group.siblings) {
          expect(
            tokenAuthorizesOp(requiredCapability(op), [sibling]),
            op,
          ).toBe(false);
        }
      }
    }
  });
});

describe("spoke-cognition family (parity with dispatch.rs)", () => {
  it("has one canonical family constant and no op-table row", () => {
    expect(CAPABILITY_SPOKE_COGNITION).toBe("spoke-cognition");
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
    expect(
      cognitionRequirementField("memory.recall", { cognition_filter: null }),
    ).toBe("cognition_filter");
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
    expect(negotiatedCapabilities(["l5-mind"], ["spoke-cognition"])).toEqual([]);
    expect(negotiatedCapabilities(["spoke-cognition"], ["l5-mind"])).toEqual([]);
  });
});
