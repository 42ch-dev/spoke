import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type {
  MemoryCandidatesListRequest,
  MemoryCandidatesReviewRequest,
  MemoryCaptureRequest,
  MemoryGrantsGrantRequest,
  MemoryGrantsListRequest,
  MemoryImportsStageRequest,
  MemoryRecallRequest,
  MemoryRecordsCreateRequest,
  MemoryRecordsListRequest,
  MemoryRecordsUpdateRequest,
  MemoryReflectionsGenerateRequest,
  MemoryReflectionsListRequest,
  MemoryReflectionsStateRequest,
  MemorySpacesListRequest,
  MemorySubjectsFenceRequest,
  ReadSelection,
  ProvenanceFilter,
} from "@42ch/spoke-schemas";

import {
  contentSerializedBytes,
  isFreezeAllowlisted,
  validateMemoryCandidatesList,
  validateMemoryCandidatesReview,
  validateMemoryCapture,
  validateMemoryGrantsGrant,
  validateMemoryGrantsList,
  validateMemoryImportsStage,
  validateMemoryRecall,
  validateMemoryRecordsCreate,
  validateMemoryRecordsList,
  validateMemoryRecordsUpdate,
  validateMemoryReflectionsGenerate,
  validateMemoryReflectionsList,
  validateMemoryReflectionsState,
  validateMemorySpacesList,
  validateMemorySubjectsFence,
} from "./validate.js";

const MAX_PAGE = 100;

function expectInvalid(
  result: { ok: boolean; code?: string; details?: Record<string, unknown> },
  field: string,
): void {
  expect(result.ok).toBe(false);
  expect(result.code).toBe("INVALID_INPUT");
  expect(result.details?.field).toBe(field);
}

function expectLimit(
  result: { ok: boolean; code?: string; details?: Record<string, unknown> },
  limit: string,
): void {
  expect(result.ok).toBe(false);
  expect(result.code).toBe("INVALID_INPUT");
  expect(result.details?.limit).toBe(limit);
}

function expectInvalidCognition(
  result: { ok: boolean; code?: string; details?: Record<string, unknown> },
): void {
  expect(result.ok).toBe(false);
  expect(result.code).toBe("INVALID_INPUT");
  expect(result.details?.field).toBe("content.cognition");
  expect(result.details?.reason).toBe("invalid_cognition_record");
}

describe("isFreezeAllowlisted (§8.2)", () => {
  it("admits reads and maintenance ops", () => {
    for (const op of [
      "memory.describe",
      "memory.spaces.list",
      "memory.subjects.get",
      "memory.grants.list",
      "memory.candidates.list",
      "memory.candidates.get",
      "memory.records.get",
      "memory.records.list",
      "memory.recall",
      "memory.profile.get",
      "memory.reflections.state",
      "memory.reflections.list",
      "memory.receipts.get",
      "memory.exports.get",
      "memory.spaces.unfreeze",
      "memory.grants.revoke",
      "memory.exports.create",
      "memory.imports.stage",
      "memory.imports.activate",
      "memory.imports.discard",
    ]) {
      expect(isFreezeAllowlisted(op)).toBe(true);
    }
  });

  it("rejects other mutations and unknown ops", () => {
    for (const op of [
      "memory.spaces.freeze",
      "memory.subjects.ensure",
      "memory.subjects.fence",
      "memory.grants.grant",
      "memory.capture",
      "memory.candidates.dismiss",
      "memory.candidates.review",
      "memory.records.create",
      "memory.records.update",
      "memory.records.delete",
      "memory.records.promote",
      "memory.profile.update",
      "memory.reflections.generate",
      "memory.extract",
    ]) {
      expect(isFreezeAllowlisted(op)).toBe(false);
    }
  });
});

describe("page limits (§7.0.7)", () => {
  it("rejects limit above max_page_size with details.limit", () => {
    const ok = { limit: 100 } as MemorySpacesListRequest;
    expect(validateMemorySpacesList(ok, MAX_PAGE).ok).toBe(true);
    const tooBig = { limit: 101 } as MemorySpacesListRequest;
    expectLimit(validateMemorySpacesList(tooBig, MAX_PAGE), "max_page_size");

    const grants = { limit: 101 } as MemoryGrantsListRequest;
    expectLimit(validateMemoryGrantsList(grants, MAX_PAGE), "max_page_size");
    const candidates = { limit: 101 } as MemoryCandidatesListRequest;
    expectLimit(validateMemoryCandidatesList(candidates, MAX_PAGE), "max_page_size");
  });

  it("recall rejects budget.max_items above max_page_size", () => {
    const ok = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: { mode: "shared_only" },
      budget: { max_items: 100 },
    } as MemoryRecallRequest;
    expect(validateMemoryRecall(ok, MAX_PAGE).ok).toBe(true);

    const tooBig = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: { mode: "shared_only" },
      budget: { max_items: 101 },
    } as MemoryRecallRequest;
    expectLimit(validateMemoryRecall(tooBig, MAX_PAGE), "max_page_size");
  });

  it("requires positive integer limits before the upper bound", () => {
    for (const bad of [0, -1, 1.5]) {
      expectInvalid(
        validateMemorySpacesList({ limit: bad } as MemorySpacesListRequest, MAX_PAGE),
        "limit",
      );
      expectInvalid(
        validateMemoryGrantsList({ limit: bad } as MemoryGrantsListRequest, MAX_PAGE),
        "limit",
      );
      expectInvalid(
        validateMemoryCandidatesList({ limit: bad } as MemoryCandidatesListRequest, MAX_PAGE),
        "limit",
      );

      const recall = {
        space_id: "sp_1",
        subject_id: "s1",
        selection: { mode: "shared_only" },
        budget: { max_items: bad },
      } as MemoryRecallRequest;
      expectInvalid(validateMemoryRecall(recall, MAX_PAGE), "max_items");
    }

    expect(validateMemorySpacesList({ limit: 1 } as MemorySpacesListRequest, MAX_PAGE).ok).toBe(
      true,
    );
    const recallOne = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: { mode: "shared_only" },
      budget: { max_items: 1 },
    } as MemoryRecallRequest;
    expect(validateMemoryRecall(recallOne, MAX_PAGE).ok).toBe(true);
  });
});

describe("selection / filter invariants (§6.12, §6.13)", () => {
  const emptyContexts: ReadSelection = {
    mode: "selected_contexts",
    context_ids: [] as unknown as [string, ...string[]],
  };
  const emptyFilter: ProvenanceFilter = {
    source_context_ids: [] as unknown as [string, ...string[]],
  };

  it("rejects empty selected_contexts in recall", () => {
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: emptyContexts,
      budget: { max_items: 10 },
    } as MemoryRecallRequest;
    expectInvalid(validateMemoryRecall(request, MAX_PAGE), "selection.context_ids");
  });

  it("rejects empty provenance filter in records.list", () => {
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: { mode: "subject_all" },
      filter: emptyFilter,
    } as MemoryRecordsListRequest;
    expectInvalid(validateMemoryRecordsList(request, MAX_PAGE), "filter.source_context_ids");
  });

  it("rejects empty selected_contexts in reflections.state", () => {
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: emptyContexts,
    } as MemoryReflectionsStateRequest;
    expectInvalid(validateMemoryReflectionsState(request), "selection.context_ids");
  });

  it("rejects empty provenance filter in reflections.generate", () => {
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      selection: { mode: "shared_only" },
      filter: emptyFilter,
      expected_state_digest: "d".repeat(64),
      idempotency_key: "k1",
    } as MemoryReflectionsGenerateRequest;
    expectInvalid(validateMemoryReflectionsGenerate(request), "filter.source_context_ids");
  });

  it("validates optional selection/filter and limit in reflections.list", () => {
    const ok = {
      space_id: "sp_1",
      subject_id: "s1",
      limit: 50,
    } as MemoryReflectionsListRequest;
    expect(validateMemoryReflectionsList(ok, MAX_PAGE).ok).toBe(true);

    const tooBig = { ...ok, limit: 101 } as MemoryReflectionsListRequest;
    expectLimit(validateMemoryReflectionsList(tooBig, MAX_PAGE), "max_page_size");
  });
});

describe("per-op conditionals", () => {
  it("subjects.fence rejects archive on context target (§7.7)", () => {
    const archiveContext = {
      space_id: "sp_1",
      subject_id: "s1",
      target: { type: "context", context_id: "c1" },
      action: "archive",
      expected_revision: 1,
      idempotency_key: "k1",
    } as MemorySubjectsFenceRequest;
    expectInvalid(validateMemorySubjectsFence(archiveContext), "target");

    const fenceContext = { ...archiveContext, action: "fence" } as MemorySubjectsFenceRequest;
    expect(validateMemorySubjectsFence(fenceContext).ok).toBe(true);

    const archiveSubject = {
      ...archiveContext,
      target: { type: "subject" },
    } as MemorySubjectsFenceRequest;
    expect(validateMemorySubjectsFence(archiveSubject).ok).toBe(true);
  });

  it("grants.grant rejects empty/duplicate capabilities and empty context_ids (§7.9)", () => {
    const restricted = {
      space_id: "sp_1",
      principal_id: "p2",
      capabilities: ["capture"],
      context_ids: ["c1"],
      idempotency_key: "k1",
    } as MemoryGrantsGrantRequest;

    expectInvalid(
      validateMemoryGrantsGrant({
        ...restricted,
        capabilities: [],
      } as MemoryGrantsGrantRequest),
      "capabilities",
    );
    expectInvalid(
      validateMemoryGrantsGrant({
        ...restricted,
        capabilities: ["capture", "capture"],
      } as MemoryGrantsGrantRequest),
      "capabilities",
    );
    expectInvalid(
      validateMemoryGrantsGrant({
        ...restricted,
        context_ids: [],
      } as MemoryGrantsGrantRequest),
      "context_ids",
    );

    const unrestricted = {
      space_id: "sp_1",
      principal_id: "p2",
      capabilities: ["capture"],
      idempotency_key: "k1",
    } as MemoryGrantsGrantRequest;
    expect(validateMemoryGrantsGrant(unrestricted).ok).toBe(true);
    expectInvalid(
      validateMemoryGrantsGrant({ ...unrestricted, capabilities: [] } as MemoryGrantsGrantRequest),
      "capabilities",
    );
  });

  it("candidates.review rejects empty fragment list (§7.15)", () => {
    const request = {
      space_id: "sp_1",
      candidate_id: "cand_1",
      expected_revision: 1,
      decision: { action: "fragment", fragments: [] },
      idempotency_key: "k1",
    } as MemoryCandidatesReviewRequest;
    expectInvalid(validateMemoryCandidatesReview(request), "decision.fragments");
  });

  it("records.update accepts kind-only, content-only, and combined (§7.18)", () => {
    const combined = {
      space_id: "sp_1",
      record_id: "r1",
      expected_revision: 1,
      kind: "note",
      content: { text: "combined update" },
      idempotency_key: "k1",
    } as MemoryRecordsUpdateRequest;
    expect(validateMemoryRecordsUpdate(combined, 1024).ok).toBe(true);

    const kindOnly = {
      space_id: "sp_1",
      record_id: "r1",
      expected_revision: 1,
      kind: "note",
      idempotency_key: "k1",
    } as MemoryRecordsUpdateRequest;
    expect(validateMemoryRecordsUpdate(kindOnly, 1024).ok).toBe(true);

    const contentOnly = {
      space_id: "sp_1",
      record_id: "r1",
      expected_revision: 1,
      content: { text: "fine" },
      idempotency_key: "k1",
    } as MemoryRecordsUpdateRequest;
    expect(validateMemoryRecordsUpdate(contentOnly, 1024).ok).toBe(true);

    // Neither present (runtime bypass of the generated oneOf) → reject.
    const neither = {
      space_id: "sp_1",
      record_id: "r1",
      expected_revision: 1,
      idempotency_key: "k1",
    } as unknown as MemoryRecordsUpdateRequest;
    expectInvalid(validateMemoryRecordsUpdate(neither, 1024), "content");

    // Content still rides the provider byte limit on the combined shape.
    const oversize = {
      ...combined,
      content: { text: "x".repeat(2048) },
    } as MemoryRecordsUpdateRequest;
    expectLimit(validateMemoryRecordsUpdate(oversize, 1024), "max_content_bytes");
  });

  it("imports.stage rejects non-zero header chunk ordinal (§7.31)", () => {
    const request = {
      space_id: "sp_1",
      chunk: {
        export_id: "e1",
        chunk_ordinal: 1,
        chunk_count: 2,
        bundle_digest: "b".repeat(64),
        chunk_digest: "c".repeat(64),
        bytes: 10,
      },
      conflict_policy: "fail",
      idempotency_key: "k1",
    } as unknown as MemoryImportsStageRequest;
    expectInvalid(validateMemoryImportsStage(request), "chunk.chunk_ordinal");
  });
});

describe("content byte limits (§7.1)", () => {
  it("contentSerializedBytes pins the shared known vectors", () => {
    // Same explicit byte values are pinned by the Rust parity test.
    expect(contentSerializedBytes({ text: "héllo" })).toBe(17);
    expect(contentSerializedBytes({ text: "ab", structured: { nested: [true, null] } })).toBe(49);
    expect(contentSerializedBytes({ text: "" })).toBe(11);
  });

  it("contentSerializedBytes canonicalizes explicit null like Rust", () => {
    // Rust deserializes `structured: null` to `None` and skips it; both
    // languages measure `{"text":""}` = 11.
    expect(contentSerializedBytes({ text: "", structured: null })).toBe(11);
  });

  it("contentSerializedBytes pins the ECMAScript number form like Rust", () => {
    // Rust measures through RFC 8785 JCS: `1.0` → `1`, `1e2` → `100`.
    expect(contentSerializedBytes({ text: "", structured: { n: 1.0, e: 1e2 } })).toBe(40);
  });

  it("capture admits explicit null content at the exact boundary", () => {
    // `{"text":"","structured":null}` canonicalizes to `{"text":""}`,
    // 11 bytes; the Rust parity test pins the same pair.
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      content: { text: "", structured: null },
      capture_scope: { visibility: "shared" },
      idempotency_key: "k1",
    } as MemoryCaptureRequest;
    expect(validateMemoryCapture(request, 11).ok).toBe(true);
    expectLimit(validateMemoryCapture(request, 10), "max_content_bytes");
  });

  it("capture rejects oversize content with details.limit", () => {
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      content: { text: "x".repeat(2048) },
      capture_scope: { visibility: "shared" },
      idempotency_key: "k1",
    } as MemoryCaptureRequest;
    expectLimit(validateMemoryCapture(request, 1024), "max_content_bytes");

    const small = {
      ...request,
      content: { text: "fine" },
      capture_scope: { visibility: "context", context_id: "c1" },
    } as MemoryCaptureRequest;
    expect(validateMemoryCapture(small, 1024).ok).toBe(true);
  });

  it("capture accepts the exact content byte boundary", () => {
    // `{"text":"fine"}` is exactly 15 bytes under the serialized measure.
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      content: { text: "fine" },
      capture_scope: { visibility: "shared" },
      idempotency_key: "k1",
    } as MemoryCaptureRequest;
    expect(validateMemoryCapture(request, 15).ok).toBe(true);
    expectLimit(validateMemoryCapture(request, 14), "max_content_bytes");
  });

  it("records.create rejects oversize content", () => {
    const request = {
      space_id: "sp_1",
      subject_id: "s1",
      kind: "note",
      content: { text: "x".repeat(2048) },
      scope: { visibility: "shared" },
      idempotency_key: "k1",
    } as MemoryRecordsCreateRequest;
    expectLimit(validateMemoryRecordsCreate(request, 1024), "max_content_bytes");
  });
});

// Shared cross-language C8 lifecycle vectors: the TS measure and validators
// must observe the same exact canonical admission bytes and the same
// accept/reject outcome as the Rust suite for every content-admission vector.
const lifecycleVectors = JSON.parse(
  readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "../../../../fixtures/cognition/memory-lifecycle-vectors.json",
    ),
    "utf8",
  ),
) as { vectors: LifecycleVector[] };

interface LifecycleVector {
  id: string;
  shape: string;
  content: { text: string; cognition?: unknown[] };
  canonical_bytes: number;
  max_content_bytes: number;
  expected: "accept" | "reject";
  reason?: "max_content_bytes" | "invalid_cognition_record";
}

describe("cognition lifecycle shared admission vectors (C8)", () => {
  const admission = lifecycleVectors.vectors.filter(
    (vector) => vector.shape === "content-admission",
  );
  // The handoff from P1/T1 L2 requires a valid non-empty recorded-at-time
  // carrier: every admission vector must exercise the real boundary.
  expect(admission.length).toBeGreaterThanOrEqual(5);
  const valid = admission.find((vector) => vector.id === "lifecycle-admission-cognition-valid");
  expect(valid?.content.cognition?.length).toBeGreaterThan(0);

  it.each(admission.map((vector) => [vector.id, vector] as const))(
    "%s measures the exact shared canonical bytes",
    (_id, vector) => {
      expect(contentSerializedBytes(vector.content)).toBe(vector.canonical_bytes);
    },
  );

  it.each(admission.map((vector) => [vector.id, vector] as const))(
    "%s admits or rejects exactly like Rust",
    (_id, vector) => {
      const request = {
        space_id: "sp_1",
        subject_id: "s1",
        content: vector.content,
        capture_scope: { visibility: "shared" },
        idempotency_key: "k1",
      } as MemoryCaptureRequest;
      const result = validateMemoryCapture(request, vector.max_content_bytes);
      if (vector.expected === "accept") {
        expect(result.ok).toBe(true);
        return;
      }
      expect(result.ok).toBe(false);
      expect(result.code).toBe("INVALID_INPUT");
      if (vector.reason === "max_content_bytes") {
        expect(result.details?.limit).toBe("max_content_bytes");
      } else {
        // The cognition guard runs before byte admission: even when the same
        // content is over budget, the malformed carrier rejects on the field
        // with the contract's invalid_cognition_record identity.
        expect(vector.canonical_bytes).toBeGreaterThan(vector.max_content_bytes);
        expect(result.details?.field).toBe("content.cognition");
        expect(result.details?.reason).toBe("invalid_cognition_record");
      }
    },
  );

  it("guard wins over an over-budget admission on create and update", () => {
    // Structurally complete record, only the view is wrong, and the same
    // content exceeds the supplied budget: the cognition rejection must win.
    const complete = valid!.content.cognition![0] as Record<string, unknown>;
    const malformed = {
      text: "fine",
      cognition: [{ ...complete, view: "current" }],
    };
    const budget = contentSerializedBytes(malformed) - 1;
    expect(budget).toBeGreaterThan(0);
    const create = {
      space_id: "sp_1",
      subject_id: "s1",
      kind: "note",
      content: malformed,
      scope: { visibility: "shared" },
      idempotency_key: "k1",
    } as MemoryRecordsCreateRequest;
    expectInvalidCognition(validateMemoryRecordsCreate(create, budget));

    const update = {
      space_id: "sp_1",
      record_id: "r1",
      content: malformed,
      expected_revision: 1,
      idempotency_key: "k1",
    } as unknown as MemoryRecordsUpdateRequest;
    expectInvalidCognition(validateMemoryRecordsUpdate(update, budget));
  });

  it("explicit empty cognition is measured as supplied", () => {
    // The admission budget bounds the complete caller wire value: an
    // explicitly present empty array is 32 bytes, distinct from omission.
    expect(contentSerializedBytes({ text: "Memory" })).toBe(17);
    expect(contentSerializedBytes({ text: "Memory", cognition: [] })).toBe(32);
  });
});
