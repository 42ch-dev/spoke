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
import { isStrictTimestamp } from "./timestamp.js";

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

// note: Reserved verdict.detail direct member names in the fixed protocol
// probe order (contract §verdict core and detail) — the first present member
// decides details.field, independent of wire member order.
const DETAIL_RESERVED_MEMBERS = ["target", "evidence", "judgment_kind", "outcome"] as const;

// note: Shared ExtensionMap grammar — namespace keys (contract §Wire-wide rules).
const NAMESPACE_RE = /^[a-z][a-z0-9_-]*$/;

// note: Non-empty is raw JSON Schema minLength: 1 — whitespace-only strings are
// valid wire values for every open string in this contract.
function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// note: Interoperable integer domain [0, 2^53-1]; integral spellings (1.0)
// decode as the integer they spell.
function isPositiveInteger(value: unknown): value is number {
  return (
    typeof value === "number" && Number.isSafeInteger(value) && value >= 1
  );
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
// note: Wire members are own enumerable properties only (contract §verdict
// core and detail) — non-enumerable own names are representation detail, not
// wire members.
function ownEnumerable(value: object, key: string): boolean {
  return Object.prototype.propertyIsEnumerable.call(value, key);
}

// note: The producer-facing wire projection is the engine's own JSON wire
// emission: `JSON.parse(JSON.stringify(value))` reproduces the wire bytes a
// producer sends and a reader parses (contract §Wire member enumerability).
// Own enumerable members only, `toJSON` invoked with the member's key exactly
// as wire emission does, sparse array indices emitted as `null` holes (not
// compacted), `__proto__` re-parsed as an own data member, and no
// hand-rolled recursion — stringify's traversal is engine-implemented.
// Throws on values no wire exists for (cycles, bigints); both call sites
// translate a throw into an INVALID_INPUT refusal.
function wireForm(value: unknown): unknown {
  return JSON.parse(JSON.stringify(value));
}

// note: Lexicographic first-offender order is Unicode code-point order
// (contract §Wire-wide rules), matching Rust UTF-8 string order. ECMAScript's
// default `.sort()` compares UTF-16 code units, which misorders
// supplementary-plane keys (a leading surrogate sorts below every BMP code
// point >= U+E000).
function compareCodePoints(left: string, right: string): number {
  const leftPoints = Array.from(left, (unit) => unit.codePointAt(0)!);
  const rightPoints = Array.from(right, (unit) => unit.codePointAt(0)!);
  const shared = Math.min(leftPoints.length, rightPoints.length);
  for (let i = 0; i < shared; i += 1) {
    if (leftPoints[i] !== rightPoints[i]) {
      return leftPoints[i] - rightPoints[i];
    }
  }
  return leftPoints.length - rightPoints.length;
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
  if (!isNonEmptyString(ref.kind)) {
    return invalid(`${field}.kind`, `${field}.kind must be a non-empty string`);
  }
  if (!isNonEmptyString(ref.id)) {
    return invalid(`${field}.id`, `${field}.id must be a non-empty string`);
  }
  if (ref.namespace !== undefined && !isNonEmptyString(ref.namespace)) {
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
  // note: Validation reads wire members only — the producer object is first
  // re-emitted through the engine's JSON wire emission so off-wire members
  // and serializer hooks resolve to their wire form (or refuse outright).
  let wire: Judgment;
  try {
    wire = wireForm(judgment) as Judgment;
  } catch {
    return invalid(
      "judgment",
      "judgment is not representable on the JSON wire (cycle or unserializable member)",
    );
  }
  judgment = wire;
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
  if (!isNonEmptyString(judgment.judgment_id)) {
    return invalid("judgment_id", "judgment_id must be a non-empty string");
  }
  if (!isNonEmptyString(judgment.judgment_kind)) {
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
  if (!isNonEmptyString(verdict.outcome)) {
    return invalid("verdict.outcome", "verdict.outcome must be a non-empty string");
  }
  if (verdict.confidence !== undefined) {
    if (
      typeof verdict.confidence !== "number" ||
      !Number.isFinite(verdict.confidence) ||
      verdict.confidence < 0 ||
      verdict.confidence > 1
    ) {
      return invalid(
        "verdict.confidence",
        "verdict.confidence must be a finite number in [0, 1]",
      );
    }
  }
  if (verdict.detail !== undefined) {
    if (!isPlainObject(verdict.detail)) {
      return invalid("verdict.detail", "verdict.detail must be an object");
    }
    // Bounded probe over the four protocol constants in the fixed order,
    // over own enumerable members only — inherited and non-enumerable names
    // are not wire members.
    for (const member of DETAIL_RESERVED_MEMBERS) {
      if (ownEnumerable(verdict.detail, member)) {
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
      !isStrictTimestamp(judgment.created_at)
    ) {
      return invalid(
        "created_at",
        "created_at must be an RFC 3339 date-time with an explicit UTC offset when present",
      );
    }
  }
  if (!isPlainObject(judgment.extensions)) {
    return invalid("extensions", "extensions must be a present object");
  }
  // First-offender order is Unicode code-point order (contract §Wire-wide
  // rules), matching the Rust map's UTF-8 string order; ECMAScript default
  // sort disagrees on supplementary-plane keys. Own enumerable members only.
  for (const key of Object.keys(judgment.extensions)
    .filter((key) => ownEnumerable(judgment.extensions, key))
    .sort(compareCodePoints)) {
    const entry = judgment.extensions[key];
    if (!NAMESPACE_RE.test(key) || !isPlainObject(entry)) {
      return invalid(
        `extensions.${key}`,
        `extensions.${key} must carry a namespace-pattern key and an object value`,
      );
    }
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
    // note: the engine's JSON wire emission resolves `toJSON` hooks and
    // off-wire members to their wire form before canonicalization.
    return canonicalize(wireForm(value));
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
