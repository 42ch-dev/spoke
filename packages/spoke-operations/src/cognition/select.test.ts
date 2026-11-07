import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  CognitionFilter,
  CognitionRecord,
  CognitionSelection,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  cognitionRecordMatchesFilter,
  projectCognitionRecords,
  selectCognitionRecords,
  type CognitionProjection,
} from "./select.js";
import { validateCognitionProjection } from "./validate.js";

// Shared cross-language cognition selection contract (TS ↔ Rust): both
// suites execute every vector id from this single file and must observe the
// same matched record ids, projection summaries or reject codes.
const vectorFile = JSON.parse(
  readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "../../../../fixtures/cognition/selection-vectors.json",
    ),
    "utf8",
  ),
) as { vectors: SelectionVector[] };

interface SelectionVector {
  id: string;
  case: "select" | "project";
  records: CognitionRecord[];
  filter?: CognitionFilter;
  selection?: CognitionSelection;
  policy_selected_record_id?: string | null;
  policy_selects?: "first_input";
  expected:
    | { matched_record_ids: string[] }
    | { error: string }
    | ProjectionExpectation;
}

interface ProjectionExpectation {
  view: "current" | "recorded_at";
  items: ProjectionItemExpectation[];
}

interface ProjectionItemExpectation {
  presence: "assessed" | "no_record";
  holder: [string, string, string];
  target: [[string, string, string], string, string];
  context: [string, string | null];
  assessments: string[];
  selected: string | null;
  policy: [string, string] | null;
  input_record_ids: string[] | null;
}

function projectionSummary(projection: CognitionProjection): ProjectionExpectation {
  return {
    view: projection.view,
    items: projection.items.map((item) => ({
      presence: item.presence,
      holder: [
        item.holder.kind,
        item.holder.namespace,
        item.holder.id,
      ],
      target: [
        [
          item.target.carrier.kind,
          item.target.carrier.namespace,
          item.target.carrier.id,
        ],
        item.target.kind,
        item.target.target_id,
      ],
      context: [
        item.assessment_context.context_id,
        item.assessment_context.fork_id ?? null,
      ],
      assessments: item.assessments.map((record) => record.record_id),
      selected: item.selection?.selected_record_id ?? null,
      policy:
        item.selection === undefined
          ? null
          : [
              item.selection.policy.policy_id,
              item.selection.policy.policy_version,
            ],
      input_record_ids: item.selection?.input_record_ids ?? null,
    })),
  };
}

function runVector(vector: SelectionVector): void {
  if (vector.case === "select") {
    const result = selectCognitionRecords(
      vector.records,
      vector.filter as CognitionFilter,
    );
    if ("error" in vector.expected) {
      expect(result.ok, vector.id).toBe(false);
      if (!result.ok) {
        expect(result.code, vector.id).toBe(vector.expected.error);
      }
      return;
    }
    expect(result.ok, vector.id).toBe(true);
    if (result.ok) {
      expect(
        result.value.map((record) => record.record_id),
        vector.id,
      ).toEqual(vector.expected.matched_record_ids);
    }
    return;
  }
  const policyId = vector.policy_selected_record_id;
  const policy =
    vector.policy_selects === "first_input"
      ? {
          select: ({
            assessments,
          }: {
            assessments: readonly CognitionRecord[];
          }) => assessments[0]!.record_id,
        }
      : policyId === null || policyId === undefined
        ? undefined
        : { select: () => policyId };
  const result = projectCognitionRecords(
    vector.records,
    vector.selection as CognitionSelection,
    policy,
  );
  if ("error" in vector.expected) {
    expect(result.ok, vector.id).toBe(false);
    if (!result.ok) {
      expect(result.code, vector.id).toBe(vector.expected.error);
    }
    return;
  }
  expect(result.ok, vector.id).toBe(true);
  if (result.ok) {
    expect(projectionSummary(result.value), vector.id).toEqual(
      vector.expected,
    );
    expect(validateCognitionProjection(result.value), vector.id).toBe(true);
  }
}

describe("cognition selection shared vectors", () => {
  it("executes every selection vector id exactly once in TS", () => {
    const executed = new Set<string>();
    for (const vector of vectorFile.vectors) {
      runVector(vector);
      expect(executed.has(vector.id)).toBe(false);
      executed.add(vector.id);
    }
    expect(executed.size).toBe(vectorFile.vectors.length);
  });
});

describe("cognitionRecordMatchesFilter", () => {
  it("never matches across namespaces with the same bare id", () => {
    const record = vectorFile.vectors.find(
      (vector) => vector.id === "reference-cross-system-same-bare-id-filter",
    )!.records[1]!;
    expect(
      cognitionRecordMatchesFilter(record, {
        holder: { kind: "principal", namespace: "system_a", id: "alice" },
      }),
    ).toBe(false);
  });

  it("treats a present field as never absent", () => {
    const record = vectorFile.vectors.find(
      (vector) => vector.id === "truth-absent-matches-missing-excludes-present",
    )!.records[0]!;
    expect(cognitionRecordMatchesFilter(record, { truth: { absent: true } })).toBe(
      false,
    );
  });
});

describe("selectCognitionRecords reject envelope", () => {
  const reference = {
    kind: "knowledge_entry" as const,
    namespace: "world_a",
    id: "kb_mira",
  };

  it("rejects unsupported requirements with INVALID_INPUT and reason", () => {
    const result = selectCognitionRecords(
      [],
      { q_min: 0.5 } as unknown as CognitionFilter,
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        reason: "unsupported_cognition_requirement",
      });
    }
  });

  it("keeps the C5 relative field for an invalid filter", () => {
    const result = selectCognitionRecords(
      [],
      { holder: { ...reference, id: 7 } } as unknown as CognitionFilter,
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toEqual({
        reason: "unsupported_cognition_requirement",
        field: "holder.id",
      });
    }
  });

  it("keeps a quoted member identity in the C5 relative field", () => {
    const result = selectCognitionRecords(
      [],
      { q: { min: 0.5, "a.b": 1 } } as unknown as CognitionFilter,
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toEqual({
        reason: "unsupported_cognition_requirement",
        field: 'q["a.b"]',
      });
    }
  });
});

describe("projectCognitionRecords reject envelope", () => {
  const reference = {
    kind: "knowledge_entry" as const,
    namespace: "world_a",
    id: "kb_mira",
  };
  const policy = { policy_id: "p", policy_version: "v1" };

  it("nests the invalid filter field under the selection", () => {
    const result = projectCognitionRecords(
      [],
      {
        view: "current",
        filter: { holder: { ...reference, id: 7 } },
        selection_policy: policy,
      } as unknown as CognitionSelection,
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toEqual({
        reason: "unsupported_cognition_requirement",
        field: "filter.holder.id",
      });
    }
  });

  it("keeps the C5 relative field for an invalid policy identity", () => {
    const result = projectCognitionRecords(
      [],
      {
        view: "current",
        filter: {},
        selection_policy: { policy_id: "p" },
      } as unknown as CognitionSelection,
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toEqual({
        reason: "unsupported_cognition_requirement",
        field: "selection_policy.policy_version",
      });
    }
  });
});
