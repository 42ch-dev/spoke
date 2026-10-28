import { describe, expect, it } from "vitest";

import type {
  MemoryCandidatesDismissRequest,
  MemoryCandidatesGetRequest,
  MemoryCandidatesListRequest,
  MemoryCandidatesReviewRequest,
  MemoryCaptureRequest,
  MemoryDescribeRequest,
  MemoryExportsCreateRequest,
  MemoryExportsGetRequest,
  MemoryGrantsGrantRequest,
  MemoryGrantsListRequest,
  MemoryGrantsRevokeRequest,
  MemoryImportsActivateRequest,
  MemoryImportsDiscardRequest,
  MemoryImportsStageRequest,
  MemoryProfileGetRequest,
  MemoryProfileUpdateRequest,
  MemoryRecallRequest,
  MemoryReceiptsGetRequest,
  MemoryRecordsCreateRequest,
  MemoryRecordsDeleteRequest,
  MemoryRecordsGetRequest,
  MemoryRecordsListRequest,
  MemoryRecordsPromoteRequest,
  MemoryRecordsUpdateRequest,
  MemoryReflectionsGenerateRequest,
  MemoryReflectionsListRequest,
  MemoryReflectionsStateRequest,
  MemorySpacesFreezeRequest,
  MemorySpacesListRequest,
  MemorySpacesUnfreezeRequest,
  MemorySubjectsEnsureRequest,
  MemorySubjectsFenceRequest,
  MemorySubjectsGetRequest,
  Principal,
} from "@42ch/spoke-schemas";

import {
  SpokeRejectCode,
  type MemoryPort,
  type MemoryPorts,
  type SpokeResult,
} from "../index.js";

const PRINCIPAL: Principal = {
  principal_id: "p1",
  extensions: {},
};

/** A structural smoke: every one of the 33 methods is callable via the type. */
function methodArity(port: MemoryPort): number {
  return [
    port.describe,
    port.listSpaces,
    port.freezeSpace,
    port.unfreezeSpace,
    port.ensureSubject,
    port.getSubject,
    port.fenceSubject,
    port.listGrants,
    port.grant,
    port.revokeGrant,
    port.capture,
    port.listCandidates,
    port.getCandidate,
    port.dismissCandidate,
    port.reviewCandidate,
    port.createRecord,
    port.getRecord,
    port.updateRecord,
    port.deleteRecord,
    port.listRecords,
    port.promoteRecord,
    port.recall,
    port.getProfile,
    port.updateProfile,
    port.getReflectionState,
    port.listReflections,
    port.generateReflection,
    port.getReceipt,
    port.createExport,
    port.getExport,
    port.stageImport,
    port.activateImport,
    port.discardImport,
  ].length;
}

describe("MemoryPort / MemoryPorts (TS parity with the Rust trait)", () => {
  it("exposes 33 methods taking Principal as the explicit first parameter", () => {
    const port: MemoryPort = new Proxy({} as MemoryPort, {
      get: (_target, property) => {
        if (property === "then") {
          return undefined;
        }
        return (_principal: Principal, _request: unknown): Promise<SpokeResult<never>> =>
          Promise.reject(new Error(`unexpected call: ${String(property)}`));
      },
    });
    expect(methodArity(port)).toBe(33);
  });

  it("probe defaults to undefined for adapters that do not serve memory", () => {
    const adapter: MemoryPorts = {};
    expect(adapter.asMemory?.()).toBeUndefined();
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

// Type-only imports must stay in sync with the Rust trait surface.
type _RequestTypes = [
  MemoryDescribeRequest,
  MemorySpacesListRequest,
  MemorySpacesFreezeRequest,
  MemorySpacesUnfreezeRequest,
  MemorySubjectsEnsureRequest,
  MemorySubjectsGetRequest,
  MemorySubjectsFenceRequest,
  MemoryGrantsListRequest,
  MemoryGrantsGrantRequest,
  MemoryGrantsRevokeRequest,
  MemoryCaptureRequest,
  MemoryCandidatesListRequest,
  MemoryCandidatesGetRequest,
  MemoryCandidatesDismissRequest,
  MemoryCandidatesReviewRequest,
  MemoryRecordsCreateRequest,
  MemoryRecordsGetRequest,
  MemoryRecordsUpdateRequest,
  MemoryRecordsDeleteRequest,
  MemoryRecordsListRequest,
  MemoryRecordsPromoteRequest,
  MemoryRecallRequest,
  MemoryProfileGetRequest,
  MemoryProfileUpdateRequest,
  MemoryReflectionsStateRequest,
  MemoryReflectionsListRequest,
  MemoryReflectionsGenerateRequest,
  MemoryReceiptsGetRequest,
  MemoryExportsCreateRequest,
  MemoryExportsGetRequest,
  MemoryImportsStageRequest,
  MemoryImportsActivateRequest,
  MemoryImportsDiscardRequest,
];
