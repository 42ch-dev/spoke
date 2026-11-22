/**
 * `spoke-judgment` envelope validation and canonical comparison (spoke-judgment
 * wire contract): `validateJudgment` checks the `Judgment` wire shape —
 * required fields, the closed `Judgment` / `JudgmentRef` / `verdict`
 * objects, positive-integer revisions, the `verdict` core, and reserved
 * `verdict.detail` member names. It is deterministic, performs no I/O, and
 * does NOT judge verdict truth, evidence existence or content correctness,
 * authorization, or product-specific `detail` shape.
 *
 * `judgmentsEqualCanonical` is the RFC 8785 comparison helper: two producers
 * of the same `Judgment` emit byte-identical canonical bytes, `verdict.detail`
 * and `extensions` included.
 *
 * Rust parity: `crates/spoke-operations/src/judgment.rs`.
 */

import canonicalize from "canonicalize";

import type { Judgment } from "@42ch/spoke-schemas";

import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
} from "./result.js";
import { isParseableDateTime } from "./timestamp.js";

// note: Closed-envelope property tables (judgment.schema.json,
// additionalProperties: false on Judgment, JudgmentRef and Verdict).
const JUDGMENT_KEYS: Record<string, true> = {
  schema_version: true,
  judgment_id: true,
  judgment_kind: true,
  target: true,
  evidence: true,
  verdict: true,
  basis: true,
  created_at: true,
  extensions: true,
};

const JUDGMENT_REF_KEYS: Record<string, true> = {
  kind: true,
  id: true,
  namespace: true,
  revision: true,
};

const VERDICT_KEYS: Record<string, true> = {
  outcome: true,
  confidence: true,
  detail: true,
};

// note: Reserved verdict.detail direct member names — they duplicate
// protocol-level envelope locations (contract §verdict core and detail).
const DETAIL_RESERVED_MEMBERS: Record<string, true> = {
  target: true,
  evidence: true,
  judgment_kind: true,
  outcome: true,
};

function isNonEmptyTrimmedString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1;
}

function invalid(field: string, message: string): SpokeResult<void> {
  return spokeReject(SpokeRejectCode.INVALID_INPUT, message, { field });
}

function hasOnlyKeys(
  value: Record<string, unknown>,
  allowed: Record<string, true>,
): boolean {
  return Object.keys(value).every((key) => allowed[key] === true);
}

function validateJudgmentRef(
  ref: unknown,
  field: string,
  requireRevision: boolean,
): SpokeResult<void> {
  if (!isPlainObject(ref)) {
    return invalid(field, `${field} must be a JudgmentRef object`);
  }
  if (!hasOnlyKeys(ref, JUDGMENT_REF_KEYS)) {
    return invalid(field, `${field} carries a member outside the JudgmentRef shape`);
  }
  if (!isNonEmptyTrimmedString(ref.kind)) {
    return invalid(`${field}.kind`, `${field}.kind must be a non-empty string`);
  }
  if (!isNonEmptyTrimmedString(ref.id)) {
    return invalid(`${field}.id`, `${field}.id must be a non-empty string`);
  }
  if (ref.namespace !== undefined && !isNonEmptyTrimmedString(ref.namespace)) {
    return invalid(
      `${field}.namespace`,
      `${field}.namespace must be a non-empty string when present`,
    );
  }
  if (requireRevision && ref.revision === undefined) {
    return invalid(
      `${field}.revision`,
      `${field}.revision is required on evidence references`,
    );
  }
  if (ref.revision !== undefined && !isPositiveInteger(ref.revision)) {
    return invalid(
      `${field}.revision`,
      `${field}.revision must be a positive integer`,
    );
  }
  return spokeOk();
}

/**
 * Wire-shape gate for `Judgment` (optional capability spoke-judgment).
 *
 * Every shape violation rejects `INVALID_INPUT` with `details.field` set to
 * the logical field path (dotted members, bracketed array indices, e.g.
 * `target.revision`, `evidence[0].revision`, `verdict.detail.target`). Pure
 * envelope validation only — verdict truth, evidence correctness and
 * product-specific `detail` shape stay product-owned.
 */
export function validateJudgment(judgment: Judgment): SpokeResult<void> {
  if (!isPlainObject(judgment)) {
    return invalid("judgment", "judgment must be an object");
  }
  if (!hasOnlyKeys(judgment, JUDGMENT_KEYS)) {
    return invalid(
      "judgment",
      "judgment carries a member outside the Judgment shape",
    );
  }
  if (!isPositiveInteger(judgment.schema_version)) {
    return invalid(
      "schema_version",
      "schema_version must be an integer >= 1",
    );
  }
  if (!isNonEmptyTrimmedString(judgment.judgment_id)) {
    return invalid("judgment_id", "judgment_id must be a non-empty string");
  }
  if (!isNonEmptyTrimmedString(judgment.judgment_kind)) {
    return invalid("judgment_kind", "judgment_kind must be a non-empty string");
  }

  const target = validateJudgmentRef(judgment.target, "target", false);
  if (!target.ok) {
    return target;
  }

  if (!Array.isArray(judgment.evidence)) {
    return invalid("evidence", "evidence must be an array (may be empty)");
  }
  for (let i = 0; i < judgment.evidence.length; i += 1) {
    const item = validateJudgmentRef(
      judgment.evidence[i],
      `evidence[${i}]`,
      true,
    );
    if (!item.ok) {
      return item;
    }
  }

  const verdict = judgment.verdict;
  if (!isPlainObject(verdict)) {
    return invalid("verdict", "verdict must be an object");
  }
  if (!hasOnlyKeys(verdict, VERDICT_KEYS)) {
    return invalid(
      "verdict",
      "verdict carries a member outside the Verdict shape",
    );
  }
  if (!isNonEmptyTrimmedString(verdict.outcome)) {
    return invalid("verdict.outcome", "verdict.outcome must be a non-empty string");
  }
  if (verdict.confidence !== undefined) {
    if (
      typeof verdict.confidence !== "number" ||
      verdict.confidence < 0 ||
      verdict.confidence > 1
    ) {
      return invalid(
        "verdict.confidence",
        "verdict.confidence must be a number in [0, 1]",
      );
    }
  }
  if (verdict.detail !== undefined) {
    if (!isPlainObject(verdict.detail)) {
      return invalid("verdict.detail", "verdict.detail must be an object");
    }
    for (const member of Object.keys(verdict.detail)) {
      if (DETAIL_RESERVED_MEMBERS[member] === true) {
        return invalid(
          `verdict.detail.${member}`,
          `verdict.detail.${member} duplicates a protocol envelope location`,
        );
      }
    }
  }

  if (judgment.basis !== undefined && typeof judgment.basis !== "string") {
    return invalid("basis", "basis must be a string when present");
  }
  if (judgment.created_at !== undefined) {
    if (
      typeof judgment.created_at !== "string" ||
      !isParseableDateTime(judgment.created_at)
    ) {
      return invalid(
        "created_at",
        "created_at must be an RFC 3339 timestamp when present",
      );
    }
  }
  if (!isPlainObject(judgment.extensions)) {
    return invalid("extensions", "extensions must be a present object");
  }

  return spokeOk();
}

/**
 * RFC 8785 canonical JSON bytes (as a string) of `value`, or `undefined`
 * when the value cannot be canonicalized (mirrors `serde_jcs::to_vec`
 * failure).
 */
function canonicalJson(value: unknown): string | undefined {
  try {
    return canonicalize(value);
  } catch {
    return undefined;
  }
}

/**
 * Whether two `Judgment`s are byte-identical under RFC 8785 canonical JSON —
 * the contract's cross-producer equality rule. `verdict.detail`,
 * `extensions`, and every present member (including present-empty arrays and
 * objects) are byte-significant.
 *
 * Rejects with `INVALID_INPUT` when either judgment cannot be serialized to
 * canonical bytes.
 */
export function judgmentsEqualCanonical(
  left: Judgment,
  right: Judgment,
): SpokeResult<boolean> {
  const leftBytes = canonicalJson(left);
  if (leftBytes === undefined) {
    return spokeReject(
      "INVALID_INPUT",
      "JCS canonicalization failed: value is not JSON-canonicalizable (RFC 8785)",
    );
  }
  const rightBytes = canonicalJson(right);
  if (rightBytes === undefined) {
    return spokeReject(
      "INVALID_INPUT",
      "JCS canonicalization failed: value is not JSON-canonicalizable (RFC 8785)",
    );
  }
  return spokeOk(leftBytes === rightBytes);
}
