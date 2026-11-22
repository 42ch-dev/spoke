# SPOKE Judgment

> **Status:** Normative (spoke-judgment optional data/conformance capability)  
> **Document class:** Detail — data + pure-validation capability layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/data/judgment.schema.json`

## Scope

The `spoke-judgment` optional data/conformance capability: the `Judgment` assessment envelope, the
`JudgmentRef` / `JudgmentEvidenceRef` reference shapes it is built on, the `verdict` core-plus-detail
split, the `judgment_kind` open vocabulary, the pure `validateJudgment` wire-shape gate and the
`judgmentsEqualCanonical` RFC 8785 comparison helper, and the capability's manifest-only
advertisement semantics.

It does not cover Connect request/response envelopes or dispatch rules (see
[`spoke-ops.md`](spoke-ops.md)), the operations-library package contract the helpers live in (see
[`spoke-operations.md`](spoke-operations.md)), cognition carriers (`MindState` family — see
[`spoke-data-model.md`](spoke-data-model.md)), or any engine, reasoner, storage behavior, or product
assessment policy. `spoke-judgment` defines no Connect operation of its own.

## Current state

### Purpose

A `Judgment` is a declared assessment about a target, backed by evidence that pins exact revisions:
a consistency verdict on a KnowledgeEntry, an evidentiary-support verdict, an acceptance decision, a
review outcome. The envelope is portable across products while assessment content stays
product-owned. Producers and validators conform by validating the envelope shape and comparing
canonical bytes; nothing about how a judgment is produced, stored, or acted on is protocol behavior.

### `JudgmentRef` and `JudgmentEvidenceRef`

`JudgmentRef` is the dedicated lightweight reference used by `Judgment.target` and every
`Judgment.evidence[]` item. It is a closed object (`additionalProperties: false`):

| Field | Presence | Type | Contract |
|---|---|---|---|
| `kind` | required | non-empty open string | Identifies the referenced thing's kind without a protocol registry. |
| `id` | required | non-empty string | Opaque stable id within the applicable kind and product scope. |
| `namespace` | optional | non-empty string | Product scope for the reference; not a required component of the referenced thing's identity. |
| `revision` | conditional | positive integer (`>= 1`) | Pins the referenced revision when revision pinning applies. |

`JudgmentRef` carries only kind, id, optional product scope, and optional revision. It does not
reuse `CognitionReference` or `CognitionTarget`; those types retain their cognition-scoped carrier,
mandatory-namespace, target, and assessment-context semantics (see
[`spoke-data-model.md`](spoke-data-model.md) § optional `spoke-cognition` MindState family).

Narrowing rule: on `Judgment.target`, `revision` is optional and positive when present. Every
`Judgment.evidence[]` item is a `JudgmentEvidenceRef` — a `JudgmentRef` with `revision` **required**
and positive. Evidence therefore always carries the exact revisions the judgment relies on; the
target may be assessed unpinned (product decision).

### `Judgment`

`Judgment` is a closed object (`additionalProperties: false`):

| Field | Presence | Type | Contract |
|---|---|---|---|
| `schema_version` | required | shared `SchemaVersion` | Integer `>= 1`; version of the `Judgment` wire schema. |
| `judgment_id` | required | non-empty string | Opaque stable judgment id. |
| `judgment_kind` | required | non-empty open string | Assessment-kind vocabulary (§ `judgment_kind` vocabulary). |
| `target` | required | `JudgmentRef` | The thing assessed. |
| `evidence` | required | `JudgmentEvidenceRef[]` | Present even when empty; every item pins a required positive `revision`. |
| `verdict` | required | `Verdict` | Closed core plus open product detail (§ `verdict` core and detail). |
| `basis` | optional | string | Product-supplied explanation or rationale; it does not replace `evidence`. |
| `created_at` | optional | shared `Timestamp` | RFC 3339 date-time with offset. |
| `extensions` | required | shared `ExtensionMap` | Product-namespace extensions; `{}` when empty. Unknown namespaces and keys round-trip unchanged (see [`spoke-data-model.md`](spoke-data-model.md) §Extensions). |

Required members are `schema_version`, `judgment_id`, `judgment_kind`, `target`, `evidence`,
`verdict`, `extensions`. Only `basis` and `created_at` are optional.

### `judgment_kind` vocabulary

`judgment_kind` is an open string. Core values (documented, not enforced by an `enum`):

| Value | Meaning |
|---|---|
| `consistency` | Assesses consistency. |
| `support` | Assesses evidentiary support. |
| `acceptance` | Assesses acceptance. |
| `review` | Records a review outcome. |

Products add namespaced kinds such as `my-product:belief-assessment`. Unknown non-empty values are
valid and need no registry update; readers still decode and interpret the common envelope fields.

### `verdict` core and detail

`verdict` is a closed object (`additionalProperties: false`):

| Field | Presence | Type | Contract |
|---|---|---|---|
| `outcome` | required | non-empty open string | Portable core outcome. |
| `confidence` | optional | number in `[0, 1]` | Core confidence value. |
| `detail` | optional | object | Product-specific assessment content, opaque to the envelope validator and included in canonicalization. |

`verdict.outcome` is an open string with no protocol-enforced vocabulary; products define outcome
values and readers interpret them.

`detail` is a free-form object except that its direct members MUST NOT duplicate protocol-level
envelope locations. The reserved direct member names are `target`, `evidence`, `judgment_kind`, and
`outcome`; for example, `verdict.detail.target` and `verdict.detail.evidence` are invalid.
Product-specific members inside `detail`, including nested objects, retain product-defined shapes.
The reserved-member check protects protocol field locations only — `detail` content shape is not
otherwise validated.

### Pure validation — `validateJudgment`

TypeScript exposes `validateJudgment(judgment: Judgment): SpokeResult<void>`; Rust exposes the
behaviorally identical `validate_judgment(judgment: &Value) -> SpokeResult<()>` (snake_case). The
helper is deterministic and performs no I/O. The TS parameter type is the generated structural
`Judgment` type; the validated value is the undecoded wire value in both languages (see
[`2026-11-23-spoke-judgment.md`](../../adr/implemented/feature/2026-11-23-spoke-judgment.md)).

`validateJudgment` checks:

1. Presence and wire shape of every required field, including the closed `Judgment`, `JudgmentRef`,
   and `verdict` objects and the required `extensions` map.
2. `schema_version` as an integer `>= 1`; `judgment_id` and `judgment_kind` as non-empty strings.
3. `target` and `evidence` reference shape: non-empty `kind` and `id`, non-empty `namespace` when
   present, and positive-integer revisions where applicable.
4. Presence of `evidence`, including the valid empty-array case, and a required positive integer
   `revision` on every evidence item.
5. `verdict.outcome` as a non-empty string, optional `verdict.confidence` as a number in `[0, 1]`,
   and optional `verdict.detail` as an object without the reserved direct members listed above.
6. Optional `basis`, `created_at`, and reference revisions against their declared wire types and
   constraints.

`validateJudgment` does **not** determine verdict truth, evidence existence or content correctness,
authorization, business thresholds, or product-specific `detail` shape. Its reserved-member check
protects protocol field locations only. It performs no registry lookup, storage access, reasoning,
or product policy evaluation.

### Refusal vocabulary

Every shape violation returns `INVALID_INPUT` with `details.field` containing the exact logical
field path. Paths use dotted members and bracketed array indices — `judgment_kind`,
`target.revision`, `evidence[0].revision`, `verdict.outcome`, `verdict.confidence`,
`verdict.detail.target`. `field` is the only protocol-required member of `details` for these
refusals; the human-readable `message` carries no stable machine semantics. The reject envelope is
the shared `SpokeResult` / `SpokeRejectCode` surface (see
[`spoke-operations.md`](spoke-operations.md) § Result / reject envelope); `INVALID_INPUT` is its
shared shape-violation code and judgment validation introduces no new reject codes.

### Canonical comparison — RFC 8785

Two independent producers of the same `Judgment` emit byte-identical bytes under RFC 8785
canonical JSON. `judgmentsEqualCanonical(left, right)` (Rust `judgments_equal_canonical`) compares
those canonical bytes and returns `SpokeResult<bool>`; it rejects `INVALID_INPUT` when either value
cannot be canonicalized. Canonicalization covers the complete envelope, including `verdict.detail`,
`extensions`, present empty arrays and objects, and all optional members that are present. `detail`
remains opaque to the envelope but is never omitted, normalized by product rules, or excluded from
canonical comparison.

### Manifest advertisement and `tools.*` transport

A host that validates or produces `Judgment` advertises `spoke-judgment` in
`HostCapabilityManifest.capabilities` (see [`spoke-data-model.md`](spoke-data-model.md) §
HostCapabilityManifest). The declaration signals data/validator conformance — nothing more.

`spoke-judgment` has **no** Connect dispatch-table row and **no** `op` of its own. A tool whose
payload carries `Judgment` is declared by its `ToolDescriptor` in
`HostCapabilityManifest.tools[]` and invoked through the existing opaque
`{ "arguments": ... }` / `{ "result": ... }` tool payloads. Each `tools.<ns>.<tool_id>` operation
is gated on exact membership of that full operation string in `negotiated_capabilities`; its
descriptor's `capability_id` is that same string and appears in the declaring manifest's
`capabilities[]`.

For example, `tools.mindcore.deliberate` is gated on `tools.mindcore.deliberate`, not on
`spoke-judgment`. `spoke-judgment` is neither a substitute nor an additional dispatch gate for any
`tools.*` operation. Connect envelopes and the existing tool dispatch rules are unchanged.

### Illustrative instance

```json
{
  "schema_version": 1,
  "judgment_id": "judg_01HXYZ",
  "judgment_kind": "consistency",
  "target": { "kind": "knowledge_entry", "id": "kb_mira" },
  "evidence": [
    { "kind": "knowledge_entry", "id": "kb_foreshadow", "revision": 2 }
  ],
  "verdict": { "outcome": "consistent", "confidence": 0.8 },
  "extensions": {}
}
```

`basis` and `created_at` are omitted here; both remain valid when present. Product assessment
content beyond the portable core belongs in `verdict.detail` (never under reserved direct member
names) or `extensions.<ns>`.

## Invariants

1. `Judgment`, `JudgmentRef`, and `verdict` are closed objects; `validateJudgment` rejects any
   member outside the documented field set.
2. `evidence` is always present (empty array is valid) and every evidence item carries a required
   positive-integer `revision`; the target's `revision` is optional but positive when present.
3. `verdict.detail` direct members never include the reserved names `target`, `evidence`,
   `judgment_kind`, `outcome`.
4. Every shape violation rejects `INVALID_INPUT` with `details.field` set to the logical field path
   (dotted members, bracketed array indices); no other reject code exists for judgment validation.
5. `validateJudgment` and `judgmentsEqualCanonical` are pure and deterministic: no I/O, no registry,
   no storage, no reasoning, no product policy evaluation, in both language packages.
6. Producer equality is RFC 8785 byte comparison over the complete envelope, including
   `verdict.detail`, `extensions`, and present empty arrays and objects.
7. Open vocabularies (`judgment_kind`, `verdict.outcome`, `JudgmentRef.kind`) round-trip verbatim;
   unknown values are valid without registry updates.
8. `spoke-judgment` never appears as a Connect dispatch row or `op`, and never gates a `tools.*`
   operation; `tools.<ns>.<tool_id>` operations gate on exact membership of their own full string.
9. TypeScript and Rust helpers stay behaviorally identical (field-path strings, reject codes,
   canonical comparison semantics) at package lockstep SemVer (see
   [`spoke-operations.md`](spoke-operations.md)).

## Where it lives

| Artifact | Path |
|---|---|
| Wire schema (SSOT) | `schemas/data/judgment.schema.json` |
| Generated Rust types | `crates/spoke-schemas/src/generated/data/judgment.rs` (module `spoke_schemas::judgment`) |
| Generated TS types | `packages/spoke-schemas/src/generated/data/judgment.ts` (re-exported by `@42ch/spoke-schemas`) |
| Rust helpers + tests | `crates/spoke-operations/src/judgment.rs` — `validate_judgment`, `judgments_equal_canonical` |
| TS helpers | `packages/spoke-operations/src/judgment.ts` — `validateJudgment`, `judgmentsEqualCanonical` |
| TS conformance suite | `packages/spoke-operations/src/judgment.test.ts` |
| Decision record | `.agents/adr/implemented/feature/2026-11-23-spoke-judgment.md` |

## Change obligations

- Change `schemas/data/judgment.schema.json` → regenerate both type packages and update this page
  in the same change.
- Change validator behavior, field-path strings, or refusal vocabulary → update this page and both
  language implementations together; conformance suites cover the same cases on both sides.
- Change capability advertisement or `tools.*` transport semantics → update § Manifest advertisement
  and `tools.*` transport here.
- Add a spec or ADR that references the judgment envelope → link this page as the normative source;
  do not restate the field tables elsewhere.
