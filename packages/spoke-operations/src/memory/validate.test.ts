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
  contentWireBytes,
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
  it("contentWireBytes counts text and structured", () => {
    expect(contentWireBytes("héllo", undefined)).toBe(6);
    const structured = { a: 1 };
    const structuredBytes = new TextEncoder().encode(JSON.stringify(structured)).length;
    expect(contentWireBytes("ab", structured)).toBe(2 + structuredBytes);
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
