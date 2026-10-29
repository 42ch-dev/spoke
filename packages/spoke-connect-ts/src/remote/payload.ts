/**
 * Success-payload shape validation for `RemoteAdapter` invoke responses
 * (frozen contract §5.2 / §8.2 — Rust parity).
 *
 * The Rust adapter deserializes the success `payload` into the expected wire
 * type (`serde_json::from_value::<T>`); a payload that does not deserialize
 * rejects with `INTERNAL_ERROR` `details.kind = "transport"`. This module
 * mirrors that rejection for the TS side, where the generic `T` would
 * otherwise flow through `spokeOk` unvalidated (`spokeOk(garbage)`).
 *
 * Scope: the structural surface of the required fields — top-level JSON kind,
 * required-field presence, and the primitive JSON kind of each required
 * field (types mirrored from `schemas/data/*.schema.json`). Recursive
 * schema-level validation (enum membership, nested object bodies) is the
 * schema package's job and is intentionally not duplicated here.
 */

/** Primitive JSON kinds the required-field guards enforce. */
type FieldKind = "string" | "number" | "object" | "array" | "boolean";

interface Field {
  name: string;
  kind: FieldKind;
}

/** Required-field shape of one ops/data wire type (schemas/data/*.schema.json). */
interface SuccessShape {
  /** Top-level JSON kind of the success payload. */
  kind: "object" | "array";
  /** Required fields (of the payload, or of each array element). */
  fields: readonly Field[];
}

const KNOWLEDGE_ENTRY_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "entry_id", kind: "string" },
  { name: "entry_type", kind: "string" },
  { name: "canonical_name", kind: "string" },
  { name: "status", kind: "string" },
  { name: "body", kind: "object" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const RELATION_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "relation_id", kind: "string" },
  { name: "relation_type", kind: "string" },
  { name: "from_id", kind: "string" },
  { name: "to_id", kind: "string" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const FINDING_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "finding_id", kind: "string" },
  { name: "severity", kind: "string" },
  { name: "status", kind: "string" },
  { name: "title", kind: "string" },
  { name: "description", kind: "string" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const RULE_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "rule_id", kind: "string" },
  { name: "canonical_name", kind: "string" },
  { name: "kind", kind: "string" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const TIMELINE_EVENT_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "timeline_event_id", kind: "string" },
  { name: "canonical_name", kind: "string" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const HOST_MANIFEST_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "host_id", kind: "string" },
  { name: "roles", kind: "array" },
  { name: "capabilities", kind: "array" },
  { name: "namespaces", kind: "array" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

/** Required fields shared by every memory receipt state (committed,
 * pending, failed) — the base receipt envelope
 * (schemas/data/memory-receipt.schema.json). */
const MEMORY_RECEIPT_FIELDS = [
  { name: "schema_version", kind: "number" },
  { name: "receipt_id", kind: "string" },
  { name: "op", kind: "string" },
  { name: "space_id", kind: "string" },
  { name: "requester_principal_id", kind: "string" },
  { name: "payload_digest", kind: "string" },
  { name: "status", kind: "string" },
  { name: "created_at", kind: "string" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const MEMORY_DESCRIBE_FIELDS = [
  { name: "memory_protocol_version", kind: "number" },
  { name: "capabilities", kind: "array" },
  { name: "limits", kind: "object" },
  { name: "required_profile_kinds", kind: "array" },
  { name: "engine_policy_version", kind: "string" },
] as const satisfies readonly Field[];

const MEMORY_RECALL_FIELDS = [
  { name: "records", kind: "array" },
  { name: "total_authorized", kind: "number" },
  { name: "truncated", kind: "boolean" },
  { name: "engine_policy_version", kind: "string" },
] as const satisfies readonly Field[];

const MEMORY_RECORDS_LIST_FIELDS = [
  { name: "records", kind: "array" },
  { name: "total_authorized", kind: "number" },
] as const satisfies readonly Field[];

const MEMORY_REFLECTION_STATE_FIELDS = [
  { name: "state_digest", kind: "string" },
  { name: "basis", kind: "object" },
  { name: "stale", kind: "boolean" },
  { name: "missing_required_profile_kinds", kind: "array" },
  { name: "data_sufficiency", kind: "string" },
] as const satisfies readonly Field[];

/** Success-payload shape per `port.*` op (contract §5.2 catalogue). */
const SUCCESS_SHAPES: Readonly<Record<string, SuccessShape>> = {
  "port.knowledge.get": { kind: "object", fields: KNOWLEDGE_ENTRY_FIELDS },
  "port.knowledge.put": { kind: "object", fields: KNOWLEDGE_ENTRY_FIELDS },
  "port.relation.get": { kind: "object", fields: RELATION_FIELDS },
  "port.relation.put": { kind: "object", fields: RELATION_FIELDS },
  "port.scope.list_knowledge_entries": {
    kind: "array",
    fields: KNOWLEDGE_ENTRY_FIELDS,
  },
  "port.scope.list_timeline_events": {
    kind: "array",
    fields: TIMELINE_EVENT_FIELDS,
  },
  "port.finding.put": { kind: "array", fields: FINDING_FIELDS },
  "port.rule.list": { kind: "array", fields: RULE_FIELDS },
  "port.host.list_peer_manifests": { kind: "array", fields: HOST_MANIFEST_FIELDS },
  // Memory reads (schemas/ops/memory-*-response.schema.json success branch).
  "memory.describe": { kind: "object", fields: MEMORY_DESCRIBE_FIELDS },
  "memory.spaces.list": {
    kind: "object",
    fields: [{ name: "spaces", kind: "array" }],
  },
  "memory.subjects.get": {
    kind: "object",
    fields: [{ name: "subject", kind: "object" }],
  },
  "memory.candidates.get": {
    kind: "object",
    fields: [{ name: "candidate", kind: "object" }],
  },
  "memory.candidates.list": {
    kind: "object",
    fields: [{ name: "candidates", kind: "array" }],
  },
  "memory.grants.list": {
    kind: "object",
    fields: [{ name: "grants", kind: "array" }],
  },
  "memory.records.get": {
    kind: "object",
    fields: [{ name: "record", kind: "object" }],
  },
  "memory.records.list": { kind: "object", fields: MEMORY_RECORDS_LIST_FIELDS },
  "memory.recall": { kind: "object", fields: MEMORY_RECALL_FIELDS },
  "memory.profile.get": {
    kind: "object",
    fields: [{ name: "profiles", kind: "array" }],
  },
  "memory.reflections.state": {
    kind: "object",
    fields: MEMORY_REFLECTION_STATE_FIELDS,
  },
  "memory.reflections.list": {
    kind: "object",
    fields: [{ name: "reflections", kind: "array" }],
  },
  "memory.exports.get": {
    kind: "object",
    fields: [{ name: "chunk", kind: "object" }],
  },
  "memory.receipts.get": {
    kind: "object",
    fields: [{ name: "receipt", kind: "object" }],
  },
  // Receipt states: every mutating memory op answers with a receipt whose
  // envelope fields are pinned above (committed and pending alike).
  "memory.spaces.freeze": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.spaces.unfreeze": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.subjects.ensure": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.subjects.fence": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.grants.grant": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.grants.revoke": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.capture": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.candidates.dismiss": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.candidates.review": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.records.create": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.records.update": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.records.delete": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.records.promote": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.profile.update": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.reflections.generate": {
    kind: "object",
    fields: MEMORY_RECEIPT_FIELDS,
  },
  "memory.exports.create": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.imports.stage": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.imports.activate": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  "memory.imports.discard": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
  extract: {
    kind: "object",
    fields: [
      { name: "candidates", kind: "array" },
      { name: "run", kind: "object" },
    ],
  },
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function matchesFieldKind(value: unknown, kind: FieldKind): boolean {
  switch (kind) {
    case "string":
      return typeof value === "string";
    case "number":
      return typeof value === "number" && Number.isFinite(value);
    case "object":
      return isRecord(value);
    case "array":
      return Array.isArray(value);
    case "boolean":
      return typeof value === "boolean";
  }
}

function matchesShape(value: Record<string, unknown>, shape: SuccessShape): boolean {
  return shape.fields.every(
    (field) =>
      field.name in value && matchesFieldKind(value[field.name], field.kind),
  );
}

const EXTRACT_RESPONSE_WIRE_KEYS = new Set(["candidates", "run", "extensions"]);

const EXTRACTION_RUN_WIRE_KEYS = new Set(["run_id", "method", "coverage_hint"]);

const EXTENSION_MAP_KEY_PATTERN = /^[a-z][a-z0-9_-]*$/;

function isValidExtensionMap(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const [key, namespaceValue] of Object.entries(value)) {
    if (!EXTENSION_MAP_KEY_PATTERN.test(key)) {
      return false;
    }
    if (!isRecord(namespaceValue)) {
      return false;
    }
  }
  return true;
}

function isValidExtractionRunMetadata(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const key of Object.keys(value)) {
    if (!EXTRACTION_RUN_WIRE_KEYS.has(key)) {
      return false;
    }
  }
  if (typeof value.run_id !== "string" || value.run_id.length === 0) {
    return false;
  }
  if (
    "method" in value &&
    value.method !== undefined &&
    (typeof value.method !== "string" || value.method.length === 0)
  ) {
    return false;
  }
  return true;
}

const EXTRACT_CANDIDATE_SHAPE: SuccessShape = {
  kind: "object",
  fields: KNOWLEDGE_ENTRY_FIELDS,
};

/**
 * Whether `value` matches the success-payload shape for `op` (Rust §8.2
 * parity: malformed payloads must reject with `INTERNAL_ERROR` instead of
 * flowing through `spokeOk` unvalidated).
 *
 * Ops outside the baseline catalogue have no shape table entry and are
 * accepted as-is (the dispatch gate owns the op vocabulary).
 */
function isValidExtractSuccessPayload(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const key of Object.keys(value)) {
    if (!EXTRACT_RESPONSE_WIRE_KEYS.has(key)) {
      return false;
    }
  }
  if (!Array.isArray(value.candidates)) {
    return false;
  }
  if (
    !value.candidates.every(
      (candidate) =>
        isRecord(candidate) && matchesShape(candidate, EXTRACT_CANDIDATE_SHAPE),
    )
  ) {
    return false;
  }
  if (!isValidExtractionRunMetadata(value.run)) {
    return false;
  }
  if (
    "extensions" in value &&
    value.extensions !== undefined &&
    !isValidExtensionMap(value.extensions)
  ) {
    return false;
  }
  return true;
}

export function isValidSuccessPayload(op: string, value: unknown): boolean {
  const shape = SUCCESS_SHAPES[op];
  if (shape === undefined) {
    return true;
  }
  if (op === "extract") {
    return isValidExtractSuccessPayload(value);
  }
  if (shape.kind === "object") {
    return isRecord(value) && matchesShape(value, shape);
  }
  return (
    Array.isArray(value) &&
    value.every((element) => isRecord(element) && matchesShape(element, shape))
  );
}
