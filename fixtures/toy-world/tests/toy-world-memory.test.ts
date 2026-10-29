import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  contentSerializedBytes,
  grantCovers,
  jcsSha256Hex,
  jcsSha256HexExcluding,
  validateGrantIssuance,
  validateMemoryCandidatesReview,
  validateMemoryCapture,
  validateMemoryRecall,
} from "@42ch/spoke-operations";
import type {
  MemoryCandidatesReviewRequest,
  MemoryCandidate,
  MemoryCaptureRequest,
  MemoryCaptureResponse,
  MemoryExportBundle,
  MemoryGrant,
  MemoryRecallRequest,
  MemoryRecallResponse,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  FIXTURE_SCHEMA_MAP,
  FIXTURES_ROOT,
  compileSchemaValidator,
  createSchemaValidator,
} from "./schema-validator.js";

const MAX_PAGE_SIZE = 100;
const MAX_CONTENT_BYTES = 65536;
const NOW = Date.parse("2026-07-23T06:30:00Z");

const SPACE_ID = "spc_tw_harbor";
const MIRA = "toy:character:mira";
const HOST_PRINCIPAL = "prn_tw_host_primary";
const OPERATOR_PRINCIPAL = "prn_tw_cli_operator";
const GATE_CONTEXT = "ctx_tw_customs_gate";
const MARKET_CONTEXT = "ctx_tw_market_square";

const MEMORY_FIXTURES = Object.keys(FIXTURE_SCHEMA_MAP).filter((name) =>
  name.startsWith("mem_tw_"),
);

type CaptureReceipt = Extract<MemoryCaptureResponse, { status: "committed" }>;
type RecallPage = Extract<MemoryRecallResponse, { records: unknown[] }>;
type RestrictedGrant = Extract<MemoryGrant, { context_ids: string[] }>;

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

describe("fixtures/toy-world memory samples", () => {
  const ajv = createSchemaValidator();

  it("covers the memory sample family", () => {
    expect(MEMORY_FIXTURES.length).toBe(16);
  });

  it.each(MEMORY_FIXTURES)("validates %s against its memory schema", (filename) => {
    const validate = compileSchemaValidator(ajv, FIXTURE_SCHEMA_MAP[filename]!);
    const valid = validate(loadFixture(filename));

    expect(validate.errors, JSON.stringify(validate.errors, null, 2)).toBeNull();
    expect(valid).toBe(true);
  });
});

describe("memory.capture scenario", () => {
  const request = loadFixture<MemoryCaptureRequest>("mem_tw_capture_request.json");
  const receipt = loadFixture<CaptureReceipt>("mem_tw_capture_response.json");

  it("passes the provider content limit", () => {
    expect(validateMemoryCapture(request, MAX_CONTENT_BYTES)).toEqual({ ok: true });
  });

  it("rejects content above the provider limit", () => {
    const result = validateMemoryCapture(
      { ...request, content: { text: "x".repeat(MAX_CONTENT_BYTES + 1) } },
      MAX_CONTENT_BYTES,
    );

    expect(result.ok).toBe(false);
    expect(result).toMatchObject({
      code: "INVALID_INPUT",
      details: { field: "content", limit: "max_content_bytes" },
    });
  });

  it("binds the committed receipt to the request payload", () => {
    const digest = jcsSha256Hex(request);

    expect(digest.ok && digest.value).toBe(receipt.payload_digest);
    expect(receipt.result.classified).toBe(1);
    expect(receipt.idempotency_key).toBe(request.idempotency_key);
    expect(receipt.space_id).toBe(request.space_id);
  });

  it("keeps only the content digest on the consumed candidate", () => {
    const candidate = loadFixture<{ content_digest: string; proposed: object }>(
      "mem_tw_candidate_consumed_gate_key.json",
    );
    const digest = jcsSha256Hex(request.content);

    expect(digest.ok && digest.value).toBe(candidate.content_digest);
    expect(candidate.proposed).not.toHaveProperty("content");
  });
});

describe("memory.candidates.review scenario", () => {
  const promote = loadFixture<MemoryCandidatesReviewRequest>(
    "mem_tw_review_promote_request.json",
  );
  const fragment = loadFixture<MemoryCandidatesReviewRequest>(
    "mem_tw_review_fragment_request.json",
  );

  it("accepts the promote and fragment decisions", () => {
    expect(validateMemoryCandidatesReview(promote)).toEqual({ ok: true });
    expect(validateMemoryCandidatesReview(fragment)).toEqual({ ok: true });
  });

  it("rejects a fragment decision with no fragments", () => {
    const result = validateMemoryCandidatesReview({
      ...fragment,
      decision: { action: "fragment", fragments: [] },
    } as MemoryCandidatesReviewRequest);

    expect(result.ok).toBe(false);
    expect(result).toMatchObject({
      code: "INVALID_INPUT",
      details: { field: "decision.fragments" },
    });
  });
});

describe("grant coverage scenario", () => {
  const admin = loadFixture<MemoryGrant>("mem_tw_grant_operator_admin.json");
  const restricted = loadFixture<MemoryGrant>(
    "mem_tw_grant_mira_context.json",
  ) as RestrictedGrant;
  const recallNeed = {
    capability: "recall",
    subjectId: MIRA,
    contextIds: [GATE_CONTEXT],
    readsShared: true,
    subjectAll: false,
  };

  it("covers recall inside the granted context", () => {
    expect(
      grantCovers(restricted, HOST_PRINCIPAL, SPACE_ID, NOW, recallNeed),
    ).toBe(true);
  });

  it("fails coverage for a context outside the grant", () => {
    expect(
      grantCovers(restricted, HOST_PRINCIPAL, SPACE_ID, NOW, {
        ...recallNeed,
        contextIds: [MARKET_CONTEXT],
      }),
    ).toBe(false);
  });

  it("never covers subject_all through a restricted grant", () => {
    expect(
      grantCovers(restricted, HOST_PRINCIPAL, SPACE_ID, NOW, {
        ...recallNeed,
        contextIds: [],
        readsShared: false,
        subjectAll: true,
      }),
    ).toBe(false);
    expect(
      grantCovers(admin, OPERATOR_PRINCIPAL, SPACE_ID, NOW, {
        capability: "recall",
        subjectId: null,
        contextIds: [],
        readsShared: true,
        subjectAll: true,
      }),
    ).toBe(true);
  });

  const issued = {
    capabilities: ["recall"],
    subjectId: MIRA,
    contextIds: [GATE_CONTEXT],
    sharedAccess: true,
    expiresAt: null,
  };

  it("admits issuance within admin coverage and rejects it otherwise", () => {
    expect(
      validateGrantIssuance([admin], OPERATOR_PRINCIPAL, SPACE_ID, NOW, issued),
    ).toEqual({ ok: true });

    const denied = validateGrantIssuance(
      [restricted],
      HOST_PRINCIPAL,
      SPACE_ID,
      NOW,
      issued,
    );

    expect(denied.ok).toBe(false);
    expect(denied).toMatchObject({ code: "NOT_AUTHORIZED" });
  });
});

describe("memory.recall scenario", () => {
  const request = loadFixture<MemoryRecallRequest>("mem_tw_recall_request.json");
  const page = loadFixture<RecallPage>("mem_tw_recall_response.json");

  it("passes the provider page limit", () => {
    expect(validateMemoryRecall(request, MAX_PAGE_SIZE)).toEqual({ ok: true });
  });

  it("rejects a budget above the provider page limit", () => {
    const result = validateMemoryRecall(
      { ...request, budget: { ...request.budget, max_items: MAX_PAGE_SIZE + 1 } },
      MAX_PAGE_SIZE,
    );

    expect(result.ok).toBe(false);
    expect(result).toMatchObject({
      code: "INVALID_INPUT",
      details: { field: "max_items", limit: "max_page_size" },
    });
  });

  it("keeps the page inside the budget and reports truncation", () => {
    expect(page.records).toHaveLength(request.budget.max_items);
    expect(page.total_authorized).toBeGreaterThan(page.records.length);
    expect(page.truncated).toBe(true);
  });

  it("defers the max_items overflow without an overflow_record_id", () => {
    // §9: overflow_record_id marks an oversized first item on an otherwise
    // empty page, never an item deferred by the max_items cap.
    expect(page).not.toHaveProperty("overflow_record_id");
  });

  it("surfaces an oversized first item whole on an empty page", () => {
    const overflowRequest = loadFixture<MemoryRecallRequest>(
      "mem_tw_recall_overflow_request.json",
    );
    const overflowPage = loadFixture<RecallPage>(
      "mem_tw_recall_overflow_response.json",
    );

    expect(validateMemoryRecall(overflowRequest, MAX_PAGE_SIZE)).toEqual({ ok: true });
    expect(overflowPage.records).toHaveLength(1);

    const [surfaced] = overflowPage.records;
    const content: { text: string; structured?: unknown } = surfaced!.content;
    const contentBytes = contentSerializedBytes(content);

    expect(contentBytes).toBeGreaterThan(overflowRequest.budget.max_bytes!);
    expect(overflowPage.overflow_record_id).toBe(surfaced!.record_id);
    expect(overflowPage.truncated).toBe(true);
    expect(overflowPage.total_authorized).toBeGreaterThan(1);
  });

  it("returns only records the selection authorizes", () => {
    const selected = new Set(
      request.selection.mode === "selected_contexts"
        ? request.selection.context_ids
        : [],
    );

    for (const record of page.records) {
      expect(record.space_id).toBe(SPACE_ID);
      expect(record.subject_id).toBe(MIRA);
      const scope: { visibility: string; context_id?: string } = record.scope;
      const inSelection =
        scope.visibility === "shared" || selected.has(scope.context_id!);
      expect(inSelection).toBe(true);
    }
  });
});

describe("memory export bundle scenario", () => {
  const bundle = loadFixture<MemoryExportBundle>("mem_tw_export_bundle.json");
  const captureRequest = loadFixture<MemoryCaptureRequest>(
    "mem_tw_capture_request.json",
  );
  const reviewRequest = loadFixture<MemoryCandidatesReviewRequest>(
    "mem_tw_review_fragment_request.json",
  );

  it("carries a digest over the rest of the bundle", () => {
    const digest = jcsSha256HexExcluding(bundle, "digest");

    expect(digest.ok && digest.value).toBe(bundle.digest);
  });

  it("pins receipt summaries to the request payloads they committed", () => {
    const [capture, review] = bundle.receipt_summaries;
    const captureDigest = jcsSha256Hex(captureRequest);
    const reviewDigest = jcsSha256Hex(reviewRequest);

    expect(capture.payload_digest).toBe(captureDigest.ok ? captureDigest.value : "");
    expect(review.payload_digest).toBe(reviewDigest.ok ? reviewDigest.value : "");
  });

  it("exports consumed candidates as scrubbed shells", () => {
    const consumed = bundle.candidates.filter(
      (candidate): candidate is Extract<MemoryCandidate, { status: "consumed" }> =>
        candidate.status === "consumed",
    );

    expect(consumed).not.toHaveLength(0);
    for (const candidate of consumed) {
      expect(candidate.proposed).not.toHaveProperty("content");
      expect(candidate.content_digest).toMatch(/^[0-9a-f]{64}$/);
    }
  });

  it("exports redacted grant summaries only", () => {
    for (const grant of bundle.grants) {
      expect(grant).not.toHaveProperty("principal_id");
      expect(grant).not.toHaveProperty("space_id");
      expect(grant.principal_ref).toBeTruthy();
    }
  });
});
