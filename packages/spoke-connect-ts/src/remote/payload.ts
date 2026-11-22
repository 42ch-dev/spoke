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

type FieldKind =
  | "string"
  | "number"
  | "object"
  | "array"
  | "boolean"
  | "datetime";

interface Field {
  name: string;
  kind: FieldKind;
}

/** note: required-field shape of one ops/data wire type (schemas/data/*.schema.json). */
interface SuccessShape {
  kind: "object" | "array";
  /** note: required fields (of the payload, or of each array element). */
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

/** note: required fields shared by every memory receipt state (committed,
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
  { name: "created_at", kind: "datetime" },
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
  // note: memory reads (schemas/ops/memory-*-response.schema.json success branch).
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
  // note: every mutating memory op answers with a receipt whose envelope
  // fields are pinned above (committed and pending alike).
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
  // note: `knowledge.adopt` success branch (knowledge-adopt-response
  // schema): `{ outcome, entry }` — the error branch never reaches this
  // gate (a refusal answers the connect error envelope).
  "knowledge.adopt": {
    kind: "object",
    fields: [
      { name: "outcome", kind: "string" },
      { name: "entry", kind: "object" },
    ],
  },
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
    case "datetime":
      return isRfc3339Timestamp(value);
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

/** note: chrono `DateTime<Utc>` serde parity — RFC 3339 (`date-time`):
 * full date, `T`/`t` separator (chrono's serde impl accepts both cases),
 * seconds, optional fractional seconds, `Z`/`z` or ±hh:mm offset.
 * `Date.parse` (NaN rejects) covers the ±23:59 offset ceiling; calendar
 * days and clock fields are range-checked here because `Date.parse`
 * normalizes `2026-02-30` and `24:00:00` into valid dates while chrono
 * rejects them, and `Date.parse` rejects second `:60` while chrono
 * accepts leap seconds. */
const RFC3339_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;

const MONTH_DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

function isRfc3339Timestamp(value: unknown): value is string {
  if (typeof value !== "string") {
    return false;
  }
  const match = RFC3339_PATTERN.exec(value);
  if (match === null) {
    return false;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  if (hour > 23 || minute > 59 || second > 60) {
    return false;
  }
  // `Date.parse` normalizes out-of-range calendar days (`2026-02-30`
  // rolls into March) while chrono rejects them, so bound the day here.
  const daysInMonth =
    month === 2 && year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0)
      ? 29
      : (MONTH_DAYS[month - 1] ?? 0);
  if (day < 1 || day > daysInMonth) {
    return false;
  }
  const offset = /([+-])(\d{2}):(\d{2})$/.exec(value);
  if (offset !== null && (Number(offset[2]) > 23 || Number(offset[3]) > 59)) {
    return false;
  }
  // `:60` leap second is accepted by chrono but NaN in `Date.parse`;
  // parse the clock one second earlier to reuse the remaining checks.
  const normalized =
    second === 60 ? `${value.slice(0, 17)}59${value.slice(19)}` : value;
  return !Number.isNaN(Date.parse(normalized));
}

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

/** note: `KnowledgeEntry.modules` (knowledge-entry.schema.json) — Rust
 * `HashMap<KnowledgeEntryModulesKey, KnowledgeEntryModulesValue>`:
 * keys match the ExtensionKey pattern; values are object | array only
 * (scalar module values fail Rust typed decode). */
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

/**
 * note: strict `extract` success shape — Rust §8.2 parity: a malformed
 * payload must reject with `INTERNAL_ERROR` instead of flowing through
 * `spokeOk` unvalidated.
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

/** note: `revision` / `deleted` / `floor_revision` are `u64` on the wire — exact integers only, never rounded (spoke-history contract). */
function isExactNonNegativeInteger(value: unknown): value is number {
  return (
    typeof value === "number" && Number.isSafeInteger(value) && value >= 0
  );
}

/**
 * note: strict `RevisionSnapshot` decode (spoke-history contract
 * §RevisionSnapshot): all six envelope members present and well-formed,
 * `snapshot` a COMPLETE core shape (`KnowledgeEntry` or `Relation`
 * required-field set — matching `port.knowledge.get` / `port.relation.get`
 * strictness), the variant agreeing with `entity_kind` for the core
 * kinds, and the nested entity's own id and revision equal to the
 * envelope's. `selectors` — bound at the serving face and each
 * `RemoteAdapter` — additionally requires the envelope to match the
 * request (Rust parity: `validate_history_snapshot`). An open product
 * kind declares its core shape, so the nested member must still decode
 * under one consistent core shape.
 */
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
  // note: RevisionSnapshot equality rules — the nested entity's own id
  // equals the envelope `entity_id` and its own `revision` equals the
  // envelope `revision` (contract §RevisionSnapshot: "equals the selected
  // entity snapshot's own revision"). The nested revision is required, not
  // conditional: the authored response example carries them equal.
  if (isKnowledgeEntry && entity.entry_id !== value.entity_id) {
    return false;
  }
  if (isRelation && entity.relation_id !== value.entity_id) {
    return false;
  }
  return (
    isExactNonNegativeInteger(entity.revision) &&
    entity.revision === value.revision
  );
}

/**
 * note: strict `RevisionSnapshot` envelope shape (spoke-history contract
 * §RevisionSnapshot): all six required members present and well-formed.
 */
function isValidRevisionSnapshotEnvelope(snapshot: unknown): boolean {
  return (
    isRecord(snapshot) &&
    typeof snapshot.schema_version === "number" &&
    Number.isInteger(snapshot.schema_version) &&
    typeof snapshot.entity_kind === "string" &&
    typeof snapshot.entity_id === "string" &&
    isExactNonNegativeInteger(snapshot.revision) &&
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

/**
 * note: optional fields of the schema-declared `KnowledgeEntry` /
 * `Relation` wire types (schemas/data/*.schema.json) — checked only when
 * present. Required members stay in the `*_FIELDS` tables above.
 */
const KNOWLEDGE_ENTRY_OPTIONAL_FIELDS = [
  { name: "source_anchor", kind: "object" },
  { name: "adoption_anchor", kind: "object" },
  { name: "revision", kind: "number" },
  { name: "created_at", kind: "datetime" },
  { name: "updated_at", kind: "datetime" },
  { name: "modules", kind: "object" },
  { name: "owner", kind: "string" },
  { name: "disclosure", kind: "string" },
] as const satisfies readonly Field[];

const RELATION_OPTIONAL_FIELDS = [
  { name: "label", kind: "string" },
  { name: "metadata", kind: "object" },
  { name: "revision", kind: "number" },
  { name: "created_at", kind: "datetime" },
  { name: "updated_at", kind: "datetime" },
] as const satisfies readonly Field[];

const SOURCE_ANCHOR_REQUIRED_FIELDS = [
  { name: "schema_version", kind: "number" },
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

/**
 * note: every listed field is optional — only present members must match
 * their declared JSON kind.
 */
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

/** note: closed `SourceSpan` member set (additionalProperties: false). */
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
    isExactNonNegativeInteger(value.start) &&
    isExactNonNegativeInteger(value.end)
  );
}
/** note: closed `SourceAnchor` member set (additionalProperties: false). */
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

/**
 * note: `BodyAttribute` (common.schema.json): closed member set —
 * `trait_type` non-empty string, `value` string | number | boolean;
 * optional `display_type` / `max_value`. An undeclared member fails Rust
 * typed decode (`deny_unknown_fields`), so extras are malformed.
 */
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

/** note: closed `KnowledgeEntry.body` member set (additionalProperties: false). */
const KNOWLEDGE_ENTRY_BODY_WIRE_KEYS: Record<string, true> = {
  summary: true,
  tags: true,
  attributes: true,
  state: true,
  computable: true,
};

/**
 * note: `KnowledgeEntry.body` (knowledge-entry.schema.json): closed member
 * set — optional `summary`/`tags`/`attributes`/`state`/`computable`, each
 * typed; an undeclared member or a present member with the wrong JSON kind
 * is malformed, not absent.
 */
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

/** note: closed `AdoptionAnchor` member set (additionalProperties: false). */
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
    !isExactNonNegativeInteger(value.source_revision)
  ) {
    return false;
  }
  return isValidExtensionMap(value.extensions);
}

/** note: closed `KnowledgeEntry` member set (additionalProperties: false). */
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

/**
 * note: full schema-declared `KnowledgeEntry` field set — the closed
 * member list, required members plus every optional member's declared
 * JSON kind, recursively for `body` / `source_anchor` / `adoption_anchor`,
 * plus the schema's range constraints (`schema_version` integer >= 1,
 * `canonical_name` / `owner` / `disclosure` non-empty, `revision` a
 * non-negative integer). An undeclared member fails Rust typed decode
 * (`deny_unknown_fields`), so extras are malformed, not tolerated.
 */
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
  if (!matchesOptionalFields(entity, KNOWLEDGE_ENTRY_OPTIONAL_FIELDS)) {
    return false;
  }
  // `revision` is a u64; `owner` / `disclosure` are non-empty when present.
  if ("revision" in entity && !isExactNonNegativeInteger(entity.revision)) {
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

/**
 * note: full schema-declared `Relation` field set — required members plus
 * every optional member's declared JSON kind. Unknown extra keys are
 * tolerated (Rust serde typed-decode parity of the declared members).
 */
function isValidRelationDeep(entity: Record<string, unknown>): boolean {
  return (
    matchesShape(entity, RELATION_SHAPE) &&
    isValidExtensionMap(entity.extensions) &&
    matchesOptionalFields(entity, RELATION_OPTIONAL_FIELDS)
  );
}

/**
 * note: strict `history.revisions.*` success shapes (spoke-history
 * contract §Connect operations): get requires a present `snapshot` member
 * decoding as `RevisionSnapshot`; prune requires `deleted` integer ≥ 0
 * and a present, nullable `floor_revision`. Malformed values fail the
 * gate → `INTERNAL_ERROR` transport decode, never `Ok`.
 */
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
    isExactNonNegativeInteger(value.deleted) &&
    "floor_revision" in value &&
    (value.floor_revision === null ||
      isExactNonNegativeInteger(value.floor_revision))
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
