---
module: protocol
date: 2026-11-24
last_updated: 2026-11-24
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["closing an open wire numeric or timestamp domain", "adding a strict decoder to generated Rust types", "signing envelopes that may carry a revision endpoint", "aligning TypeScript and Rust wire accept-sets", "reviewing boundary fixtures"]
tags: [wire-boundary, schema-version, timestamp, revision-domain, typify, serde-jcs, parsed-value]
---

# Wire scalar boundary closure across both stacks

## Context

`SchemaVersion`, `Timestamp`, and revision-typed integers (`revision`, `deleted`, `floor_revision`, `keep`, `expected_base_revision`, `source_revision`) are shared by every family. When the published schema leaves their domain open, TypeScript and Rust each implement their own accept-set: `typify` enforces no integer bounds, chrono's relaxed parser accepts spellings the TypeScript gate refuses, and JavaScript cannot represent integers above `2^53` exactly. The result is a payload one stack accepts and the other rejects, silently, at any read path.

## Guidance

**State one closed domain in the schema and the specs.** `SchemaVersion` is `[1, 2^53-1]`. `Timestamp` is RFC 3339 `date-time` with a mandatory explicit offset: lower-case `t`/`z`, leap second `:60` on any date, `-00:00`, offsets to `±23:59`, year `0000`, and arbitrary fractional digits are admitted; leading/trailing whitespace, a space separator, a naive value, a `UTC` suffix, and uncompensated fields are refused. Revision-typed members default to the inclusive `[0, 2^53]`; a member's published contract may narrow (Judgment `evidence[].revision` and `target.revision` are `[1, 2^53-1]`). `sequence` is not in the revision family and keeps `2^53-1`.

**The bound is asserted on the parsed integer value.** `2^53` is in domain. A JavaScript host cannot distinguish a wire literal above `2^53` from `2^53` after parsing, so it refuses the first representable value above the bound; exact refusal of an unrepresentable literal is a parser-level property. A Rust decoder must keep exact integer tokens exact (refuse `2^53+1`) and accept an in-domain integral-float spelling (`9007199254740992.0`) by its parsed value.

**Generated Rust needs a post-generation step.** `common.schema.json` `pattern` is not enforced when `format: date-time` yields `chrono::DateTime<Utc>`, and typify enforces no minimum/maximum. The codegen pipeline therefore rewrites generated output: bounded integer fields keep their public type (`NonZeroU64`/`u64`) and gain `deserialize_with` bounds; direct `DateTime<Utc>` fields gain strict `deserialize_with`; `Timestamp`/`StoredTimestamp` transparent newtypes share one strict parser (ASCII syntax gate, then `DateTime::parse_from_rfc3339` — chrono's `parse_from_rfc3339` alone still accepts a space separator). Detection and rewrite must share one spelling-independent matcher, and generation must fail when any copy is not rewritten. Adding `minimum: 0` + `maximum` to a revision member can flip the generated type from `u64` to `i64`; mark such members so typify keeps the intended type, or enforce the bound in the validation layer.

**Signed canonicalization follows owning paths, not member names.** RFC 8785 canonicalization rejects an exact integer above `2^53-1`, so a legal revision endpoint cannot cross a signed envelope without an endpoint exception. Grant that exception only at the exact op-and-direction paths and the response variant that own the member (success vs error, plus the error discriminator, e.g. `REVISION_CONFLICT` for `details.adopted_revision`), carrying only prefixes that already matched. A name-based rewrite lets an opaque product bag borrow another member's exception and makes two keys in the same open object behave differently. Opaque bags (`extensions`, `modules`, `state`, `computable`, `Finding.text_position`, `Relation.metadata`) are never classified by key name, and non-owning integral floats at `±2^53` are refused as well.

**Every consumer surface shares the gate.** Ordinary success payloads, dial session snapshots, operation validators (upsert/promote nested anchors), prune/history request and response slots, and both signing directions apply the same predicate. A shape-driven scalar walk over schema-owned fields keeps opaque bags untouched; adding a response variant means adding its shape row.

## Verification

- Schema-side: AJV registers the strict timestamp gate as its `date-time` format; stock `ajv-formats` disagrees in both directions (accepts a space separator, restricts `:60` to the UTC day boundary).
- Boundary fixtures carry accepted and refused exemplars for all three domains and are consumed by the surface that produces the verdict (AJV for schema-side, real predicates for runtime-side), not dead data.
- Targeted proofs: generated wrapped and inlined `SchemaVersion` refuse `2^53`; revision decoders accept `2^53`, refuse exact `2^53+1`, accept `...992.0`; direct timestamp fields refuse the refused spellings; a signed endpoint payload, an adoption conflict receipt, and a prune response with `keep = 2^53` sign and verify; an opaque `2^53` under two different keys refuses identically.
- `pnpm run verify-codegen` passes and a second codegen run is byte-identical.
