import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { validateMindState } from "../mind-state/validate.js";
import type { SpokeResult } from "../result.js";
import {
  validateCognitionContent,
  validateCognitionFilter,
  validateCognitionModule,
  validateCognitionProjection,
  validateCognitionRecord,
  validateCognitionSelection,
  validateObservationModule,
} from "./validate.js";

// Shared cross-language cognition validator contract (TS ↔ Rust): both
// suites execute every vector id from this single file by shape and must
// record the same accept/reject result. An unknown shape is an explicit
// failure, never a skip.
const vectorFile = JSON.parse(
  readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "../../../../fixtures/cognition/validator-vectors.json",
    ),
    "utf8",
  ),
) as { vectors: VectorCase[] };

interface VectorCase {
  id: string;
  shape: string;
  input: unknown;
  expected: "accept" | "reject";
  observation_state?: string;
}

// Every vector is dispatched directly to its declared guard.
const SHAPES: Record<string, (input: unknown) => boolean> = {
  "cognition-content": validateCognitionContent,
  "cognition-record": validateCognitionRecord,
  "cognition-module": validateCognitionModule,
  "observation-module": validateObservationModule,
  "cognition-filter": (input) => validateCognitionFilter(input).ok,
  "cognition-selection": (input) => validateCognitionSelection(input).ok,
  "cognition-projection": validateCognitionProjection,
  "mind-state": validateMindState,
};

function runVector(vector: VectorCase): boolean {
  if (vector.shape === "timeline-event-observation") {
    const event = vector.input as { modules: Record<string, unknown> };
    expect(event.modules).toBeTypeOf("object");
    switch (vector.observation_state) {
      case "absent":
        expect(event.modules).not.toHaveProperty("observation");
        return true;
      case "empty":
        expect(event.modules).toHaveProperty("observation");
        expect(event.modules.observation).toEqual({ observers: [] });
        return validateObservationModule(event.modules.observation);
      case "malformed":
        expect(event.modules).toHaveProperty("observation");
        expect(event.modules.observation).toEqual({});
        return validateObservationModule(event.modules.observation);
      default:
        throw new Error(`unsupported observation state: ${vector.observation_state}`);
    }
  }
  const guard = SHAPES[vector.shape];
  if (guard === undefined) {
    throw new Error(`unsupported vector shape: ${vector.shape}`);
  }
  return guard(vector.input);
}

describe("shared cognition validator vectors", () => {
  it.each(vectorFile.vectors)("$id", (vector) => {
    expect(["accept", "reject"]).toContain(vector.expected);
    if (vector.id === "perception-valid") {
      expect(vector.input).toMatchObject({
        kind: "perception",
        q: { assertion: "actual_capture" },
      });
    }
    expect(runVector(vector)).toBe(vector.expected === "accept");
  });
});

describe("cognition context fork identity", () => {
  it.each([
    ["", false],
    ["fork_1", true],
    [" ", true],
  ])("validates record fork_id %j", (forkId, expected) => {
    const record = vectorFile.vectors.find((vector) => vector.id === "belief-order1-valid")!
      .input as Record<string, unknown>;
    expect(validateCognitionRecord({
      ...record,
      assessment_context: { context_id: "ctx_default", fork_id: forkId },
    })).toBe(expected);
  });

  it("rejects an empty fork_id in a metric context", () => {
    const record = structuredClone(
      vectorFile.vectors.find((vector) => vector.id === "belief-metric-b-valid")!.input,
    ) as { metrics: { b: { context: { fork_id?: string } } } };
    record.metrics.b.context.fork_id = "";
    expect(validateCognitionRecord(record)).toBe(false);
  });

  it("rejects an empty fork_id in a projection item context", () => {
    const projection = structuredClone(
      vectorFile.vectors.find((vector) => vector.id === "projection-current-assessed-valid")!.input,
    ) as { items: { assessment_context: { fork_id?: string } }[] };
    projection.items[0]!.assessment_context.fork_id = "";
    expect(validateCognitionProjection(projection)).toBe(false);
  });
});

type BeliefFixture = {
  holder: { id: string };
  target: { target_id: string };
  assessment_context: { context_id: string };
  record_id: string;
  q: { method: string };
};

describe("cognition nonempty wire values", () => {
  it.each([
    ["holder.id", (record: BeliefFixture, value: string) => { record.holder.id = value; }],
    ["target.target_id", (record: BeliefFixture, value: string) => { record.target.target_id = value; }],
    ["assessment_context.context_id", (record: BeliefFixture, value: string) => { record.assessment_context.context_id = value; }],
    ["record_id", (record: BeliefFixture, value: string) => { record.record_id = value; }],
    ["q.method", (record: BeliefFixture, value: string) => { record.q.method = value; }],
  ])("checks %s without trimming", (_field, setValue) => {
    const original = vectorFile.vectors.find((vector) => vector.id === "belief-order1-valid")!.input;
    for (const [value, expected] of [[" ", true], ["", false]] as const) {
      const record = structuredClone(original) as BeliefFixture;
      setValue(record, value);
      expect(validateCognitionRecord(record)).toBe(expected);
    }
  });

  it("checks filter context_id without trimming", () => {
    expect(validateCognitionFilter({ assessment_context: { context_id: " " } }).ok).toBe(true);
    expect(validateCognitionFilter({ assessment_context: { context_id: "" } }).ok).toBe(false);
  });

  it("checks filter fork_id eq without trimming", () => {
    expect(
      validateCognitionFilter({ assessment_context: { fork_id: { eq: " " } } }).ok,
    ).toBe(true);
    expect(
      validateCognitionFilter({ assessment_context: { fork_id: { eq: "" } } }).ok,
    ).toBe(false);
  });
});

describe("isolated cognition boundaries", () => {
  it("accepts the same epistemic target at the maximum depth", () => {
    const record = structuredClone(
      vectorFile.vectors.find((vector) => vector.id === "epistemic-target-chain-exceeds-three-reject")!.input,
    ) as { target: { about: { about: { about?: unknown } } } };
    delete record.target.about.about.about;
    expect(validateCognitionRecord(record)).toBe(true);
  });

  it("accepts the same no-record projection without selection", () => {
    const projection = structuredClone(
      vectorFile.vectors.find((vector) => vector.id === "projection-current-no-record-with-selection-reject")!.input,
    ) as { items: { selection?: unknown }[] };
    delete projection.items[0]!.selection;
    expect(validateCognitionProjection(projection)).toBe(true);
  });
});

// C5 is the sole authority for the relative member at which a filter or
// selection is rejected: the Connect responders only prefix the carrier
// location. Each case below fails for exactly one member.
describe("C5 relative diagnostic field", () => {
  const expectRejectField = (
    result: SpokeResult<void>,
    field?: string,
  ): void => {
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toEqual(field === undefined ? undefined : { field });
    }
  };
  const reference = {
    kind: "knowledge_entry",
    namespace: "world_a",
    id: "kb_mira",
  };

  it("names the invalid reference member, never a valid sibling", () => {
    expectRejectField(
      validateCognitionFilter({ holder: { ...reference, id: 7 } }),
      "holder.id",
    );
    expectRejectField(
      validateCognitionFilter({ holder: { ...reference, namespace: 42 } }),
      "holder.namespace",
    );
    expectRejectField(
      validateCognitionFilter({ holder: { ...reference, kind: "actor" } }),
      "holder.kind",
    );
  });

  it("names a missing required reference member", () => {
    const { kind: _kind, ...withoutKind } = reference;
    expectRejectField(
      validateCognitionFilter({ holder: withoutKind }),
      "holder.kind",
    );
  });

  it("names the failing carrier member under target", () => {
    expectRejectField(
      validateCognitionFilter({
        target: { carrier: { ...reference, id: "" } },
      }),
      "target.carrier.id",
    );
  });

  it("names unknown predicates at their own path", () => {
    expectRejectField(validateCognitionFilter({ q_min: 0.5 }), "q_min");
    expectRejectField(
      validateCognitionFilter({ q: { min: 0.5, extra: 1 } }),
      "q.extra",
    );
  });

  it("names an unknown member even when a categorical arm is also present", () => {
    expectRejectField(
      validateCognitionFilter({ truth: { eq: "True", extra: 1 } }),
      "truth.extra",
    );
    expectRejectField(
      validateCognitionFilter({
        assessment_context: { fork_id: { eq: "fork_1", extra: 1 } },
      }),
      "assessment_context.fork_id.extra",
    );
    expectRejectField(
      validateCognitionFilter({ truth: { absent: true, extra: 1 } }),
      "truth.extra",
    );
  });

  it("names the enclosing member for a cross-field failure", () => {
    expectRejectField(
      validateCognitionFilter({ q: { min: 0.9, max: 0.1 } }),
      "q",
    );
    expectRejectField(
      validateCognitionFilter({ truth: { eq: "True", absent: true } }),
      "truth",
    );
    expectRejectField(
      validateCognitionFilter({
        assessment_context: { fork_id: { eq: "fork_1", absent: true } },
      }),
      "assessment_context.fork_id",
    );
  });

  it("reports a failed root without a relative field", () => {
    expectRejectField(validateCognitionFilter(7));
    expectRejectField(validateCognitionSelection(["not", "an", "object"]));
  });

  it("nests filter and policy failures under the selection", () => {
    expectRejectField(
      validateCognitionSelection({
        view: "current",
        filter: { q_min: 0.5 },
        selection_policy: { policy_id: "p", policy_version: "v1" },
      }),
      "filter.q_min",
    );
    expectRejectField(
      validateCognitionSelection({
        view: "current",
        filter: {},
        selection_policy: { policy_id: "p" },
      }),
      "selection_policy.policy_version",
    );
    expectRejectField(
      validateCognitionSelection({ view: "current", filter: {} }),
      "selection_policy",
    );
    expectRejectField(
      validateCognitionSelection({
        view: "recorded_at",
        filter: {},
        selection_policy: { policy_id: "p", policy_version: "v1" },
      }),
      "selection_policy",
    );
    expectRejectField(
      validateCognitionSelection({ view: "future", filter: {} }),
      "view",
    );
  });

  it("keeps empty and non-identifier members distinct from root failure", () => {
    // A root that is not an object fails without a relative field; an empty
    // member is still a member and keeps its own identity.
    expectRejectField(validateCognitionFilter(7));
    expectRejectField(validateCognitionFilter({ "": 1 }), '[""]');
    expectRejectField(
      validateCognitionFilter({ q: { min: 0.5, "": 1 } }),
      'q[""]',
    );
    expectRejectField(validateCognitionSelection({ "": 1 }), '[""]');
  });

  it("quotes delimiter, quote and backslash member names", () => {
    expectRejectField(
      validateCognitionFilter({ q: { min: 0.5, "a.b": 1 } }),
      'q["a.b"]',
    );
    expectRejectField(
      validateCognitionFilter({ q: { min: 0.5, 'a"b': 1 } }),
      'q["a\\"b"]',
    );
    expectRejectField(
      validateCognitionFilter({ q: { min: 0.5, "a\\b": 1 } }),
      'q["a\\\\b"]',
    );
    expectRejectField(
      validateCognitionFilter({ truth: { eq: "True", "": 1 } }),
      'truth[""]',
    );
  });

  it("keeps recorded_at C5-valid and fork predicates accepted", () => {
    expect(
      validateCognitionSelection({ view: "recorded_at", filter: {} }).ok,
    ).toBe(true);
    expect(
      validateCognitionFilter({
        assessment_context: { fork_id: { eq: "fork_1" } },
        truth: { absent: true },
      }).ok,
    ).toBe(true);
  });
});
