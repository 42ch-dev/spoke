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

import { isRevisionInteger, isStrictTimestamp } from "@42ch/spoke-operations";

type FieldKind =
  | "string"
  | "number"
  | "object"
  | "array"
  | "boolean"
  | "datetime"
  | "schema_version"
  | "revision"
  | "record_revision";

interface Field {
  name: string;
  kind: FieldKind;
  optional?: boolean;
  nested?: readonly Field[];
  elements?: readonly Field[];
}

interface SuccessShape {
  kind: "object" | "array";
  fields: readonly Field[];
}

const SOURCE_ANCHOR_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
] as const satisfies readonly Field[];

const ADOPTION_ANCHOR_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "source_revision", kind: "revision" },
] as const satisfies readonly Field[];

const KNOWLEDGE_ENTRY_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "entry_id", kind: "string" },
  { name: "entry_type", kind: "string" },
  { name: "canonical_name", kind: "string" },
  { name: "status", kind: "string" },
  { name: "body", kind: "object" },
  { name: "extensions", kind: "object" },
  { name: "source_anchor", kind: "object", optional: true, nested: SOURCE_ANCHOR_SCALARS },
  { name: "adoption_anchor", kind: "object", optional: true, nested: ADOPTION_ANCHOR_SCALARS },
  { name: "revision", kind: "revision", optional: true },
  { name: "created_at", kind: "datetime", optional: true },
  { name: "updated_at", kind: "datetime", optional: true },
  { name: "modules", kind: "object", optional: true },
  { name: "owner", kind: "string", optional: true },
  { name: "disclosure", kind: "string", optional: true },
] as const satisfies readonly Field[];

const RELATION_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "relation_id", kind: "string" },
  { name: "relation_type", kind: "string" },
  { name: "from_id", kind: "string" },
  { name: "to_id", kind: "string" },
  { name: "extensions", kind: "object" },
  { name: "revision", kind: "revision", optional: true },
  { name: "created_at", kind: "datetime", optional: true },
  { name: "updated_at", kind: "datetime", optional: true },
  { name: "label", kind: "string", optional: true },
  { name: "metadata", kind: "object", optional: true },
] as const satisfies readonly Field[];

const FINDING_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "finding_id", kind: "string" },
  { name: "severity", kind: "string" },
  { name: "status", kind: "string" },
  { name: "title", kind: "string" },
  { name: "description", kind: "string" },
  { name: "extensions", kind: "object" },
  { name: "created_at", kind: "datetime", optional: true },
  { name: "updated_at", kind: "datetime", optional: true },
  { name: "source_anchor", kind: "object", optional: true, nested: SOURCE_ANCHOR_SCALARS },
  { name: "text_position", kind: "object", optional: true },
] as const satisfies readonly Field[];

const RULE_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "rule_id", kind: "string" },
  { name: "canonical_name", kind: "string" },
  { name: "kind", kind: "string" },
  { name: "extensions", kind: "object" },
  { name: "created_at", kind: "datetime", optional: true },
  { name: "updated_at", kind: "datetime", optional: true },
] as const satisfies readonly Field[];

const TIMELINE_EVENT_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "timeline_event_id", kind: "string" },
  { name: "canonical_name", kind: "string" },
  { name: "extensions", kind: "object" },
  { name: "created_at", kind: "datetime", optional: true },
  { name: "updated_at", kind: "datetime", optional: true },
] as const satisfies readonly Field[];

const HOST_MANIFEST_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "host_id", kind: "string" },
  { name: "roles", kind: "array" },
  { name: "capabilities", kind: "array" },
  { name: "namespaces", kind: "array" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const MEMORY_RECEIPT_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "receipt_id", kind: "string" },
  { name: "op", kind: "string" },
  { name: "space_id", kind: "string" },
  { name: "requester_principal_id", kind: "string" },
  { name: "payload_digest", kind: "string" },
  { name: "status", kind: "string" },
  { name: "created_at", kind: "datetime" },
  { name: "completed_at", kind: "datetime", optional: true },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const MEMORY_GRANT_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "created_at", kind: "datetime" },
  { name: "expires_at", kind: "datetime" },
  { name: "revoked_at", kind: "datetime" },
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const MEMORY_CANDIDATE_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "occurred_at", kind: "datetime" },
  { name: "created_at", kind: "datetime" },
  { name: "expires_at", kind: "datetime" },
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const MEMORY_RECORD_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "occurred_at", kind: "datetime" },
  { name: "created_at", kind: "datetime" },
  { name: "updated_at", kind: "datetime" },
  { name: "revision", kind: "record_revision" },
  {
    name: "provenance",
    kind: "object",
    nested: [{ name: "captured_at", kind: "datetime" }],
  },
  {
    name: "import_ref",
    kind: "object",
    optional: true,
    nested: [{ name: "source_revision", kind: "revision" }],
  },
  {
    name: "tombstone",
    kind: "object",
    optional: true,
    nested: [{ name: "deleted_at", kind: "datetime" }],
  },
] as const satisfies readonly Field[];

const MEMORY_SPACE_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "created_at", kind: "datetime" },
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const SUBJECT_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "created_at", kind: "datetime" },
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const MEMORY_PROFILE_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "created_at", kind: "datetime" },
  { name: "updated_at", kind: "datetime" },
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const MEMORY_REFLECTION_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "created_at", kind: "datetime" },
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const MEMORY_EXPORT_HEADER_SCALARS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "created_at", kind: "datetime" },
  { name: "extensions", kind: "object", optional: true },
] as const satisfies readonly Field[];

const MEMORY_RECEIPT_SUMMARY_SCALARS = [
  { name: "created_at", kind: "datetime" },
] as const satisfies readonly Field[];

const MEMORY_GRANT_SUMMARY_SCALARS = [
  { name: "revision", kind: "revision" },
] as const satisfies readonly Field[];

const MEMORY_EXPORT_CHUNK_FIELDS = [
  { name: "export_id", kind: "string" },
  { name: "chunk_ordinal", kind: "number" },
  { name: "chunk_count", kind: "number" },
  { name: "bundle_digest", kind: "string" },
  { name: "chunk_digest", kind: "string" },
  { name: "bytes", kind: "number" },
  {
    name: "header",
    kind: "object",
    optional: true,
    nested: MEMORY_EXPORT_HEADER_SCALARS,
  },
  { name: "subjects", kind: "array", optional: true, elements: SUBJECT_SCALARS },
  {
    name: "records",
    kind: "array",
    optional: true,
    elements: MEMORY_RECORD_SCALARS,
  },
  {
    name: "profiles",
    kind: "array",
    optional: true,
    elements: MEMORY_PROFILE_SCALARS,
  },
  {
    name: "reflections",
    kind: "array",
    optional: true,
    elements: MEMORY_REFLECTION_SCALARS,
  },
  {
    name: "candidates",
    kind: "array",
    optional: true,
    elements: MEMORY_CANDIDATE_SCALARS,
  },
  {
    name: "receipt_summaries",
    kind: "array",
    optional: true,
    elements: MEMORY_RECEIPT_SUMMARY_SCALARS,
  },
  {
    name: "grants",
    kind: "array",
    optional: true,
    elements: MEMORY_GRANT_SUMMARY_SCALARS,
  },
] as const satisfies readonly Field[];

const MEMORY_DESCRIBE_FIELDS = [
  { name: "memory_protocol_version", kind: "number" },
  { name: "capabilities", kind: "array" },
  { name: "limits", kind: "object" },
  { name: "required_profile_kinds", kind: "array" },
  { name: "engine_policy_version", kind: "string" },
] as const satisfies readonly Field[];

const MEMORY_RECALL_FIELDS = [
  { name: "records", kind: "array", elements: MEMORY_RECORD_SCALARS },
  { name: "total_authorized", kind: "number" },
  { name: "truncated", kind: "boolean" },
  { name: "engine_policy_version", kind: "string" },
] as const satisfies readonly Field[];

const MEMORY_RECORDS_LIST_FIELDS = [
  { name: "records", kind: "array", elements: MEMORY_RECORD_SCALARS },
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
  "memory.describe": { kind: "object", fields: MEMORY_DESCRIBE_FIELDS },
  "memory.spaces.list": {
    kind: "object",
    fields: [{ name: "spaces", kind: "array", elements: MEMORY_SPACE_SCALARS }],
  },
  "memory.subjects.get": {
    kind: "object",
    fields: [{ name: "subject", kind: "object", nested: SUBJECT_SCALARS }],
  },
  "memory.candidates.get": {
    kind: "object",
    fields: [
      { name: "candidate", kind: "object", nested: MEMORY_CANDIDATE_SCALARS },
    ],
  },
  "memory.candidates.list": {
    kind: "object",
    fields: [
      { name: "candidates", kind: "array", elements: MEMORY_CANDIDATE_SCALARS },
    ],
  },
  "memory.grants.list": {
    kind: "object",
    fields: [{ name: "grants", kind: "array", elements: MEMORY_GRANT_SCALARS }],
  },
  "memory.records.get": {
    kind: "object",
    fields: [{ name: "record", kind: "object", nested: MEMORY_RECORD_SCALARS }],
  },
  "memory.records.list": { kind: "object", fields: MEMORY_RECORDS_LIST_FIELDS },
  "memory.recall": { kind: "object", fields: MEMORY_RECALL_FIELDS },
  "memory.profile.get": {
    kind: "object",
    fields: [
      { name: "profiles", kind: "array", elements: MEMORY_PROFILE_SCALARS },
    ],
  },
  "memory.reflections.state": {
    kind: "object",
    fields: MEMORY_REFLECTION_STATE_FIELDS,
  },
  "memory.reflections.list": {
    kind: "object",
    fields: [
      { name: "reflections", kind: "array", elements: MEMORY_REFLECTION_SCALARS },
    ],
  },
  "memory.exports.get": {
    kind: "object",
    fields: [
      {
        name: "chunk",
        kind: "object",
        nested: MEMORY_EXPORT_CHUNK_FIELDS,
      },
    ],
  },
  "memory.receipts.get": {
    kind: "object",
    fields: [{ name: "receipt", kind: "object", nested: MEMORY_RECEIPT_FIELDS }],
  },
  "memory.spaces.create": { kind: "object", fields: MEMORY_RECEIPT_FIELDS },
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
      { name: "candidates", kind: "array", elements: KNOWLEDGE_ENTRY_FIELDS },
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
    case "datetime":
      return typeof value === "string" && isStrictTimestamp(value);
    case "schema_version":
      return (
        typeof value === "number" &&
        Number.isSafeInteger(value) &&
        value >= 1
      );
    case "revision":
      return isRevisionInteger(value);
    case "record_revision":
      return isRevisionInteger(value) && value >= 1;
  }
}

function matchesField(value: unknown, field: Field): boolean {
  if (!matchesFieldKind(value, field.kind)) {
    return false;
  }
  if (field.nested !== undefined) {
    if (!isRecord(value) || !matchesPresentFields(value, field.nested)) {
      return false;
    }
  }
  if (field.elements !== undefined) {
    if (
      !Array.isArray(value) ||
      !value.every(
        (element) =>
          isRecord(element) &&
          matchesPresentFields(element, field.elements as readonly Field[]),
      )
    ) {
      return false;
    }
  }
  return true;
}

function matchesPresentFields(
  value: Record<string, unknown>,
  fields: readonly Field[],
): boolean {
  return fields.every(
    (field) => !(field.name in value) || matchesField(value[field.name], field),
  );
}

function matchesShape(value: Record<string, unknown>, shape: SuccessShape): boolean {
  return shape.fields.every((field) => {
    if (!(field.name in value)) {
      return field.optional === true;
    }
    return matchesField(value[field.name], field);
  });
}

const EXTRACT_RESPONSE_WIRE_KEYS = new Set(["candidates", "run", "extensions"]);

const EXTRACTION_RUN_WIRE_KEYS = new Set(["run_id", "method", "coverage_hint"]);

const EXTENSION_MAP_KEY_PATTERN = /^[a-z][a-z0-9_-]*$/;

export function isValidExtensionMap(value: unknown): boolean {
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

function isValidModulesMap(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const [key, moduleValue] of Object.entries(value)) {
    if (!EXTENSION_MAP_KEY_PATTERN.test(key)) {
      return false;
    }
    if (!isRecord(moduleValue) && !Array.isArray(moduleValue)) {
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

export { isRevisionInteger } from "@42ch/spoke-operations";

export function isValidRevisionSnapshot(
  value: unknown,
  selectors?: { entity_kind: string; entity_id: string; revision: number },
): boolean {
  if (!isRecord(value) || !isValidRevisionSnapshotEnvelope(value)) {
    return false;
  }
  if (
    selectors !== undefined &&
    (value.entity_kind !== selectors.entity_kind ||
      value.entity_id !== selectors.entity_id ||
      value.revision !== selectors.revision)
  ) {
    return false;
  }
  const entity = value.snapshot;
  const isKnowledgeEntry =
    isRecord(entity) && isValidKnowledgeEntryDeep(entity);
  const isRelation = isRecord(entity) && isValidRelationDeep(entity);
  if (value.entity_kind === "knowledge_entry" && !isKnowledgeEntry) {
    return false;
  }
  if (value.entity_kind === "relation" && !isRelation) {
    return false;
  }
  if (!isKnowledgeEntry && !isRelation) {
    return false;
  }
  if (isKnowledgeEntry && entity.entry_id !== value.entity_id) {
    return false;
  }
  if (isRelation && entity.relation_id !== value.entity_id) {
    return false;
  }
  return (
    isRevisionInteger(entity.revision) &&
    entity.revision === value.revision
  );
}

const REVISION_SNAPSHOT_ENVELOPE_KEYS: Record<string, true> = {
  schema_version: true,
  entity_kind: true,
  entity_id: true,
  revision: true,
  snapshot: true,
  extensions: true,
};

function isValidRevisionSnapshotEnvelope(snapshot: unknown): boolean {
  return (
    isRecord(snapshot) &&
    Object.keys(snapshot).every(
      (key) => REVISION_SNAPSHOT_ENVELOPE_KEYS[key] === true,
    ) &&
    typeof snapshot.schema_version === "number" &&
    Number.isSafeInteger(snapshot.schema_version) &&
    snapshot.schema_version > 0 &&
    typeof snapshot.entity_kind === "string" &&
    typeof snapshot.entity_id === "string" &&
    isRevisionInteger(snapshot.revision) &&
    isRecord(snapshot.snapshot) &&
    isValidExtensionMap(snapshot.extensions)
  );
}

const KNOWLEDGE_ENTRY_SHAPE: SuccessShape = {
  kind: "object",
  fields: KNOWLEDGE_ENTRY_FIELDS,
};

const RELATION_SHAPE: SuccessShape = {
  kind: "object",
  fields: RELATION_FIELDS,
};

const SOURCE_ANCHOR_REQUIRED_FIELDS = [
  { name: "schema_version", kind: "schema_version" },
  { name: "source_id", kind: "string" },
  { name: "extensions", kind: "object" },
] as const satisfies readonly Field[];

const SOURCE_ANCHOR_OPTIONAL_FIELDS = [
  { name: "span", kind: "object" },
  { name: "label", kind: "string" },
  { name: "mime_type", kind: "string" },
] as const satisfies readonly Field[];

const SOURCE_SPAN_REQUIRED_FIELDS = [
  { name: "start", kind: "number" },
  { name: "end", kind: "number" },
] as const satisfies readonly Field[];

const BODY_ATTRIBUTE_OPTIONAL_FIELDS = [
  { name: "display_type", kind: "string" },
  { name: "max_value", kind: "number" },
] as const satisfies readonly Field[];

function matchesOptionalFields(
  value: Record<string, unknown>,
  fields: readonly Field[],
): boolean {
  return fields.every(
    (field) =>
      !(field.name in value) ||
      matchesFieldKind(value[field.name], field.kind),
  );
}

const SOURCE_SPAN_WIRE_KEYS: Record<string, true> = { start: true, end: true };

function isValidSourceSpan(value: unknown): boolean {
  return (
    isRecord(value) &&
    Object.keys(value).every((key) => SOURCE_SPAN_WIRE_KEYS[key] === true) &&
    SOURCE_SPAN_REQUIRED_FIELDS.every(
      (field) =>
        field.name in value &&
        matchesFieldKind(value[field.name], field.kind),
    ) &&
    // `start`/`end` are plain numbers on the wire (Rust `f64`) — decimals
    // are schema-valid, only non-numbers fail.
    Number.isFinite(value.start) &&
    Number.isFinite(value.end)
  );
}
const SOURCE_ANCHOR_WIRE_KEYS: Record<string, true> = {
  schema_version: true,
  source_id: true,
  span: true,
  label: true,
  mime_type: true,
  extensions: true,
};

function isValidSourceAnchor(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  if (
    !Object.keys(value).every((key) => SOURCE_ANCHOR_WIRE_KEYS[key] === true)
  ) {
    return false;
  }
  if (
    !SOURCE_ANCHOR_REQUIRED_FIELDS.every(
      (field) =>
        field.name in value &&
        matchesFieldKind(value[field.name], field.kind),
    ) ||
    // SchemaVersion is an integer >= 1 — a loose number is malformed.
    !Number.isSafeInteger(value.schema_version) ||
    (value.schema_version as number) < 1 ||
    !isValidExtensionMap(value.extensions)
  ) {
    return false;
  }
  if (!matchesOptionalFields(value, SOURCE_ANCHOR_OPTIONAL_FIELDS)) {
    return false;
  }
  return !("span" in value) || isValidSourceSpan(value.span);
}

const BODY_ATTRIBUTE_WIRE_KEYS: Record<string, true> = {
  trait_type: true,
  value: true,
  display_type: true,
  max_value: true,
};

function isValidBodyAttribute(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  if (
    !Object.keys(value).every((key) => BODY_ATTRIBUTE_WIRE_KEYS[key] === true)
  ) {
    return false;
  }
  if (typeof value.trait_type !== "string" || value.trait_type.length === 0) {
    return false;
  }
  if (
    !("value" in value) ||
    (typeof value.value !== "string" &&
      typeof value.value !== "number" &&
      typeof value.value !== "boolean")
  ) {
    return false;
  }
  return matchesOptionalFields(value, BODY_ATTRIBUTE_OPTIONAL_FIELDS);
}

const KNOWLEDGE_ENTRY_BODY_WIRE_KEYS: Record<string, true> = {
  summary: true,
  tags: true,
  attributes: true,
  state: true,
  computable: true,
};

function isValidKnowledgeEntryBody(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  if (
    !Object.keys(value).every(
      (key) => KNOWLEDGE_ENTRY_BODY_WIRE_KEYS[key] === true,
    )
  ) {
    return false;
  }
  if ("summary" in value && typeof value.summary !== "string") {
    return false;
  }
  if (
    "tags" in value &&
    (!Array.isArray(value.tags) ||
      !value.tags.every((tag) => typeof tag === "string"))
  ) {
    return false;
  }
  if (
    "attributes" in value &&
    (!Array.isArray(value.attributes) ||
      !value.attributes.every(isValidBodyAttribute))
  ) {
    return false;
  }
  if ("state" in value && !isRecord(value.state)) {
    return false;
  }
  return !("computable" in value) || isRecord(value.computable);
}

const ADOPTION_ANCHOR_WIRE_KEYS: Record<string, true> = {
  schema_version: true,
  source_scope_id: true,
  source_entry_id: true,
  source_revision: true,
  extensions: true,
};

/**
 * The `AdoptionAnchor` schema check (adoption-anchor.schema.json, closed
 * member set): an integer `schema_version >= 1`, non-empty
 * `source_scope_id` / `source_entry_id`, a non-negative integer
 * `source_revision`, and a shared-`ExtensionMap` `extensions` member.
 */
export function isWellFormedAdoptionAnchor(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const key of Object.keys(value)) {
    if (ADOPTION_ANCHOR_WIRE_KEYS[key] !== true) {
      return false;
    }
  }
  if (
    typeof value.schema_version !== "number" ||
    !Number.isSafeInteger(value.schema_version) ||
    value.schema_version < 1 ||
    typeof value.source_scope_id !== "string" ||
    value.source_scope_id.length === 0 ||
    typeof value.source_entry_id !== "string" ||
    value.source_entry_id.length === 0 ||
    !isRevisionInteger(value.source_revision)
  ) {
    return false;
  }
  return isValidExtensionMap(value.extensions);
}

const KNOWLEDGE_ENTRY_WIRE_KEYS: Record<string, true> = {
  schema_version: true,
  entry_id: true,
  entry_type: true,
  canonical_name: true,
  status: true,
  body: true,
  source_anchor: true,
  adoption_anchor: true,
  revision: true,
  created_at: true,
  updated_at: true,
  extensions: true,
  modules: true,
  owner: true,
  disclosure: true,
};

export function isValidKnowledgeEntryDeep(entity: Record<string, unknown>): boolean {
  if (
    !Object.keys(entity).every((key) => KNOWLEDGE_ENTRY_WIRE_KEYS[key] === true)
  ) {
    return false;
  }
  if (!matchesShape(entity, KNOWLEDGE_ENTRY_SHAPE)) {
    return false;
  }
  // SchemaVersion is an integer >= 1; `canonical_name` is minLength 1.
  if (
    !Number.isSafeInteger(entity.schema_version) ||
    (entity.schema_version as number) < 1 ||
    (entity.canonical_name as string).length === 0
  ) {
    return false;
  }
  if (!isValidExtensionMap(entity.extensions)) {
    return false;
  }
  if ("modules" in entity && !isValidModulesMap(entity.modules)) {
    return false;
  }
  // `revision` is revision-typed payload: the interoperable integer
  // domain is `[0, 2^53]` inclusive (contract E1); `owner` / `disclosure`
  // are non-empty when present.
  if ("revision" in entity && !isRevisionInteger(entity.revision)) {
    return false;
  }
  for (const key of ["owner", "disclosure"] as const) {
    if (key in entity && (entity[key] as string).length === 0) {
      return false;
    }
  }
  if (!isValidKnowledgeEntryBody(entity.body)) {
    return false;
  }
  if ("source_anchor" in entity && !isValidSourceAnchor(entity.source_anchor)) {
    return false;
  }
  return (
    !("adoption_anchor" in entity) ||
    isWellFormedAdoptionAnchor(entity.adoption_anchor)
  );
}

function isValidRelationDeep(entity: Record<string, unknown>): boolean {
  return (
    matchesShape(entity, RELATION_SHAPE) && isValidExtensionMap(entity.extensions)
  );
}

function isValidHistorySuccessPayload(op: string, value: unknown): boolean {
  if (op === "history.revisions.get") {
    return (
      isRecord(value) &&
      "snapshot" in value &&
      isValidRevisionSnapshot(value.snapshot)
    );
  }
  return (
    isRecord(value) &&
    isRevisionInteger(value.deleted) &&
    "floor_revision" in value &&
    (value.floor_revision === null ||
      isRevisionInteger(value.floor_revision))
  );
}

/** A `knowledge.adopt` response payload that carries the failure `error`
 * member at all (valid or not): lets the client gate separate a malformed
 * failure — a MIXED envelope (`outcome`/`entry` alongside `error`)
 * included — from the success shape. Rust's typed untagged decode refuses
 * a mixed envelope, so neither branch may accept it (qc1 F-032). */
export function carriesAdoptFailureMember(value: unknown): boolean {
  return isRecord(value) && "error" in value;
}

/** Served `knowledge.adopt` failure branch (contract §`KnowledgeAdoptResponse`
 * `OneOf1`, E3): a closed `{error, extensions?}` envelope. A provider
 * refusal travels as the response PAYLOAD (a successful port outcome),
 * never a connect error envelope — the client gate must accept it
 * verbatim (qc2 QC2-21 / F-030; Rust `invoke_mapped<KnowledgeAdoptResponse>`
 * decodes both branches). */
export function isAdoptFailurePayload(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const member of Object.keys(value)) {
    if (member !== "error" && member !== "extensions") {
      return false;
    }
  }
  const error = value.error;
  if (!isRecord(error)) {
    return false;
  }
  for (const member of Object.keys(error)) {
    if (
      member !== "code" &&
      member !== "message" &&
      member !== "details" &&
      member !== "extensions"
    ) {
      return false;
    }
  }
  if (
    typeof error.code !== "string" ||
    typeof error.message !== "string" ||
    !isValidExtensionMap(error.extensions)
  ) {
    return false;
  }
  if ("details" in error) {
    if (!isRecord(error.details)) {
      return false;
    }
    if (
      "adopted_revision" in error.details &&
      !isRevisionInteger(error.details.adopted_revision)
    ) {
      return false;
    }
  }
  if ("extensions" in value && !isValidExtensionMap(value.extensions)) {
    return false;
  }
  return true;
}

/** Served `knowledge.adopt` success branch (knowledge-adopt-response
 * schema `OneOf0`, `additionalProperties: false`): the closed top-level
 * member set `{outcome, entry, extensions?}`; `outcome` is the closed
 * vocabulary `"adopted" | "already_present"`; `extensions` is an
 * `ExtensionMap` when supplied; and `entry` is a COMPLETE `KnowledgeEntry`
 * carrying a well-formed `adoption_anchor` — the same bar the serving face
 * and the typed Rust `OneOfEntry` decode apply. Accepting the shallow
 * `{outcome, entry}` shape alone would surface `spokeOk` for a payload the
 * Rust client refuses (qc1 F-033 / round-15). */
export function isValidAdoptSuccessPayload(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  for (const member of Object.keys(value)) {
    if (member !== "outcome" && member !== "entry" && member !== "extensions") {
      return false;
    }
  }
  if (value.outcome !== "adopted" && value.outcome !== "already_present") {
    return false;
  }
  if ("extensions" in value && !isValidExtensionMap(value.extensions)) {
    return false;
  }
  const entry = value.entry;
  return (
    isRecord(entry) &&
    isValidKnowledgeEntryDeep(entry) &&
    isWellFormedAdoptionAnchor(entry.adoption_anchor)
  );
}

export function isValidSuccessPayload(op: string, value: unknown): boolean {
  if (op === "history.revisions.get" || op === "history.revisions.prune") {
    return isValidHistorySuccessPayload(op, value);
  }
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
