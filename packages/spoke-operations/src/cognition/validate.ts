import type { CognitionRecord } from "@42ch/spoke-schemas";

import { isParseableDateTime } from "../mind-state/validate.js";
import {
  spokeOk,
  spokeReject,
  SpokeRejectCode,
  type SpokeReject,
  type SpokeResult,
} from "../result.js";

// Pure wire-shape gates for the frozen spoke-cognition family
// (schemas/data/cognition.schema.json). These guards check required/variant
// fields, closed/open keys, enums, reference/tuple shape, target-chain/order
// consistency, non-empty evidence/prior basis, category/assertion/metric
// compatibility, view placement, filter grammar and projection group/trace
// consistency only. They never resolve references, judge evidence
// sufficiency, compute probabilities or perform I/O.

/**
 * Placement of a standard record: holder-side current authority
 * (`view = "current"`) or recorded-at-time Memory content
 * (`view = "recorded_at"`). Absent placement means placement-agnostic
 * shape validation (both views are enum-valid).
 */
type CognitionPlacement = "authority" | "memory";

// Closed vocabularies (cognition.schema.json). Static membership tables are
// Records, matching the existing validator convention.
const REFERENCE_KINDS: Record<string, true> = {
  knowledge_entry: true,
  timeline_event: true,
  memory_record: true,
  memory_subject: true,
  principal: true,
  world: true,
  external: true,
};
const TARGET_KINDS: Record<string, true> = {
  proposition: true,
  question: true,
  perceived_content: true,
  episode: true,
};
const ASSERTIONS: Record<string, true> = {
  belief_attribution: true,
  actual_capture: true,
  knowledge_criterion: true,
  ignorance_criterion: true,
  assessor_indeterminacy: true,
};
const CALIBRATION_STATUSES: Record<string, true> = {
  calibrated: true,
  uncalibrated: true,
  unknown: true,
};
const ACQUISITION_PATHS: Record<string, true> = {
  Narration: true,
  Perception: true,
  Memory: true,
  Testimony: true,
  Inference: true,
  Imagination: true,
  Unknown: true,
};
const UNCERTAINTY_KINDS: Record<string, true> = {
  subjective_credible: true,
  statistical_confidence: true,
  other: true,
};
const ASSESSOR_ROLES: Record<string, true> = {
  holder_self_report: true,
  external_judgment: true,
  product_assessment: true,
};
const TRUTHS: Record<string, true> = { True: true, False: true, Unknown: true };
const ACCESSES: Record<string, true> = {
  Private: true,
  Shared: true,
  Public: true,
};
const REPRESENTATIONS: Record<string, true> = {
  Explicit: true,
  Implicit: true,
};
const CONTENT_TYPES: Record<string, true> = {
  Location: true,
  "Contents/Physical State": true,
  "Identity/Relation": true,
  Epistemic: true,
  "Desire/Intention": true,
  Emotion: true,
  "Trait/Value": true,
  "Action/Event": true,
};
const BELIEF_CONTEXTS: Record<string, true> = {
  Deceptive: true,
  Temporal: true,
  Counterfactual: true,
  Neutral: true,
};
const EPISTATES: Record<string, true> = {
  known: true,
  ignorant: true,
  undetermined: true,
};
const CATEGORIES: Record<string, true> = {
  epistemic: true,
  perception: true,
  belief: true,
};
const VIEWS: Record<string, true> = { current: true, recorded_at: true };
const PRESENCES: Record<string, true> = { assessed: true, no_record: true };
const EPISODE_KINDS: Record<string, true> = {
  timeline_event: true,
  memory_record: true,
  external: true,
};

// Fixed metric key → estimand mapping; the key is present only when its
// distinct estimate exists, and no cross-estimand arithmetic is inferred.
const KEY_ESTIMAND: Record<string, string> = {
  b: "holder_credence",
  p: "capture",
  f: "correspondence",
  k: "knowledge_criterion",
  r: "recording_fidelity",
};
// Category-compatible metric keys: Belief permits b/r; Perception permits
// p/f/r; epistemic permits k/r.
const CATEGORY_METRIC_KEYS: Record<string, string[]> = {
  belief: ["b", "r"],
  perception: ["p", "f", "r"],
  epistemic: ["k", "r"],
};
// q.assertion per epistemic state (no-record is a projection state, not an
// enum value).
const EPISTEMIC_ASSERTIONS: Record<string, string> = {
  known: "knowledge_criterion",
  ignorant: "ignorance_criterion",
  undetermined: "assessor_indeterminacy",
};

// Closed-envelope property tables (additionalProperties: false).
const REFERENCE_KEYS: Record<string, true> = {
  kind: true,
  id: true,
  namespace: true,
};
const TARGET_KEYS: Record<string, true> = {
  carrier: true,
  target_id: true,
  kind: true,
  text: true,
  about: true,
};
const CONTEXT_KEYS: Record<string, true> = { context_id: true, fork_id: true };
const CRITERION_KEYS: Record<string, true> = {
  criterion_id: true,
  expression: true,
};
const BASIS_KEYS: Record<string, true> = {
  kind: true,
  evidence: true,
  description: true,
  acquisition: true,
};
const UNCERTAINTY_KEYS: Record<string, true> = {
  kind: true,
  lower: true,
  upper: true,
  level: true,
  description: true,
};
const Q_KEYS: Record<string, true> = {
  value: true,
  assertion: true,
  method: true,
  calibration: true,
  uncertainty: true,
};
const METRIC_KEYS: Record<string, true> = {
  value: true,
  estimand: true,
  assessor: true,
  assessor_role: true,
  basis: true,
  context: true,
  effective_at: true,
  assessed_at: true,
  method: true,
  criterion: true,
  calibration: true,
  uncertainty: true,
};
const METRICS_KEYS: Record<string, true> = {
  b: true,
  p: true,
  f: true,
  k: true,
  r: true,
};
const BELIEF_KEYS: Record<string, true> = {
  kind: true,
  record_id: true,
  holder: true,
  target: true,
  assessor: true,
  assessor_role: true,
  basis: true,
  assessment_context: true,
  effective_at: true,
  assessed_at: true,
  view: true,
  criterion: true,
  q: true,
  metrics: true,
  extensions: true,
  order: true,
  truth: true,
  access: true,
  representation: true,
  content_type: true,
  source: true,
  context: true,
};
const EPISTEMIC_KEYS: Record<string, true> = {
  kind: true,
  record_id: true,
  holder: true,
  target: true,
  assessor: true,
  assessor_role: true,
  basis: true,
  assessment_context: true,
  effective_at: true,
  assessed_at: true,
  view: true,
  state: true,
  criterion: true,
  classification_policy: true,
  q: true,
  metrics: true,
  extensions: true,
};
const PERCEPTION_KEYS: Record<string, true> = {
  kind: true,
  record_id: true,
  holder: true,
  target: true,
  assessor: true,
  assessor_role: true,
  basis: true,
  assessment_context: true,
  effective_at: true,
  assessed_at: true,
  view: true,
  episode: true,
  content: true,
  access: true,
  criterion: true,
  q: true,
  metrics: true,
  extensions: true,
};
const WORLD_FACT_KEYS: Record<string, true> = {
  fact_id: true,
  holder: true,
  target: true,
  order: true,
  truth: true,
  access: true,
  representation: true,
  content_type: true,
  source: true,
  context: true,
  assessment_context: true,
  effective_at: true,
  recorded_at: true,
  extensions: true,
};
const MODULE_KEYS: Record<string, true> = {
  records: true,
  world_facts: true,
  extensions: true,
};
const OBSERVATION_MODULE_KEYS: Record<string, true> = {
  observers: true,
  access: true,
};
const FILTER_KEYS: Record<string, true> = {
  holder: true,
  target: true,
  assessment_context: true,
  category: true,
  state: true,
  q: true,
  order: true,
  truth: true,
  access: true,
  representation: true,
  content_type: true,
  source: true,
  belief_context: true,
};
const TARGET_PREDICATE_KEYS: Record<string, true> = {
  carrier: true,
  target_id: true,
  kind: true,
};
const CONTEXT_PREDICATE_KEYS: Record<string, true> = {
  context_id: true,
  fork_id: true,
};
const QRANGE_KEYS: Record<string, true> = { min: true, max: true };
const POLICY_KEYS: Record<string, true> = {
  policy_id: true,
  policy_version: true,
};
const SELECTION_TRACE_KEYS: Record<string, true> = {
  policy: true,
  input_record_ids: true,
  selected_record_id: true,
};
const PROJECTION_KEYS: Record<string, true> = {
  view: true,
  items: true,
  extensions: true,
};
const PROJECTION_ITEM_KEYS: Record<string, true> = {
  holder: true,
  target: true,
  assessment_context: true,
  presence: true,
  assessments: true,
  selection: true,
};

// Required identity realm for every reference kind.
const NAMESPACE_RE = /^[a-z][a-z0-9_-]*$/;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyTrimmedString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function hasOnlyKeys(
  value: Record<string, unknown>,
  keys: Record<string, true>,
): boolean {
  for (const key of Object.keys(value)) {
    if (keys[key] !== true) {
      return false;
    }
  }
  return true;
}

function isExtensionMap(value: unknown): boolean {
  return isPlainObject(value) &&
    Object.entries(value).every(([key, entry]) => NAMESPACE_RE.test(key) && isPlainObject(entry));
}

// Finite declared probability in the closed 0..1 range; 0 and 1 are admitted
// only as deliberately assigned values, never label defaults.
function isProbability01(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    value >= 0 &&
    value <= 1
  );
}

// --- C5 diagnostic grammar -------------------------------------------------
//
// Every `check*` below returns the relative path of the first member that
// fails that grammar, or `undefined` when the value passes. A failure with no
// member path (`null`) names the checked value itself, so a cross-field
// failure (reversed `q` bounds, a predicate with two arms) or a non-object
// keeps its enclosing member's path. Member names are rendered where they are
// discovered: an ordinary identifier stays dotted (`holder.id`), any other
// name keeps its JSON-quoted bracket form (`q[""]`, `q["a.b"]`), so an empty
// member never collides with the enclosing object's own failure. Paths are
// built only on failure, and one traversal walks the original value: no probe
// copies and no second schema map.

/**
 * `undefined` passes; `null` is the checked value itself; a string is the
 * rendered relative path of the failing member.
 */
type Problem = string | null;

const MEMBER_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

function renderMember(name: string): string {
  return MEMBER_NAME_RE.test(name) ? name : `[${JSON.stringify(name)}]`;
}

function childPath(name: string, problem: Problem): string {
  if (problem === null) {
    return name;
  }
  // A rendered non-identifier member already carries its brackets.
  return problem.startsWith("[")
    ? `${name}${problem}`
    : `${name}.${problem}`;
}

function firstUnknownKey(
  value: Record<string, unknown>,
  keys: Record<string, true>,
): string | undefined {
  return Object.keys(value).find((key) => keys[key] !== true);
}

function checkOptional(
  value: Record<string, unknown>,
  key: string,
  check: (child: unknown) => Problem | undefined,
): Problem | undefined {
  if (value[key] === undefined) {
    return undefined;
  }
  const problem = check(value[key]);
  return problem === undefined ? undefined : childPath(key, problem);
}

// Complete (kind, namespace, id) tuple identity; shape only — no resolution.
function checkCognitionReference(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, REFERENCE_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  if (typeof value.kind !== "string" || REFERENCE_KINDS[value.kind] !== true) {
    return "kind";
  }
  if (!isNonEmptyString(value.id)) {
    return "id";
  }
  if (typeof value.namespace !== "string" || !NAMESPACE_RE.test(value.namespace)) {
    return "namespace";
  }
  return undefined;
}

function isCognitionReference(value: unknown): value is Record<string, unknown> {
  return checkCognitionReference(value) === undefined;
}

// Target chain never exceeds 3; `depth` counts nested `about` links.
function isCognitionTarget(value: unknown, depth: number): boolean {
  if (depth > 2 || !isPlainObject(value) || !hasOnlyKeys(value, TARGET_KEYS)) {
    return false;
  }
  if (
    !isCognitionReference(value.carrier) ||
    !isNonEmptyString(value.target_id) ||
    typeof value.kind !== "string" ||
    TARGET_KINDS[value.kind] !== true
  ) {
    return false;
  }
  if (value.text !== undefined && typeof value.text !== "string") {
    return false;
  }
  if (value.about !== undefined && !isCognitionTarget(value.about, depth + 1)) {
    return false;
  }
  return true;
}

function targetChainLength(value: Record<string, unknown>): number {
  const about = value.about;
  if (isPlainObject(about)) {
    return 1 + targetChainLength(about);
  }
  return 1;
}

function isCognitionContext(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, CONTEXT_KEYS)) {
    return false;
  }
  if (!isNonEmptyString(value.context_id)) {
    return false;
  }
  return value.fork_id === undefined || isNonEmptyString(value.fork_id);
}

function isCognitionCriterion(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, CRITERION_KEYS)) {
    return false;
  }
  return (
    isNonEmptyString(value.criterion_id) &&
    (value.expression === undefined || typeof value.expression === "string")
  );
}

// Every completed standard assessment/metric has a non-empty evidence basis
// or an explicit declared prior basis; a declared basis may remain
// uncalibrated and supplies no default probability, interval or calibration.
function isAssessmentBasis(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, BASIS_KEYS)) {
    return false;
  }
  if (
    value.acquisition !== undefined &&
    (typeof value.acquisition !== "string" ||
      ACQUISITION_PATHS[value.acquisition] !== true)
  ) {
    return false;
  }
  if (value.kind === "evidence") {
    return (
      value.description === undefined &&
      Array.isArray(value.evidence) &&
      value.evidence.length > 0 &&
      value.evidence.every(isCognitionReference)
    );
  }
  if (value.kind === "prior") {
    return value.evidence === undefined && isNonEmptyTrimmedString(value.description);
  }
  return false;
}

// 0 <= lower <= upper <= 1; level strictly inside (0, 1); kind "other"
// requires an explicit description. Unquantified uncertainty is an absent
// `uncertainty`, never a fabricated zero-width interval.
function isProbabilityUncertainty(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, UNCERTAINTY_KEYS)) {
    return false;
  }
  if (
    typeof value.kind !== "string" ||
    UNCERTAINTY_KINDS[value.kind] !== true
  ) {
    return false;
  }
  if (
    !isProbability01(value.lower) ||
    !isProbability01(value.upper) ||
    value.lower > value.upper
  ) {
    return false;
  }
  if (
    value.level !== undefined &&
    (typeof value.level !== "number" ||
      !Number.isFinite(value.level) ||
      value.level <= 0 ||
      value.level >= 1)
  ) {
    return false;
  }
  if (value.kind === "other") {
    return isNonEmptyTrimmedString(value.description);
  }
  return value.description === undefined || typeof value.description === "string";
}

function isDeclaredProbability(
  value: unknown,
  assertion: keyof typeof ASSERTIONS,
): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, Q_KEYS)) {
    return false;
  }
  return (
    isProbability01(value.value) &&
    value.assertion === assertion &&
    isNonEmptyString(value.method) &&
    typeof value.calibration === "string" &&
    CALIBRATION_STATUSES[value.calibration] === true &&
    (value.uncertainty === undefined || isProbabilityUncertainty(value.uncertainty))
  );
}

function isTimestamp(value: unknown): boolean {
  if (
    typeof value !== "string" ||
    value !== value.trim() ||
    !/(?:[Zz]|[+-]\d{2}:\d{2})$/.test(value) ||
    !isParseableDateTime(value)
  ) {
    return false;
  }

  if (value.slice(17, 19) !== "60") {
    return true;
  }
  // Fictional leap seconds need a UTC month boundary, not a historical lookup.
  const precedingSecond = Date.parse(`${value.slice(0, 17)}59${value.slice(19)}`);
  const utc = new Date(precedingSecond);
  return utc.getUTCHours() === 23 && utc.getUTCMinutes() === 59 &&
    new Date(precedingSecond + 1000).getUTCDate() === 1;
}

// A single metric whose estimand is fixed by its enclosing metrics key.
function isCognitionMetric(value: unknown, estimand: string): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, METRIC_KEYS)) {
    return false;
  }
  if (
    !isProbability01(value.value) ||
    value.estimand !== estimand ||
    !isCognitionReference(value.assessor) ||
    typeof value.assessor_role !== "string" ||
    ASSESSOR_ROLES[value.assessor_role] !== true ||
    !isAssessmentBasis(value.basis) ||
    !isCognitionContext(value.context) ||
    !isTimestamp(value.effective_at) ||
    !isTimestamp(value.assessed_at) ||
    !isNonEmptyString(value.method) ||
    typeof value.calibration !== "string" ||
    CALIBRATION_STATUSES[value.calibration] !== true ||
    (value.uncertainty !== undefined &&
      !isProbabilityUncertainty(value.uncertainty))
  ) {
    return false;
  }
  // criterion is required for knowledge_criterion/correspondence estimands.
  if (estimand === "knowledge_criterion" || estimand === "correspondence") {
    return isCognitionCriterion(value.criterion);
  }
  return value.criterion === undefined || isCognitionCriterion(value.criterion);
}

// Category-compatible metric keys only, each with its fixed estimand; absent
// keys are ordinary missing data, never zero or unsupported features.
function isCognitionMetrics(value: unknown, category: string): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, METRICS_KEYS)) {
    return false;
  }
  const allowed = CATEGORY_METRIC_KEYS[category];
  for (const key of Object.keys(KEY_ESTIMAND)) {
    const metric = value[key];
    if (metric === undefined) {
      continue;
    }
    if (!allowed.includes(key)) {
      return false;
    }
    if (!isCognitionMetric(metric, KEY_ESTIMAND[key])) {
      return false;
    }
  }
  return true;
}

function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((item, i) => sameJson(item, b[i]));
  }
  if (!isPlainObject(a) || !isPlainObject(b)) return false;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length &&
    keys.every((key) => Object.hasOwn(b, key) && sameJson(a[key], b[key]));
}

function duplicateEstimate(record: Record<string, unknown>, category: string): boolean {
  if (!isPlainObject(record.metrics) || !isPlainObject(record.q)) return false;
  const key = category === "perception" ? "p" : category === "epistemic" &&
    (record.state === "known" || record.state === "ignorant") ? "k" : undefined;
  if (key === undefined || !isPlainObject(record.metrics[key])) return false;
  const metric = record.metrics[key];
  const q = record.q;
  return ["value", "method", "calibration", "uncertainty"].every((field) =>
    sameJson(q[field], metric[field])) &&
    ["assessor", "assessor_role", "basis", "effective_at", "assessed_at"].every((field) =>
      sameJson(record[field], metric[field])) &&
    sameJson(record.assessment_context, metric.context) &&
    (key !== "k" || sameJson(record.criterion, metric.criterion));
}

// Shared required members of the three completed standard record variants.
// The holder is an actor/group and never kind = world.
function isAssessmentMembers(
  value: Record<string, unknown>,
  category: string,
  assertion: keyof typeof ASSERTIONS,
  placement: CognitionPlacement | undefined,
): boolean {
  if (!isNonEmptyString(value.record_id)) {
    return false;
  }
  if (
    !isCognitionReference(value.holder) ||
    value.holder.kind === "world" ||
    !isCognitionTarget(value.target, 0) ||
    !isCognitionReference(value.assessor) ||
    typeof value.assessor_role !== "string" ||
    ASSESSOR_ROLES[value.assessor_role] !== true ||
    !isAssessmentBasis(value.basis) ||
    !isCognitionContext(value.assessment_context) ||
    !isTimestamp(value.effective_at) ||
    !isTimestamp(value.assessed_at)
  ) {
    return false;
  }
  if (
    typeof value.view !== "string" ||
    VIEWS[value.view] !== true ||
    (placement === "authority" && value.view !== "current") ||
    (placement === "memory" && value.view !== "recorded_at")
  ) {
    return false;
  }
  if (value.criterion !== undefined && !isCognitionCriterion(value.criterion)) {
    return false;
  }
  if (!isDeclaredProbability(value.q, assertion)) {
    return false;
  }
  if (
    value.metrics !== undefined &&
    !isCognitionMetrics(value.metrics, category)
  ) {
    return false;
  }
  return !duplicateEstimate(value, category) && isExtensionMap(value.extensions);
}

function isBeliefRecord(
  value: Record<string, unknown>,
  placement: CognitionPlacement | undefined,
): boolean {
  if (!hasOnlyKeys(value, BELIEF_KEYS)) {
    return false;
  }
  if (
    value.kind !== "belief" ||
    typeof value.order !== "number" ||
    !Number.isInteger(value.order) ||
    value.order < 0 ||
    value.order > 3
  ) {
    return false;
  }
  // Retained Belief dimensions; every optional dimension is a closed enum.
  if (
    (value.truth !== undefined &&
      (typeof value.truth !== "string" || TRUTHS[value.truth] !== true)) ||
    (value.access !== undefined &&
      (typeof value.access !== "string" || ACCESSES[value.access] !== true)) ||
    (value.representation !== undefined &&
      (typeof value.representation !== "string" ||
        REPRESENTATIONS[value.representation] !== true)) ||
    (value.content_type !== undefined &&
      (typeof value.content_type !== "string" ||
        CONTENT_TYPES[value.content_type] !== true)) ||
    (value.source !== undefined &&
      (typeof value.source !== "string" ||
        ACQUISITION_PATHS[value.source] !== true)) ||
    (value.context !== undefined &&
      (typeof value.context !== "string" ||
        BELIEF_CONTEXTS[value.context] !== true))
  ) {
    return false;
  }
  if (!isCognitionTarget(value.target, 0)) {
    return false;
  }
  const order = value.order as number;
  const target = value.target as Record<string, unknown>;
  if (target.kind !== "proposition") return false;
  const chain = targetChainLength(target);
  if (chain > 3) {
    return false;
  }
  // order 1: about absent; order 2/3: about present and chain depth equals
  // the order. order 0 keeps the shared vocabulary without a chain rule.
  if (order === 1 && target.about !== undefined) {
    return false;
  }
  if (
    (order === 2 || order === 3) &&
    (target.about === undefined || chain !== order)
  ) {
    return false;
  }
  return isAssessmentMembers(value, "belief", "belief_attribution", placement);
}

function isEpistemicAssessment(
  value: Record<string, unknown>,
  placement: CognitionPlacement | undefined,
): boolean {
  if (!hasOnlyKeys(value, EPISTEMIC_KEYS)) {
    return false;
  }
  if (
    value.kind !== "epistemic" ||
    typeof value.state !== "string" ||
    EPISTATES[value.state] !== true ||
    !isCognitionCriterion(value.criterion)
  ) {
    return false;
  }
  if (!isPlainObject(value.target) ||
      (value.target.kind !== "proposition" && value.target.kind !== "question")) return false;
  const policy = value.classification_policy;
  if (
    !isPlainObject(policy) ||
    !hasOnlyKeys(policy, POLICY_KEYS) ||
    !isNonEmptyString(policy.policy_id) ||
    !isNonEmptyString(policy.policy_version)
  ) {
    return false;
  }
  return isAssessmentMembers(
    value,
    "epistemic",
    EPISTEMIC_ASSERTIONS[value.state] as keyof typeof ASSERTIONS,
    placement,
  );
}

// Actual capture constraints if known; open — unknown keys round-trip verbatim.
function isObservationAccess(value: unknown): boolean {
  if (!isPlainObject(value)) {
    return false;
  }
  if (
    value.position !== undefined &&
    typeof value.position !== "string" &&
    !isPlainObject(value.position)
  ) {
    return false;
  }
  for (const field of ["line_of_sight", "hearing_range"] as const) {
    const fieldValue = value[field];
    if (
      fieldValue !== undefined &&
      typeof fieldValue !== "boolean" &&
      !isPlainObject(fieldValue)
    ) {
      return false;
    }
  }
  return (
    value.modality === undefined ||
    (Array.isArray(value.modality) &&
      value.modality.every((m) => typeof m === "string"))
  );
}

function isPerceptionRecord(
  value: Record<string, unknown>,
  placement: CognitionPlacement | undefined,
): boolean {
  if (!hasOnlyKeys(value, PERCEPTION_KEYS)) {
    return false;
  }
  if (
    value.kind !== "perception" ||
    !isCognitionReference(value.episode) ||
    typeof value.episode.kind !== "string" ||
    EPISODE_KINDS[value.episode.kind] !== true ||
    value.content === undefined
  ) {
    return false;
  }
  const target = value.target;
  if (!isCognitionTarget(target, 0)) {
    return false;
  }
  // Perception target and episode share the complete carrier reference.
  const targetObject = target as Record<string, unknown>;
  if (targetObject.kind !== "perceived_content" && targetObject.kind !== "episode") return false;
  const episode = value.episode as Record<string, unknown>;
  const carrier = targetObject.carrier as Record<string, unknown>;
  if (
    carrier.kind !== episode.kind ||
    carrier.namespace !== episode.namespace ||
    carrier.id !== episode.id
  ) {
    return false;
  }
  if (value.access !== undefined && !isObservationAccess(value.access)) {
    return false;
  }
  return isAssessmentMembers(value, "perception", "actual_capture", placement);
}

/**
 * Wire-shape gate for a closed standard cognition record (Belief, epistemic
 * or Perception variant).
 *
 * When `placement` is supplied, the record `view` must match the placement:
 * `"authority"` requires `view = "current"` and `"memory"` requires
 * `view = "recorded_at"` (Memory recorded-at-time shape). Without a
 * placement, both views are shape-valid. Pure validation only — no reference
 * resolution, evidence-sufficiency judgement or probability calculation.
 */
function validatePlacedRecord(
  value: unknown,
  placement?: CognitionPlacement,
): value is CognitionRecord {
  if (!isPlainObject(value)) {
    return false;
  }
  if (value.kind === "belief") {
    return isBeliefRecord(value, placement);
  }
  if (value.kind === "epistemic") {
    return isEpistemicAssessment(value, placement);
  }
  if (value.kind === "perception") {
    return isPerceptionRecord(value, placement);
  }
  return false;
}

export function validateCognitionRecord(value: unknown): value is CognitionRecord {
  return validatePlacedRecord(value);
}

// `record_id` is unique within one carrier (§3) and nowhere else, so the same
// id may still identify different records in another carrier.
function hasDistinctRecordIds(records: readonly CognitionRecord[]): boolean {
  return new Set(records.map((record) => record.record_id)).size === records.length;
}

// One carrier's record list: every entry is a placed record of that carrier's
// view and the ids are distinct, so the trace can name one record.
function validateCarrierRecords(
  value: unknown,
  view: "current" | "recorded_at",
): boolean {
  return (
    Array.isArray(value) &&
    value.every(
      (record: unknown) => validateCognitionRecord(record) && record.view === view,
    ) &&
    hasDistinctRecordIds(value as CognitionRecord[])
  );
}

export function validateCognitionContent(value: unknown): boolean {
  return isPlainObject(value) &&
    (!("cognition" in value) ||
      validateCarrierRecords(value.cognition, "recorded_at"));
}

function isWorldFactRecord(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, WORLD_FACT_KEYS)) {
    return false;
  }
  // Narrated order-0 facts: the holder is the designated world authority.
  if (
    !isNonEmptyString(value.fact_id) ||
    !isCognitionReference(value.holder) ||
    value.holder.kind !== "world" ||
    !isCognitionTarget(value.target, 0) ||
    !isPlainObject(value.target) ||
    value.target.kind !== "proposition" ||
    value.order !== 0 ||
    (value.truth !== undefined &&
      (typeof value.truth !== "string" || TRUTHS[value.truth] !== true)) ||
    (value.access !== undefined &&
      (typeof value.access !== "string" || ACCESSES[value.access] !== true)) ||
    (value.representation !== undefined &&
      (typeof value.representation !== "string" ||
        REPRESENTATIONS[value.representation] !== true)) ||
    (value.content_type !== undefined &&
      (typeof value.content_type !== "string" ||
        CONTENT_TYPES[value.content_type] !== true)) ||
    (value.source !== undefined &&
      (typeof value.source !== "string" ||
        ACQUISITION_PATHS[value.source] !== true)) ||
    (value.context !== undefined &&
      (typeof value.context !== "string" ||
        BELIEF_CONTEXTS[value.context] !== true)) ||
    !isCognitionContext(value.assessment_context) ||
    !isTimestamp(value.effective_at) ||
    !isTimestamp(value.recorded_at) ||
    !isExtensionMap(value.extensions)
  ) {
    return false;
  }
  return targetChainLength(value.target as Record<string, unknown>) <= 3;
}

/**
 * Wire-shape gate for the KnowledgeEntry `modules.cognition` carrier: closed
 * `{records, world_facts, extensions}` with current-view actor records and
 * order-0 world facts whose holder is kind = "world".
 */
export function validateCognitionModule(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, MODULE_KEYS)) {
    return false;
  }
  return (
    validateCarrierRecords(value.records, "current") &&
    Array.isArray(value.world_facts) &&
    value.world_facts.every(isWorldFactRecord) &&
    isExtensionMap(value.extensions)
  );
}

/**
 * Wire-shape gate for the TimelineEvent `modules.observation` carrier.
 * `observers` is required and `[]` is the explicit empty could-perceive set
 * (distinct from a missing module and from an actual Perception record);
 * `access` is open and unknown keys round-trip verbatim.
 */
export function validateObservationModule(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, OBSERVATION_MODULE_KEYS)) {
    return false;
  }
  return (
    Array.isArray(value.observers) &&
    value.observers.every(isCognitionReference) &&
    (value.access === undefined || isObservationAccess(value.access))
  );
}

// Categorical predicate: exactly one of {eq: <enum member>} or {absent: true}.
function checkCategoricalPredicate(
  value: unknown,
  members: Record<string, true>,
): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const keys = Object.keys(value);
  // A genuine unknown member outranks the arm count: name it where it is.
  const unknown = keys.find((key) => key !== "eq" && key !== "absent");
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  if (keys.length !== 1) {
    return null;
  }
  const key = keys[0]!;
  if (key === "absent") {
    return value.absent === true ? undefined : "absent";
  }
  if (key !== "eq") {
    return renderMember(key);
  }
  return typeof value.eq === "string" && members[value.eq] === true
    ? undefined
    : "eq";
}

// Closed fork predicate: exactly one of {eq: <non-empty fork id>} or
// {absent: true}. An omitted predicate is a wildcard and valid.
function checkForkIdPredicate(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const keys = Object.keys(value);
  // A genuine unknown member outranks the arm count: name it where it is.
  const unknown = keys.find((key) => key !== "eq" && key !== "absent");
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  if (keys.length !== 1) {
    return null;
  }
  const key = keys[0]!;
  if (key === "absent") {
    return value.absent === true ? undefined : "absent";
  }
  if (key !== "eq") {
    return renderMember(key);
  }
  return isNonEmptyString(value.eq) ? undefined : "eq";
}

// Inclusive range over the declared foundational q.value; at least one bound
// is required and min <= max. Uncalibrated but declared values participate
// identically — the validator checks the grammar, not the semantics.
function checkQRange(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, QRANGE_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  const { min, max } = value;
  if (min === undefined && max === undefined) {
    return null;
  }
  if (min !== undefined && !isProbability01(min)) {
    return "min";
  }
  if (max !== undefined && !isProbability01(max)) {
    return "max";
  }
  return min === undefined || max === undefined || min <= max
    ? undefined
    : null;
}

// Filter target predicate: closed carrier/target_id/kind envelope.
function checkTargetPredicate(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, TARGET_PREDICATE_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  const carrier = checkOptional(value, "carrier", checkCognitionReference);
  if (carrier !== undefined) {
    return carrier;
  }
  if (value.target_id !== undefined && !isNonEmptyString(value.target_id)) {
    return "target_id";
  }
  if (
    value.kind !== undefined &&
    (typeof value.kind !== "string" || TARGET_KINDS[value.kind] !== true)
  ) {
    return "kind";
  }
  return undefined;
}

// Filter assessment-context predicate: closed context_id/fork_id envelope.
function checkContextPredicate(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, CONTEXT_PREDICATE_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  if (value.context_id !== undefined && !isNonEmptyString(value.context_id)) {
    return "context_id";
  }
  return checkOptional(value, "fork_id", checkForkIdPredicate);
}

function checkCategoricalMember(
  holder: Record<string, unknown>,
  key: string,
  members: Record<string, true>,
): Problem | undefined {
  const predicate = holder[key];
  if (predicate === undefined) {
    return undefined;
  }
  const problem = checkCategoricalPredicate(predicate, members);
  return problem === undefined ? undefined : childPath(key, problem);
}

function checkOrder(value: unknown): Problem | undefined {
  return typeof value === "number" &&
    Number.isInteger(value) &&
    value >= 0 &&
    value <= 3
    ? undefined
    : null;
}

function cognitionReject(problem: Problem, message: string): SpokeReject {
  return spokeReject(
    SpokeRejectCode.INVALID_INPUT,
    message,
    problem === null ? undefined : { field: problem },
  );
}

/**
 * Relative path of the first member that fails the closed CognitionFilter
 * grammar, or `undefined` when the filter passes. Bare numeric keys and
 * schema-unknown predicates are not standard probability filters and fail
 * the closed envelope. Pure grammar validation only — predicates are not
 * evaluated against any record set here.
 */
function checkCognitionFilter(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, FILTER_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  let problem = checkOptional(value, "holder", checkCognitionReference);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkOptional(value, "target", checkTargetPredicate);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkOptional(value, "assessment_context", checkContextPredicate);
  if (problem !== undefined) {
    return problem;
  }
  if (
    value.category !== undefined &&
    (typeof value.category !== "string" || CATEGORIES[value.category] !== true)
  ) {
    return "category";
  }
  if (
    value.state !== undefined &&
    (typeof value.state !== "string" || EPISTATES[value.state] !== true)
  ) {
    return "state";
  }
  problem = checkOptional(value, "q", checkQRange);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkOptional(value, "order", checkOrder);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkCategoricalMember(value, "truth", TRUTHS);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkCategoricalMember(value, "access", ACCESSES);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkCategoricalMember(value, "representation", REPRESENTATIONS);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkCategoricalMember(value, "content_type", CONTENT_TYPES);
  if (problem !== undefined) {
    return problem;
  }
  problem = checkCategoricalMember(value, "source", ACQUISITION_PATHS);
  if (problem !== undefined) {
    return problem;
  }
  return checkCategoricalMember(value, "belief_context", BELIEF_CONTEXTS);
}

/**
 * Wire-shape gate for the closed CognitionFilter grammar (see
 * {@link checkCognitionFilter}). A rejection carries `details.field` with the
 * failing member's path relative to the filter; a root failure omits it.
 */
export function validateCognitionFilter(value: unknown): SpokeResult<void> {
  const problem = checkCognitionFilter(value);
  return problem === undefined
    ? spokeOk()
    : cognitionReject(problem, "cognition filter failed wire-shape validation");
}

function checkSelectionPolicyIdentity(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, POLICY_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  if (!isNonEmptyString(value.policy_id)) {
    return "policy_id";
  }
  return isNonEmptyString(value.policy_version)
    ? undefined
    : "policy_version";
}

function isSelectionPolicyIdentity(value: unknown): boolean {
  return checkSelectionPolicyIdentity(value) === undefined;
}

// Union of the CognitionSelection envelope members; the view decides which
// subset is admitted.
const SELECTION_KEYS: Record<string, true> = {
  view: true,
  filter: true,
  selection_policy: true,
};

/**
 * Relative path of the first member that fails the CognitionSelection union:
 * `view = "current"` requires an explicitly identified `selection_policy`;
 * `view = "recorded_at"` carries a filter only. The library verifies the
 * policy identity is present — it supplies no policy algorithm.
 */
function checkCognitionSelection(value: unknown): Problem | undefined {
  if (!isPlainObject(value)) {
    return null;
  }
  const unknown = firstUnknownKey(value, SELECTION_KEYS);
  if (unknown !== undefined) {
    return renderMember(unknown);
  }
  const filter = checkCognitionFilter(value.filter);
  if (filter !== undefined) {
    return childPath("filter", filter);
  }
  if (value.view === "current") {
    if (value.selection_policy === undefined) {
      return "selection_policy";
    }
    const policy = checkSelectionPolicyIdentity(value.selection_policy);
    return policy === undefined
      ? undefined
      : childPath("selection_policy", policy);
  }
  if (value.view === "recorded_at") {
    return Object.hasOwn(value, "selection_policy")
      ? "selection_policy"
      : undefined;
  }
  return "view";
}

/**
 * Wire-shape gate for the CognitionSelection union (see
 * {@link checkCognitionSelection}). A rejection carries `details.field` with
 * the failing member's path relative to the selection; a root failure omits
 * it.
 */
export function validateCognitionSelection(value: unknown): SpokeResult<void> {
  const problem = checkCognitionSelection(value);
  return problem === undefined
    ? spokeOk()
    : cognitionReject(
        problem,
        "cognition selection failed wire-shape validation",
      );
}

function isSelectionTrace(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, SELECTION_TRACE_KEYS)) {
    return false;
  }
  return (
    isSelectionPolicyIdentity(value.policy) &&
    Array.isArray(value.input_record_ids) &&
    value.input_record_ids.every((id) => typeof id === "string") &&
    isNonEmptyString(value.selected_record_id) &&
    value.input_record_ids.includes(value.selected_record_id)
  );
}

// One item and its assessments describe one source group (§5.3): the same
// complete holder reference, target identity (carrier tuple, kind, target_id
// — never text/about) and assessment context. Ids are compared per item only,
// because the same `record_id` may legitimately live in another carrier.
function isSameGroupMember(
  item: Record<string, unknown>,
  record: CognitionRecord,
): boolean {
  const holder = item.holder as Record<string, unknown>;
  const target = item.target as Record<string, unknown>;
  const carrier = target.carrier as Record<string, unknown>;
  const context = item.assessment_context as Record<string, unknown>;
  return (
    holder.kind === record.holder.kind &&
    holder.namespace === record.holder.namespace &&
    holder.id === record.holder.id &&
    carrier.kind === record.target.carrier.kind &&
    carrier.namespace === record.target.carrier.namespace &&
    carrier.id === record.target.carrier.id &&
    target.kind === record.target.kind &&
    target.target_id === record.target.target_id &&
    context.context_id === record.assessment_context.context_id &&
    context.fork_id === record.assessment_context.fork_id
  );
}

function isCognitionProjectionItem(
  value: unknown,
  placement: CognitionPlacement,
  withSelection: boolean,
): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, PROJECTION_ITEM_KEYS)) {
    return false;
  }
  if (
    !isCognitionReference(value.holder) ||
    !isCognitionTarget(value.target, 0) ||
    !isCognitionContext(value.assessment_context) ||
    typeof value.presence !== "string" ||
    PRESENCES[value.presence] !== true ||
    !Array.isArray(value.assessments) ||
    !value.assessments.every((record) =>
      validatePlacedRecord(record, placement),
    )
  ) {
    return false;
  }
  if ((value.presence === "no_record") !== (value.assessments.length === 0)) {
    return false;
  }
  const assessments = value.assessments as CognitionRecord[];
  const recordIds = assessments.map((record) => record.record_id);
  if (
    !hasDistinctRecordIds(assessments) ||
    !assessments.every((record) => isSameGroupMember(value, record))
  ) {
    return false;
  }
  // The current selection trace is required iff view = current and presence
  // = assessed; recorded-at-time projection has no current selection. Its
  // candidates are exactly this item's assessments, so the selected id always
  // names a record the item carries.
  if (withSelection) {
    if (value.selection === undefined || !isSelectionTrace(value.selection)) {
      return false;
    }
    const trace = value.selection as {
      input_record_ids: string[];
      selected_record_id: string;
    };
    return (
      recordIds.every((id) => trace.input_record_ids.includes(id)) &&
      trace.input_record_ids.every((id) => recordIds.includes(id)) &&
      recordIds.includes(trace.selected_record_id)
    );
  }
  return value.selection === undefined;
}

/**
 * Wire-shape gate for an attribution-preserving CognitionProjection. Every
 * assessment must carry the projection's own view (current projections carry
 * current-view records; recorded-at-time projections carry recorded-at-time
 * records) and match its item's holder, target identity and assessment
 * context. A current `assessed` item must carry a selection trace whose
 * candidates are exactly that item's assessment records and whose
 * `selected_record_id` names one of them.
 */
export function validateCognitionProjection(value: unknown): boolean {
  if (!isPlainObject(value) || !hasOnlyKeys(value, PROJECTION_KEYS)) {
    return false;
  }
  if (
    typeof value.view !== "string" ||
    VIEWS[value.view] !== true ||
    !isExtensionMap(value.extensions) ||
    !Array.isArray(value.items)
  ) {
    return false;
  }
  const placement: CognitionPlacement =
    value.view === "current" ? "authority" : "memory";
  return value.items.every((item) => {
    const withSelection =
      value.view === "current" &&
      isPlainObject(item) &&
      item.presence === "assessed";
    return isCognitionProjectionItem(item, placement, withSelection);
  });
}
