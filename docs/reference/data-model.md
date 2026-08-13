---
title: Data model reference
---

# Data model reference

The data layer defines the durable wire objects narrative products exchange. All objects are transport-agnostic, carry the required `extensions.<namespace>` bag, and keep core fields closed (`additionalProperties: false`). Field tables below trace to the committed schemas in [`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data) and [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common).

## Shared definitions

| Definition | Shape | Notes |
|------------|-------|-------|
| `SchemaVersion` | integer ≥ 1 | Wire schema version |
| `Timestamp` | string, RFC 3339 UTC | Created / updated / occurred times |
| `ExtensionMap` | object; keys `^[a-z][a-z0-9_-]*$`, values opaque JSON objects | Product namespace bag; round-trip preserves unknown namespaces and keys |
| `ModuleMap` | object; keys `^[a-z][a-z0-9_-]*$`, values structured JSON (object or array) | Cross-product functional-dialect bag; round-trip preserves unknown module namespaces |
| `SourceSpan` | `{ start, end }` (inclusive start, exclusive end) | Span within a source artifact |
| `TimelineScale` | open string; core list `brief`, `narrative`, `moment` | L5 projection tier |
| `ForkId` | string ≥ 1 char | Opaque world-history branch identity (`l5-fork`) |
| `Scope` | object; required `scope_id` | Shared ops selector — see [Ops wire reference](/reference/ops) |
| `BodyAttribute` | `{ trait_type, value, display_type?, max_value? }` | ERC721-style trait item; duplicate `trait_type` allowed at array level |
| `ComputableFieldMap` | open map of field names to domain values | Shared by `body.state` and `body.computable` under `l2-computable` |
| `ComputableLogEntry` | `{ logged_at, entry_id, changes[] }` + optional `session_id` / `message` | Moment-scale presentation of computable field changes (`l2-computable`) |

## KnowledgeEntry

The atomic knowledge-base unit. Required: `schema_version`, `entry_id`, `entry_type`, `canonical_name`, `status`, `body`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `entry_id` | string | Stable id, opaque to the protocol |
| `entry_type` | open string | Core list (documented, not enforced): `character`, `location`, `event` (ontology label — distinct from the TimelineEvent wire object), `scene`, `act`, `organization`, `item`, `conflict`, `info_point`, `era`, `worldbuilding`, `note`, `research`, `ability`, `rule` (ontology label — distinct from the L6 Rule wire object). Products MAY emit values outside this list |
| `canonical_name` | string ≥ 1 char | Human-stable name |
| `status` | open string | Core list (documented, not enforced): `provisional`, `confirmed`, `deprecated`, `merged`, `deleted` |
| `body` | closed object | `summary?`, `tags[]?`, `attributes[]?` (BodyAttribute); `state?` / `computable?` (ComputableFieldMap) under `l2-computable` |
| `source_anchor` | SourceAnchor, optional | Provenance pointer |
| `revision` | integer ≥ 0 | Optimistic concurrency revision |
| `created_at` / `updated_at` | Timestamp | |
| `extensions` | ExtensionMap, required | |
| `modules` | ModuleMap, optional | Capability-flagged `narrative-modules`; carries per-entry dialects (e.g. `modules.activation`) |

## Relation

Directed edge between two KnowledgeEntries (or a KnowledgeEntry and a source anchor). Required: `schema_version`, `relation_id`, `relation_type`, `from_id`, `to_id`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `relation_id` | string | Stable relation id |
| `relation_type` | open string | Core list (documented, not enforced): `related_to`, `parent_of`, `member_of`, `located_in`, `participates_in`, `causes`, `foreshadows` |
| `from_id` / `to_id` | string | Source / target endpoint ids |
| `label` | string, optional | Human label |
| `metadata` | open object, optional | |
| `revision` | integer ≥ 0 | Optimistic concurrency revision |
| `extensions` | ExtensionMap, required | |

## SourceAnchor

Pointer to a source artifact span (manuscript, scene, external locator). Required: `schema_version`, `source_id`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `source_id` | string | Opaque source locator; products define the grammar |
| `span` | SourceSpan, optional | Byte or character span within the source |
| `label` | string, optional | Human label |
| `mime_type` | string, optional | MIME type of the referenced source |
| `extensions` | ExtensionMap, required | |

## Finding

Checker output — a distinct artifact from a KnowledgeEntry `body`. Required: `schema_version`, `finding_id`, `severity`, `status`, `title`, `description`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `finding_id` | string | Stable finding id |
| `severity` | open string | Core list (documented, not enforced): `info`, `warning`, `error` |
| `status` | open string | Core list (documented, not enforced): `open`, `resolved`, `dismissed` |
| `title` / `description` | string | Short title and detail text |
| `kind` | string, optional | Checker kind or category |
| `target_entry_id` | string, optional | KnowledgeEntry the finding targets |
| `source_anchor` | SourceAnchor, optional | Provenance pointer |
| `suggested_fix` | string, optional | Suggested remediation text |
| `text_position` | object, optional | Position hint within source text |
| `extensions` | ExtensionMap, required | |

## AssemblePacket

Wire-only context-assembly payload. Required: `schema_version`, `packet_id`, `entries`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `packet_id` | string | Stable packet id |
| `entries` | array | Slim context entries (default); full KnowledgeEntry embedding is op-specific |
| `extensions` | ExtensionMap, required | |
| `modules` | ModuleMap, optional | Capability-flagged `narrative-modules`; carries packet-level recipes (`modules.placement`, `modules.activation_trace`) |

## HostCapabilityManifest

Host self-description for in-process collaboration. Required: `schema_version`, `host_id`, `roles`, `capabilities`, `namespaces`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `host_id` | string ≥ 1 char | Stable host identity, opaque to the protocol |
| `roles` | string[], min 1, unique | Open vocabulary. Core list (documented, not enforced): `data-store`, `input-source`, `checker`, `assembler`, `computable-engine` |
| `capabilities` | string[], min 1, unique | Open string capability flags. Core list (documented, not enforced): `spoke-baseline`, `l2-computable` |
| `namespaces` | string[], min 1, unique; keys `^[a-z][a-z0-9_-]*$` | Extension namespace keys this host owns in a collaboration context |
| `authority` | `{ scope_key }`, optional | Explicit single-writer authority scope; when absent with `data-store` in roles, implicit authority is this manifest's `host_id` |
| `extensions` | ExtensionMap, required | Deployment metadata — distinct surface from KnowledgeEntry `extensions` |

## Rule

Declarative constraint input to `check` — never checker output. Required: `schema_version`, `rule_id`, `canonical_name`, `kind`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `rule_id` | string | Stable rule id, opaque to the protocol |
| `canonical_name` | string ≥ 1 char | Human-stable name |
| `kind` | open string | Core list (documented, not enforced): `rule`, `prohibition`, `style` |
| `statement` | string, optional | Declarative constraint text (human- or machine-readable; products choose the grammar) |
| `target_entry_types` | string[], optional | Ontology filter matching KnowledgeEntry `entry_type` vocabulary |
| `severity_hint` | open string, optional | Core list (documented, not enforced): `info`, `warning`, `error` |
| `status` | open string, optional | Core list (documented, not enforced): `draft`, `active`, `deprecated` |
| `source_anchor` | SourceAnchor, optional | When the rule is anchored to a manuscript |
| `extensions` | ExtensionMap, required | |

## TimelineEvent

First-class when-axis temporal object (L5). Required: `schema_version`, `timeline_event_id`, `canonical_name`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `timeline_event_id` | string | Stable id, opaque to the protocol |
| `canonical_name` | string | Human-stable label |
| `timeline_scale` | TimelineScale, optional | Projection tier: `brief`, `narrative`, `moment` |
| `occurred_at` | string | RFC 3339 or opaque fuzzy label (e.g. "Third Age") |
| `description` | string, optional | Longer narrative summary |
| `participant_entry_ids` | string[], optional | Related KnowledgeEntry ids |
| `source_anchor` | SourceAnchor, optional | |
| `sort_key` | string, optional | Opaque ordering hint within a timeline |
| `fork_id` / `parent_fork_id` | ForkId, optional | World-history branch metadata (`l5-fork`) |
| `computable_logs` | ComputableLogEntry[], optional | Moment-scale computable change history (`l2-computable`) |
| `modules` | ModuleMap, optional | Capability-flagged `narrative-modules`; carries event observation metadata (`modules.observation` under `l5-mind`) |
| `extensions` | ExtensionMap, required | |

`MindState` is the companion L5 temporal record for mental state on the same when-axis — see [MindState reference](/reference/mind-state).

## Open vocabulary

`entry_type`, `relation_type`, statuses, severities, and `kind` values are **open strings with documented core lists** — the schema keeps them open, and the core lists serve as reference values. Products emit their own values; Domain Profiles document published vocabulary (for example the profile-only `entry_type: "beat"`); adapters round-trip unknown values verbatim.

## Distinct artifacts

- Rule is declarative checker **input**; Finding is checker **output** — each keeps its own role.
- `TimelineEvent` is the L5 when-axis object; `entry_type: "event"` is an ontology label — one local concept may map to both (dual-concern).
- `MindState` is the L5 temporal mental-state record (`l5-mind`); `entry_type: "character"` / profile `mind` are ontology labels — the record is strictly derivative of the holder's `modules.mental` / `modules.belief` (settled home), never a second authority.
- `HostCapabilityManifest` carries host metadata (roles, capabilities, namespaces) on its dedicated surface, separate from KnowledgeEntry `extensions`.

## Related

- [Protocol reference](/reference/protocol) — schema inventory, extensions contract, capability flags.
- [Ops wire reference](/reference/ops) — `Scope` and the ops that read these objects.
- [Concepts](/explanation/concepts) — the layers each object belongs to.
- [Domain profiles](/explanation/domain-profiles) — open-string vocabulary published over these shapes.
- [MindState reference](/reference/mind-state) — the L5 temporal mental-state record (`l5-mind`).
