# Judgment conformance vectors

Provider-neutral conformance corpus for the optional `spoke-judgment`
capability (`spoke-judgment` wire contract). The corpus exercises the pure
`validateJudgment` / `validate_judgment` wire-shape gate and the RFC 8785
producer-equality rule; `spoke-judgment` defines no Connect operation, so no
case travels a session.

Two runners share the corpus, one per stack:

| Runner | Location | Executes |
|--------|----------|----------|
| Rust | `crates/spoke-operations/tests/judgment_conformance.rs` (`cargo +nightly test -p spoke-operations judgment_conformance`) | `validate_judgment` verdicts and `serde_jcs` canonical bytes for every case |
| TypeScript | `packages/spoke-operations/tests/judgment-conformance.test.ts` (`pnpm -F @42ch/spoke-operations test`) | `validateJudgment` verdicts and `canonicalize` canonical bytes for the same cases |

Both runners assert the same corpus, so a case that diverges between the
stacks fails on at least one side.

## Scenario files

| File | Cases | Purpose |
|------|-------|---------|
| `valid.json` | 8 | Envelopes that must pass the validator: required-only, fully populated, empty `evidence`, namespaced `judgment_kind`, populated `extensions`, non-UTC `created_at` offset, `confidence` at both `[0, 1]` bounds |
| `refusals.json` | 33 | `INVALID_INPUT` matrix: missing/empty required members, closed-envelope violations, non-positive revisions, `verdict` shape and `[0, 1]` confidence bounds, reserved `verdict.detail` members — each with the exact `details.field` path |
| `canonical-parity.json` | 3 | Two independently serialized envelopes per case plus the expected RFC 8785 canonical UTF-8 bytes; covers member-order insensitivity, minimal envelopes, and non-ASCII/`detail`/`extensions` byte-significance |

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

Each runner asserts: both `left` and `right` pass the validator;
`judgmentsEqualCanonical` / `judgments_equal_canonical` reports `true`; and
the producer's own canonicalizer output (`canonicalize` on TS, `serde_jcs`
on Rust) equals `expected_canonical` byte-for-byte — which makes the same
vector a cross-stack byte-parity check.

## Fixture-writing notes

- `created_at` values in `canonical-parity.json` use the `Z` suffix only:
  the generated `Judgment` type re-serializes through
  `chrono::DateTime<Utc>`, so a `+08:00` input would re-emit as UTC and
  break byte-identity. Offset coverage lives in `valid.json`, which asserts
  the validator, not the bytes.
- `verdict.detail: {}` is never used in a canonical-parity case: the typed
  `Verdict.detail` field is `serde(default, skip_serializing_if =
  is_empty)`, so a present-empty `detail` would survive raw `serde_jcs`
  canonicalization but drop out of the typed re-serialization. The
  present-empty rule is asserted for `evidence` and `extensions`, which are
  not skipped.

- `extensions` keys must match `^[a-z][a-z0-9_-]*$` (the generated
  `ExtensionKey` pattern): the corpus uses `acme_audit`-style namespace ids,
  never `acme:audit`. `judgment_kind` has no pattern, so `acme:`-namespaced
  kinds are fine and remain in use.