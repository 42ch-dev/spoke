# Judgment conformance vectors

Provider-neutral conformance corpus for the optional `spoke-judgment`
capability (`spoke-judgment` wire contract). The corpus exercises the pure
`validateJudgment` / `validate_judgment` wire-shape gate and the RFC 8785
producer-equality rule; `spoke-judgment` defines no Connect operation, so no
case travels a session.

Two runners share the corpus, one per stack:

| Runner | Location | Executes |
|--------|----------|----------|
| Rust | `crates/spoke-operations/tests/judgment_conformance.rs` (`cargo +nightly test -p spoke-operations --test judgment_conformance`) | `validate_judgment` verdicts and `serde_jcs` canonical bytes for every case |
| TypeScript | `packages/spoke-operations/tests/judgment-conformance.test.ts` (`pnpm -F @42ch/spoke-operations test`) | `validateJudgment` verdicts and `canonicalize` canonical bytes for the same cases |

Both runners assert the same corpus, so a case that diverges between the
stacks fails on at least one side.

## Scenario files

| File | Cases | Purpose |
|------|-------|---------|
| `valid.json` | 11 | Envelopes that must pass the validator: required-only, fully populated, empty `evidence`, namespaced `judgment_kind`, populated `extensions`, non-UTC `created_at` offset, `confidence` at both `[0, 1]` bounds, raw `minLength: 1` whitespace strings, unconstrained `detail` member values, the `2^53-1` integer-domain ceiling |
| `refusals.json` | 43 | `INVALID_INPUT` matrix: missing/empty required members, closed-envelope violations, non-positive and out-of-domain revisions, `verdict` shape and `[0, 1]` confidence bounds, reserved `verdict.detail` members including fixed-order multi-member probes, offsetless/invalid-day `created_at`, `ExtensionMap` key-pattern and value-kind violations — each with the exact `details.field` path |
| `canonical-parity.json` | 5 | Pairs of serialized envelopes plus the expected RFC 8785 canonical UTF-8 bytes; covers member-order insensitivity, minimal envelopes, non-ASCII/`detail`/`extensions` byte-significance, and wire-presence inequalities (`detail: {}` vs absent, non-UTC `created_at` spellings) |

## Case envelope

```json
{
  "id": "unique-case-id",
  "invariant": "contract section anchor",
  "what": "one line naming the observable",
  "judgment": { "...": "Judgment wire object under test" }
}
```

Refusal cases replace `judgment` assertions with:

```json
"expect": { "field": "evidence[0].revision" }
```

`field` is the exact logical path the runner must see in `details.field` on
the `INVALID_INPUT` reject (dotted members, bracketed array indices). The
human-readable `message` carries no stable machine semantics and is never
asserted.

Canonical-parity cases use:

```json
{ "left": { "...": "Judgment" }, "right": { "...": "same Judgment, different member order" }, "expected_canonical": "<RFC 8785 UTF-8 bytes as a JSON string>" }
```

Equality cases carry `expected_canonical` as one shared string. Inequality
cases set `"equal": false` and carry per-side bytes:

```json
{ "equal": false, "left": { "...": "Judgment A" }, "right": { "...": "Judgment B" }, "expected_canonical": { "left": "<bytes>", "right": "<bytes>" } }
```

Each runner asserts: both `left` and `right` pass the validator;
`judgmentsEqualCanonical` / `judgments_equal_canonical` reports the case's
`equal` verdict; and the producer's own canonicalizer output (`canonicalize`
on TS, `serde_jcs` on Rust) equals `expected_canonical` byte-for-byte — which
makes the same vector a cross-stack byte-parity check.

## Fixture-writing notes

- Canonical equality compares the WIRE JSON. `created_at` values with a
  non-UTC offset and a present-empty `verdict.detail: {}` appear in
  inequality parity cases (`equal: false`, per-side `expected_canonical`),
  because wire presence and exact string spelling are byte-significant.
- `typed_parity: false` on those inequality cases skips the Rust runner's
  typed-serialization byte check: the generated `Judgment` normalizes
  `created_at` through `chrono::DateTime<Utc>` and drops an empty
  `Verdict.detail` map, so typed re-serialization cannot reproduce the wire
  bytes there. The helper comparison itself is asserted on the raw wire
  values.
- `extensions` keys must match `^[a-z][a-z0-9_-]*$` (the shared
  `ExtensionMap` grammar): the corpus uses `acme_audit`-style namespace ids,
  never `acme:audit`. `judgment_kind` has no pattern, so `acme:`-namespaced
  kinds are fine and remain in use.