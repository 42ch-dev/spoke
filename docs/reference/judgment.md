---
title: Judgment reference
---

# Judgment reference (`spoke-judgment`)

`spoke-judgment` is the optional data/conformance capability for a declared assessment about a target: a `Judgment` envelope backed by evidence that pins exact revisions. The capability defines the envelope shape, a pure validation helper, and an RFC 8785 canonical comparison helper on both language stacks.

## Capability

| Aspect | Contract |
|--------|----------|
| Capability string | `spoke-judgment` — an optional member of `HostCapabilityManifest.capabilities` |
| Manifest declaration | The host validates or produces the `Judgment` data envelope and conforms to its pure-validation and canonical-comparison contract |
| Dependency | No co-capability; independent of `spoke-knowledge` and every other family |
| Negotiation | Manifest-only: `spoke-judgment` has no Connect dispatch row and no operation of its own |
| Connect surface | None; a `tools.<ns>.<tool_id>` operation carrying a `Judgment` stays gated on its own full operation string |

The declaration signals data and validator conformance. Validation and comparison are pure: no I/O, no registry, no storage, no reasoning, and no product policy evaluation.

## Data contract

### `Judgment`

A closed object (`additionalProperties: false`):

| Field | Presence | Type | Contract |
|-------|----------|------|----------|
| `schema_version` | required | shared `SchemaVersion` | Integer ≥ 1 inside the interoperable integer domain; version of the `Judgment` wire schema |
| `judgment_id` | required | non-empty string | Opaque stable judgment id |
| `judgment_kind` | required | non-empty open string | Assessment-kind vocabulary |
| `target` | required | `JudgmentRef` | The thing assessed; its `revision` is optional and positive when present |
| `evidence` | required | `JudgmentEvidenceRef[]` | Present even when empty; every item pins a required positive `revision` |
| `verdict` | required | `Verdict` | Closed core plus open product detail |
| `basis` | optional | string | Product-supplied explanation or rationale; it does not replace `evidence` |
| `created_at` | optional | shared `Timestamp` | RFC 3339 date-time with an explicit UTC offset |
| `extensions` | required | shared `ExtensionMap` | Product-namespace extensions; `{}` when empty |

Required members are `schema_version`, `judgment_id`, `judgment_kind`, `target`, `evidence`, `verdict`, and `extensions`; only `basis` and `created_at` are optional.

### `JudgmentRef` and `JudgmentEvidenceRef`

`JudgmentRef` is the lightweight reference used by `Judgment.target` and every `Judgment.evidence[]` item. It is a closed object:

| Field | Presence | Type | Contract |
|-------|----------|------|----------|
| `kind` | required | non-empty open string | Identifies the referenced thing's kind without a protocol registry |
| `id` | required | non-empty string | Opaque stable id within the applicable kind and product scope |
| `namespace` | optional | non-empty string | Product scope for the reference; not a required component of the referenced thing's identity |
| `revision` | conditional | positive integer (≥ 1) | Pins the referenced revision when revision pinning applies; the pin stays exact on both stacks |

Narrowing rule: `Judgment.target.revision` is optional and positive when present. Every `Judgment.evidence[]` item is a `JudgmentEvidenceRef` — a `JudgmentRef` whose `revision` is required and positive. Evidence therefore always carries the exact revisions the judgment relies on, while the target may be assessed unpinned as a product decision.

`JudgmentRef` carries only kind, id, optional product scope, and optional revision; it does not reuse `CognitionReference` or `CognitionTarget`, which retain their cognition-scoped carrier semantics.

### `Verdict`

A closed object:

| Field | Presence | Type | Contract |
|-------|----------|------|----------|
| `outcome` | required | non-empty open string | Portable core outcome; products define the values and readers interpret them |
| `confidence` | optional | finite number in `[0, 1]` | Core confidence value; non-finite numbers refuse `INVALID_INPUT` |
| `detail` | optional | object with unconstrained JSON member values | Product-specific assessment content, opaque to the envelope validator and included in canonicalization |

`verdict.detail` accepts objects, scalars, arrays, and `null` as member values. Its direct members must not duplicate protocol-level envelope locations: the reserved names are `target`, `evidence`, `judgment_kind`, and `outcome`, so `verdict.detail.target` and `verdict.detail.evidence` are invalid. When several reserved members are present, the refusal reports the first in the fixed probe order `target`, `evidence`, `judgment_kind`, `outcome`, and `details.field` carries `verdict.detail.<name>`. Product-specific members inside `detail`, including nested objects, keep product-defined shapes.

### Open vocabularies

`judgment_kind` is an open string. Core values, documented and not enforced by an `enum`:

| Value | Meaning |
|-------|---------|
| `consistency` | Assesses consistency |
| `support` | Assesses evidentiary support |
| `acceptance` | Assesses acceptance |
| `review` | Records a review outcome |

Products add namespaced kinds such as `my-product:belief-assessment`. Unknown non-empty values are valid and need no registry update. `verdict.outcome` and `JudgmentRef.kind` are open vocabularies under the same rule.

### Wire-wide rules

- **Non-empty strings.** Non-empty means JSON Schema `minLength: 1` measured on the raw wire string; a whitespace-only string is a valid non-empty wire value for every open string in this contract.
- **Interoperable integer domain.** `schema_version`, `target.revision`, and every `evidence[].revision` take non-negative integers in `[0, 2^53-1]`, the JSON numbers that are both u64-safe and IEEE-754 binary64-safe; the schema pins this with `maximum: 9007199254740991`. Integral spellings such as `1.0` denote the integer they spell. A non-integral or out-of-domain value returns `INVALID_INPUT` at that member's field path.
- **`created_at` offset and raw match.** `created_at` is an RFC 3339 `date-time` with an explicit UTC offset (`Z` or `±hh:mm`); an offsetless timestamp returns `INVALID_INPUT` at `created_at`. The grammar matches the raw wire string exactly as received: leading or trailing whitespace returns `INVALID_INPUT`, and any non-ASCII character — including `U+2212` MINUS SIGN in an offset — returns `INVALID_INPUT`, while ASCII `T`/`t` separators and `Z`/`z` suffixes stay accepted per RFC 3339. Leap-second `23:59:60` spellings remain accepted and invalid calendar days remain refused.
- **Extension maps.** `extensions` follows the shared `ExtensionMap` grammar: namespace keys match `^[a-z][a-z0-9_-]*$` and each value is an object. A non-conforming entry returns `INVALID_INPUT` with `details.field` at the offending member; several failing entries report the first offender in lexicographic key order over Unicode code points, deterministic across stacks. The check reads namespace key and value kind only; map children stay opaque.
- **Wire member enumerability.** Only an object's own enumerable properties are wire members. A non-enumerable own property never reaches the wire, and prototype members are excluded from every member enumeration this contract names, including the reserved-name probe over `verdict.detail`.

### `validateJudgment` / `validate_judgment`

TypeScript exposes `validateJudgment(judgment: Judgment): SpokeResult<void>`; Rust exposes the behaviorally identical `validate_judgment(judgment: &Value) -> SpokeResult<()>`. The helper is deterministic and performs no I/O, and it validates the undecoded wire value in both languages.

`validateJudgment` checks:

1. Presence and wire shape of every required field, including the closed `Judgment`, `JudgmentRef`, and `verdict` objects and the required `extensions` map.
2. `schema_version` as an integer ≥ 1 inside the interoperable integer domain; `judgment_id` and `judgment_kind` as non-empty raw strings.
3. `target` and `evidence` reference shape: non-empty raw `kind` and `id`, non-empty raw `namespace` when present, and positive-integer revisions inside the interoperable integer domain where applicable.
4. Presence of `evidence`, including the valid empty-array case, and a required positive integer `revision` inside the interoperable integer domain on every evidence item.
5. `verdict.outcome` as a non-empty raw string, optional `verdict.confidence` as a finite number in `[0, 1]`, and optional `verdict.detail` as an object without the reserved direct members, probed in the fixed order.
6. `extensions` entries against the shared `ExtensionMap` grammar, reporting the first offender in lexicographic key order.
7. Optional `basis`, reference revisions, and optional `created_at` against their declared wire types and constraints.

The helper does not determine verdict truth, evidence existence or content correctness, authorization, business thresholds, or product-specific `detail` shape; its reserved-member check protects protocol field locations only. Decoding into a generated typed carrier is representation only: a decoded `Judgment` is valid after passing the typed validation gate (Rust `validate_judgment_typed`, its TypeScript counterpart, or this wire-form gate), which is where the wire-integer domain and every shape rule are enforced.

### Canonical comparison

Canonical equality is wire-form equality. `judgmentsEqualCanonical(left, right)` (Rust `judgments_equal_canonical`) compares the RFC 8785 canonical serialization of the wire JSON — the JSON the producer emitted or the validator accepted — rather than the canonical serialization of any typed decode, and returns `SpokeResult<bool>`. It rejects `INVALID_INPUT` when either value cannot be canonicalized.

- Two independent producers of the same `Judgment` emit byte-identical bytes under RFC 8785 canonical JSON.
- Wire presence is comparison-significant: a present-and-empty `verdict.detail: {}` is a distinct member and compares unequal to an absent `detail`.
- `created_at` compares by its exact wire spelling, so `2026-06-30T23:59:59+08:00` and `2026-06-30T15:59:59Z` compare unequal.
- Canonicalization covers the complete envelope, including `verdict.detail`, `extensions`, present empty arrays and objects, and every optional member that is present; `detail` stays opaque to the envelope and in the comparison as produced.
- On a stack that accepts producer objects rather than parsed wire JSON, the comparison first re-emits each value through the engine's own JSON wire emission; the emitted members — own enumerable only, `toJSON` resolved with the member's key, array holes as `null` — are exactly what a wire reader parses.

## Refusal vocabulary

Every shape violation returns `INVALID_INPUT` with `details.field` containing the exact logical field path; `field` is the only protocol-required member of `details` for these refusals, and the human-readable `message` carries no stable machine semantics. Judgment validation introduces no new reject codes.

| Condition | `details.field` |
|-----------|-----------------|
| Missing or extra member, wrong type, or failed constraint | The logical path of the offending member: dotted members and bracketed array indices, for example `judgment_kind`, `target.revision`, `evidence[0].revision`, `verdict.outcome`, `verdict.confidence` |
| Reserved direct member in `verdict.detail` | `verdict.detail.<name>`, where `<name>` is the first present reserved member in the fixed probe order `target`, `evidence`, `judgment_kind`, `outcome` |
| Non-conforming `extensions` entry | `extensions.<namespace>`, the first offender in lexicographic key order |
| Malformed `created_at` | `created_at` |

The shared `SpokeResult` / `SpokeRejectCode` surface carries the reject envelope; `INVALID_INPUT` is its shared shape-violation code.

## Usage constraints

- **Data and validator conformance only.** The capability defines no engine, reasoner, storage behavior, or Connect operation. How a judgment is produced, stored, or acted on stays product-owned.
- **Existing `tools.*` transport.** A tool whose payload carries `Judgment` is declared by its `ToolDescriptor` in `HostCapabilityManifest.tools[]` and invoked through the existing opaque `{ "arguments": ... }` / `{ "result": ... }` tool payloads. Each `tools.<ns>.<tool_id>` operation is gated on exact membership of that full operation string in `negotiated_capabilities` — for example `tools.mindcore.deliberate` is gated on `tools.mindcore.deliberate`. `spoke-judgment` is neither a substitute nor an additional dispatch gate for any `tools.*` operation.
- **Portable envelope, product content.** `basis` is a product-supplied rationale and does not replace `evidence`. Product assessment content beyond the portable core belongs in `verdict.detail` (never under the reserved direct member names) or `extensions.<ns>`.
- **Evidence pins revisions.** `evidence` is always present, the empty array is valid, and every evidence item carries a required positive-integer `revision`; the target's `revision` is optional but positive when present. Every integer member stays inside the interoperable `[0, 2^53-1]` domain.
- **Open vocabularies round-trip.** `judgment_kind`, `verdict.outcome`, and `JudgmentRef.kind` round-trip verbatim; unknown values are valid without registry updates.
- **Cross-stack identity.** TypeScript and Rust helpers stay behaviorally identical — field-path strings, reject codes, and canonical comparison semantics — at package lockstep SemVer.

## Related

- [Protocol reference](/reference/protocol) — capability flags and the schema inventory.
- [Data model reference](/reference/data-model) — `HostCapabilityManifest`, `ToolDescriptor`, and `ExtensionMap`.
- [Ops wire reference](/reference/ops) — the shared `SpokeResult` / reject envelope.
- [Connect reference](/reference/connect) — tool transport and exact-string dispatch.
