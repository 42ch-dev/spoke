/**
 * Per-op request validators and pure policy predicates — TS parity with
 * `crates/spoke-operations/src/memory/validate.rs`.
 *
 * Covers only the conditional wire rules the JSON-Schema codegen cannot
 * derive (draft-07 `if`/`then`/`else`), plus provider-limit-dependent
 * checks parameterized from `memory.describe.limits` (§7.1). The TS
 * `minItems` tuple types (`[string, ...string[]]`) are compile-time only —
 * runtime callers can still pass empty arrays, so the invariants are
 * re-checked here exactly as the Rust validators do. Everything else
 * (idempotency-key presence, placement conditionality, oneOf shapes) is
 * type-enforced. Limit rejections carry `details.limit` per §7.0.7.
 */

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
} from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";

type Details = Record<string, unknown>;

function invalidInput(field: string, message: string, extra?: Details): SpokeResult<void> {
  return spokeReject("INVALID_INPUT", message, { field, ...extra });
}

function invalidLimit(field: string, limit: string, message: string): SpokeResult<void> {
  return spokeReject("INVALID_INPUT", message, { field, limit });
}

function checkPageLimit(limit: number | undefined, maxPageSize: number): SpokeResult<void> {
  if (limit !== undefined && limit > maxPageSize) {
    return invalidLimit("limit", "max_page_size", `limit exceeds the provider max_page_size (${maxPageSize})`);
  }
  return spokeOk();
}

/**
 * Byte size of a `Content` request field against `max_content_bytes`
 * (§7.1): UTF-8 bytes of `text` plus the compact serialization of
 * `structured` when present.
 */
export function contentWireBytes(text: string, structured?: unknown): number {
  const encoder = new TextEncoder();
  const structuredBytes =
    structured === undefined ? 0 : encoder.encode(JSON.stringify(structured)).length;
  return encoder.encode(text).length + structuredBytes;
}

function checkContentLimit(byteLen: number, maxContentBytes: number): SpokeResult<void> {
  if (byteLen > maxContentBytes) {
    return invalidLimit(
      "content",
      "max_content_bytes",
      `content exceeds the provider max_content_bytes (${maxContentBytes})`,
    );
  }
  return spokeOk();
}

interface SelectionLike {
  mode: string;
  context_ids?: string[];
}

interface FilterLike {
  source_context_ids?: string[];
}

function checkSelection(selection: SelectionLike | undefined, present: boolean): SpokeResult<void> {
  if (present && selection?.mode === "selected_contexts" && (selection.context_ids ?? []).length === 0) {
    return invalidInput("selection.context_ids", "selected_contexts requires at least one context_id");
  }
  return spokeOk();
}

function checkFilter(filter: FilterLike | undefined): SpokeResult<void> {
  if (filter !== undefined && (filter.source_context_ids ?? []).length === 0) {
    return invalidInput("filter.source_context_ids", "provenance filter requires at least one source context");
  }
  return spokeOk();
}

/**
 * §8.2 freeze maintenance allowlist: while a space is frozen it admits
 * exactly all read-only ops (MEM-R011), `memory.spaces.unfreeze`,
 * `memory.grants.revoke`, and the five transfer ops. `op` is the exact op
 * string (§13, e.g. `"memory.exports.create"`).
 */
export function isFreezeAllowlisted(op: string): boolean {
  switch (op) {
    // Read-only ops (MEM-R011).
    case "memory.describe":
    case "memory.spaces.list":
    case "memory.subjects.get":
    case "memory.grants.list":
    case "memory.candidates.list":
    case "memory.candidates.get":
    case "memory.records.get":
    case "memory.records.list":
    case "memory.recall":
    case "memory.profile.get":
    case "memory.reflections.state":
    case "memory.reflections.list":
    case "memory.receipts.get":
    case "memory.exports.get":
    // §8.2 maintenance mutations.
    case "memory.spaces.unfreeze":
    case "memory.grants.revoke":
    case "memory.exports.create":
    case "memory.imports.stage":
    case "memory.imports.activate":
    case "memory.imports.discard":
      return true;
    default:
      return false;
  }
}

/** §7.2 `memory.spaces.list`: page limit vs the provider's `max_page_size`. */
export function validateMemorySpacesList(
  request: MemorySpacesListRequest,
  maxPageSize: number,
): SpokeResult<void> {
  return checkPageLimit(request.limit, maxPageSize);
}

/** §7.8 `memory.grants.list`: page limit vs the provider's `max_page_size`. */
export function validateMemoryGrantsList(
  request: MemoryGrantsListRequest,
  maxPageSize: number,
): SpokeResult<void> {
  return checkPageLimit(request.limit, maxPageSize);
}

/** §7.12 `memory.candidates.list`: page limit vs the provider's `max_page_size`. */
export function validateMemoryCandidatesList(
  request: MemoryCandidatesListRequest,
  maxPageSize: number,
): SpokeResult<void> {
  return checkPageLimit(request.limit, maxPageSize);
}

/** §7.7 `memory.subjects.fence`: `archive` on a context target is `INVALID_INPUT`. */
export function validateMemorySubjectsFence(
  request: MemorySubjectsFenceRequest,
): SpokeResult<void> {
  if (request.target.type === "context" && request.action === "archive") {
    return invalidInput("target", "archive is not valid on a context target");
  }
  return spokeOk();
}

/** §7.11 `memory.capture`: content byte limit vs the provider's `max_content_bytes`. */
export function validateMemoryCapture(
  request: MemoryCaptureRequest,
  maxContentBytes: number,
): SpokeResult<void> {
  return checkContentLimit(
    contentWireBytes(request.content.text, request.content.structured),
    maxContentBytes,
  );
}

/**
 * §7.9 `memory.grants.grant`: `capabilities` must be non-empty and unique,
 * and the restricted-shape `context_ids` must be non-empty. `shared_access`
 * without `context_ids` is type-enforced (only the restricted shape carries
 * the field).
 */
export function validateMemoryGrantsGrant(
  request: MemoryGrantsGrantRequest,
): SpokeResult<void> {
  const checkCapabilities = (capabilities: string[]): SpokeResult<void> => {
    if (capabilities.length === 0) {
      return invalidInput("capabilities", "capabilities must be non-empty");
    }
    if (new Set(capabilities).size !== capabilities.length) {
      return invalidInput("capabilities", "capabilities must be unique");
    }
    return spokeOk();
  };
  const restricted = request as { context_ids?: string[] };
  const capabilitiesCheck = checkCapabilities(request.capabilities);
  if (!capabilitiesCheck.ok) {
    return capabilitiesCheck;
  }
  if (restricted.context_ids !== undefined) {
    if (restricted.context_ids.length === 0) {
      return invalidInput("context_ids", "context_ids must be non-empty when present");
    }
  }
  return spokeOk();
}

/** §7.15 `memory.candidates.review`: a fragment decision must name at least one fragment. */
export function validateMemoryCandidatesReview(
  request: MemoryCandidatesReviewRequest,
): SpokeResult<void> {
  const decision = request.decision as { action: string; fragments?: unknown[] };
  if (decision.action === "fragment" && (decision.fragments ?? []).length === 0) {
    return invalidInput("decision.fragments", "fragment decision requires at least one fragment");
  }
  return spokeOk();
}

/** §7.16 `memory.records.create`: content byte limit. */
export function validateMemoryRecordsCreate(
  request: MemoryRecordsCreateRequest,
  maxContentBytes: number,
): SpokeResult<void> {
  return checkContentLimit(
    contentWireBytes(request.content.text, request.content.structured),
    maxContentBytes,
  );
}

/**
 * §7.18 `memory.records.update`: at least one of `content` / `kind` is
 * required — a combined update carrying both is valid (placement is the
 * only forbidden change). Runtime callers can bypass the generated oneOf,
 * so absence of both is rejected here; content is bounds-checked whenever
 * present.
 */
export function validateMemoryRecordsUpdate(
  request: MemoryRecordsUpdateRequest,
  maxContentBytes: number,
): SpokeResult<void> {
  const update = request as {
    kind?: string;
    content?: { text: string; structured?: unknown };
  };
  if (update.kind === undefined && update.content === undefined) {
    return invalidInput("content", "records.update requires content or kind");
  }
  if (update.content === undefined) {
    return spokeOk();
  }
  return checkContentLimit(
    contentWireBytes(update.content.text, update.content.structured),
    maxContentBytes,
  );
}

/** §7.20 `memory.records.list`: selection/filter invariants plus the page limit. */
export function validateMemoryRecordsList(
  request: MemoryRecordsListRequest,
  maxPageSize: number,
): SpokeResult<void> {
  const selectionCheck = checkSelection(
    request.selection as SelectionLike,
    request.selection !== undefined,
  );
  if (!selectionCheck.ok) {
    return selectionCheck;
  }
  const filterCheck = checkFilter(request.filter as FilterLike | undefined);
  if (!filterCheck.ok) {
    return filterCheck;
  }
  return checkPageLimit(request.limit, maxPageSize);
}

/** §7.22 `memory.recall`: selection/filter invariants and the `budget.max_items` hard cap (§9). */
export function validateMemoryRecall(
  request: MemoryRecallRequest,
  maxPageSize: number,
): SpokeResult<void> {
  const selectionCheck = checkSelection(
    request.selection as SelectionLike,
    request.selection !== undefined,
  );
  if (!selectionCheck.ok) {
    return selectionCheck;
  }
  const filterCheck = checkFilter(request.filter as FilterLike | undefined);
  if (!filterCheck.ok) {
    return filterCheck;
  }
  if (request.budget.max_items > maxPageSize) {
    return invalidLimit(
      "max_items",
      "max_page_size",
      `budget.max_items exceeds the provider max_page_size (${maxPageSize})`,
    );
  }
  return spokeOk();
}

/** §7.25 `memory.reflections.state`: selection/filter invariants. */
export function validateMemoryReflectionsState(
  request: MemoryReflectionsStateRequest,
): SpokeResult<void> {
  const selectionCheck = checkSelection(
    request.selection as SelectionLike,
    request.selection !== undefined,
  );
  if (!selectionCheck.ok) {
    return selectionCheck;
  }
  return checkFilter(request.filter as FilterLike | undefined);
}

/** §7.26 `memory.reflections.list`: optional selection/filter invariants plus the page limit. */
export function validateMemoryReflectionsList(
  request: MemoryReflectionsListRequest,
  maxPageSize: number,
): SpokeResult<void> {
  const selectionCheck = checkSelection(
    request.selection as SelectionLike,
    request.selection !== undefined,
  );
  if (!selectionCheck.ok) {
    return selectionCheck;
  }
  const filterCheck = checkFilter(request.filter as FilterLike | undefined);
  if (!filterCheck.ok) {
    return filterCheck;
  }
  return checkPageLimit(request.limit, maxPageSize);
}

/** §7.27 `memory.reflections.generate`: selection/filter invariants. */
export function validateMemoryReflectionsGenerate(
  request: MemoryReflectionsGenerateRequest,
): SpokeResult<void> {
  const selectionCheck = checkSelection(
    request.selection as SelectionLike,
    request.selection !== undefined,
  );
  if (!selectionCheck.ok) {
    return selectionCheck;
  }
  return checkFilter(request.filter as FilterLike | undefined);
}

/**
 * §7.31 `memory.imports.stage`: the header-chunk shape (carrying
 * `conflict_policy`) must carry `chunk_ordinal == 0`; tail ordinals ≥ 1 are
 * type-enforced by the shape split.
 */
export function validateMemoryImportsStage(
  request: MemoryImportsStageRequest,
): SpokeResult<void> {
  const stage = request as { conflict_policy?: unknown; chunk?: { chunk_ordinal?: unknown } };
  if (stage.conflict_policy !== undefined && stage.chunk?.chunk_ordinal !== 0) {
    return invalidInput("chunk.chunk_ordinal", "header chunk must carry chunk_ordinal 0");
  }
  return spokeOk();
}
