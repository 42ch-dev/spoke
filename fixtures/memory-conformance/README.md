# Memory conformance vectors

Provider-neutral conformance corpus for the SPOKE memory protocol (§17 anchors),
plus the runner contract that consumes it. One JSON file per scenario; each case
names the invariant under test, whether the outcome is provider-identical or
algorithm-variable, its inputs, and the expected observable.

Two runners share the corpus:

| Runner | Location | Executes |
|--------|----------|----------|
| Engine | `crates/spoke-memory/tests/conformance.rs` (`cargo +nightly test -p spoke-memory conformance`) | `surface: "engine"` cases — classification, recall ranking/budget, reflection basis digests, transfer chunking/verification/report, and the pure P2 predicates (freeze allowlist, fence validation, JCS digest) |
| Loopback | `crates/spoke-connect/tests/` `memory_conformance` selector | `surface: "wire"` cases — provider-enforced admission, grant coverage and non-amplification, CAS, idempotency, fencing, freeze + drain barrier, cursor invalidation, export chunk retrieval, staged import/adoption, profile admission, reflection staleness triggers, and dispatch-layer capability handling |

Provider-enforced behaviour is never asserted from the engine side: an
`engine` case in a scenario without an engine handler fails the run, and the
scenarios whose expectations only a provider can produce (grants, CAS,
idempotency, admission, staleness triggers, export expiry) have no engine
handler at all. Corpus integrity (required scenarios, envelope fields, closed
expect vocabulary, reject-code vocabulary, step shape) is asserted separately.

## Scenario files

| File | Cases | engine | wire | variable |
|------|-------|--------|------|----------|
| `idempotency-replay.json` | 4 | 0 | 4 | 0 |
| `cas-conflict.json` | 3 | 0 | 3 | 0 |
| `candidate-consumption.json` | 4 | 1 | 3 | 0 |
| `grant-coverage.json` | 12 | 0 | 12 | 0 |
| `space-freeze.json` | 12 | 8 | 4 | 0 |
| `subject-fencing.json` | 5 | 2 | 3 | 0 |
| `cursor-invalidation.json` | 2 | 0 | 2 | 0 |
| `transfer-chunks.json` | 7 | 4 | 3 | 0 |
| `staging-import.json` | 6 | 4 | 2 | 0 |
| `profile-admission.json` | 3 | 0 | 3 | 0 |
| `reflection-staleness.json` | 5 | 3 | 2 | 0 |
| `classification.json` | 7 | 7 | 0 | 7 |
| `recall-budget.json` | 6 | 5 | 1 | 6 |

Totals: 76 cases — 34 engine, 42 wire, 13 variable.

`assets/tw-minimal-export.json` is a sealed `MemoryExportBundle` (real JCS
`digest`; it carries a fresh reflection, `stale: false`) shared by the transfer,
staging and import-staleness scenarios; it is not a vector file and is excluded
from corpus loading (only top-level `*.json` files are vectors).

## Identical-outcome vs algorithm-variable (§17)

`outcome` marks what a second provider must reproduce:

- `identical` — admission, CAS, idempotency, isolation, export-digest surfaces:
  any conforming provider must produce the same observable. The engine runner
  asserts the reference engine's exact value where an engine surface exists.
- `variable` — classification dispositions and recall order: providers may
  differ, but each must be deterministic for the same state, request, and
  `engine_policy_version`. Determinism is asserted **per implementation**, never
  across implementations:
  - engine runner: every `variable` case is executed twice and the two
    observables must be identical (a policy that merely matched the reference
    output once would be caught otherwise);
  - loopback runner: a `variable` wire case is repeated twice against unchanged
    state and the responses must be identical. No cross-provider ranking or
    classification is imposed by any vector.

## Case envelope

```json
{
  "id": "unique-case-id",
  "invariant": "MEM-R008",
  "surface": "engine | wire",
  "outcome": "identical | variable",
  "what": "one line naming the observable",
  "input": { "...": "engine cases only" },
  "expect": { "...": "see below" },
  "setup": [{ "op": "memory.capture", "request": {}, "expect": {}, "stage_from": {} }],
  "op": "memory.recall",
  "request": {},
  "stage_from": { "asset": "…", "chunk_limit": 4096 },
  "advance": { "from_step": 0, "path": "limits.receipt_retention_seconds", "offset_seconds": 1 },
  "then": [{ "op": "memory.candidates.get", "request": {}, "expect": {} }],
  "requires": ["space configured with required_profile_kinds [personality]"]
}
```

- Engine cases carry `input` (scenario-specific, interpreted by the engine
  handler) and `expect` (scenario-specific observables).
- Wire cases carry `op` + `request` (the step under test), optional `setup`
  steps executed first, and optional `then` steps executed after it, then the
  closed `expect` vocabulary:
  - `error_code` — wire code from §14.1, or the dispatch-layer
    `capability_missing` / `op_unsupported`.
  - `details` — object that must be contained in the error envelope `details`
    (for example `{"reason": "space_not_drained"}`).
  - `success` — the op returns a success payload.
  - `receipt_status` — `committed` | `pending` | `failed` on the returned receipt.
  - `same_receipt_as` — index into `setup`; the receipt id must equal that step's.
  - `state` — `{ "path": "dotted.path[0]", "equals": <json> }` assertions
    evaluated against the op-under-test response.
  - `absent` / `present` — dotted paths that must not / must exist.
  - `sizes` — `{ "path": "...", "equals": <int> }` array-length assertions.
- A `setup` or `then` step normally assumes success; when it carries `expect`,
  that expectation is asserted instead, which is how negative preconditions (a
  refused review, a conflicting freeze) are expressed inside a case.
- `advance` — before the op under test, the harness advances its clock by
  `json_path(setup[from_step]) + offset_seconds`; `path` points at a
  provider-declared limit in the `memory.describe` result, so retention windows
  are derived, never hardcoded.
- `stage_from` — on an `imports.stage` step: the harness partitions `asset` into
  chunks of at most `chunk_limit` bytes (§6.13 rules) and stages them in ordinal
  order; `request` carries the op-level `conflict_policy` / `adopt_grants`, which
  apply to ordinal 0. The step's response is the final chunk's receipt.
- `requires` — declarative environment preconditions (negotiated capability set,
  configured `required_profile_kinds`, seeded grants/profiles/records, and the
  polling obligation that `memory.receipts.get` is retried until the receipt is
  terminal). The harness must satisfy them; they are not assertions.

### Authoring directives

- `"$ref:<step>.<dotted.path>"` — late-bound value resolved against a prior step
  response, e.g. `"$ref:0.result.candidate_ids.0"`, `"$ref:0.state_digest"` or
  `"$ref:4.next_cursor"`. Applied to `request` payloads of the op under test and
  of `setup` / `then` steps.
- `{"$repeat": {"unit": "mira ", "times": 210}}` — expands to `unit` repeated
  `times` times. Used only to express oversized-content vectors without a
  kilobyte-long literal; the engine runner expands it before use.

## Coverage of the minimum set

| Plan requirement | Where | Surface |
|------------------|-------|---------|
| Idempotency replay + dedup-tombstone replay (`RECEIPT_EXPIRED`, no second effect) | `idempotency-replay.json` | wire |
| CAS conflict (`REVISION_CONFLICT`) | `cas-conflict.json` | wire |
| Candidate placement inheritance + consumption atomicity (scrub + `content_digest`, no orphan records) | `candidate-consumption.json` | wire (+ engine digest) |
| Grant coverage incl. `shared_access` / `subject_all` + non-amplification rejection | `grant-coverage.json` | wire |
| Space freeze allowlist + drain barrier (`space_not_drained`, success after drain) | `space-freeze.json` | engine (allowlist) + wire (barrier) |
| Subject/context fencing (`SUBJECT_FENCED`, sibling contexts usable) | `subject-fencing.json` | engine (request rules) + wire (enforcement) |
| Cursor invalidation (`CURSOR_INVALID`) | `cursor-invalidation.json` | wire |
| Chunk round-trip + digest verification | `transfer-chunks.json` | engine (partition/digests) + wire (chunk retrieval, expiry) |
| Staged-import verification report + explicit-only `adopt_grants` | `staging-import.json` | engine (report policy) + wire (staging, adoption) |
| Imported-reflection `stale: true` unconditionally | `reflection-staleness.json` (`import-activation-stales-reflection-unconditionally`) | wire |
| Profile-admission three-state distinguishability | `profile-admission.json` | wire |
| Profile-update ⇒ reflection-stale trigger | `reflection-staleness.json` | wire (+ engine digest policy) |
