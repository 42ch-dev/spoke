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
type FieldKind = "string" | "number" | "object" | "array";

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
  }
}

function matchesShape(value: Record<string, unknown>, shape: SuccessShape): boolean {
  return shape.fields.every(
    (field) =>
      field.name in value && matchesFieldKind(value[field.name], field.kind),
  );
}

/**
 * Whether `value` matches the success-payload shape for `op` (Rust §8.2
 * parity: malformed payloads must reject with `INTERNAL_ERROR` instead of
 * flowing through `spokeOk` unvalidated).
 *
 * Ops outside the baseline catalogue have no shape table entry and are
 * accepted as-is (the dispatch gate owns the op vocabulary).
 */
export function isValidSuccessPayload(op: string, value: unknown): boolean {
  const shape = SUCCESS_SHAPES[op];
  if (shape === undefined) {
    return true;
  }
  if (shape.kind === "object") {
    return isRecord(value) && matchesShape(value, shape);
  }
  return (
    Array.isArray(value) &&
    value.every((element) => isRecord(element) && matchesShape(element, shape))
  );
}
