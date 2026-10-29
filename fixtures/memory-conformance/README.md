# Memory conformance vectors

Provider-neutral conformance corpus for the SPOKE memory protocol (§17 anchors),
plus the runner contract that consumes it. One JSON file per scenario; each case
names the invariant under test, whether the outcome is provider-identical or
algorithm-variable, its inputs, and the expected observable.

Two runners share the corpus:

| Runner | Location | Executes |
|--------|----------|----------|
| Engine | `crates/spoke-memory/tests/conformance.rs` (`cargo +nightly test -p spoke-memory conformance`) | `surface: "engine"` cases — classification, recall ranking/budget, reflection basis digests, transfer chunking/verification/report, and the pure P2 predicates (freeze allowlist, fence validation, JCS digest) |
| Loopback | `crates/spoke-connect/tests/` `memory_conformance` selector | `surface: "wire"` cases — provider-enforced admission, grant coverage and non-amplification, CAS, idempotency, fencing, freeze + drain barrier, cursor invalidation, export retrieval/round-trip, staged import/activation/adoption, profile admission, reflection staleness triggers, classification/recall determinism, and dispatch-layer capability handling |

Provider-enforced behaviour is never asserted from the engine side: an
`engine` case in a scenario without an engine handler fails the run, and the
scenarios whose expectations only a provider can produce (grants, CAS,
idempotency, admission, staleness triggers, export expiry) have no engine
handler at all. Corpus integrity (required scenarios, envelope fields, closed
expect vocabulary, reject-code vocabulary, step/directive shape) is asserted
separately.

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
| `transfer-chunks.json` | 8 | 4 | 4 | 0 |
| `staging-import.json` | 6 | 4 | 2 | 0 |
| `profile-admission.json` | 3 | 0 | 3 | 0 |
| `reflection-staleness.json` | 5 | 3 | 2 | 0 |
| `classification.json` | 8 | 7 | 1 | 8 |
| `recall-budget.json` | 6 | 5 | 1 | 6 |

Totals: 78 cases — 34 engine, 44 wire, 14 variable.

`assets/tw-minimal-export.json` is a sealed `MemoryExportBundle` (real JCS
`digest`) shared by the transfer, staging and import-staleness scenarios. It
carries two subjects, three records, one consumed candidate, the `personality`
profile at revision 3, and a reflection pinned to the two `ctx_tw_customs_gate`
records plus that profile with `stale: false` — that is, an exported reflection
whose basis would still match after import, so only the §7.31 unconditional
revalidation rule can make it stale. It is not a vector file and is excluded
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
    observables must be identical;
  - loopback runner: a `variable` wire case is repeated against unchanged state
    (or, as in the classification case, the same input is submitted twice) and
    the observed values must be identical. No cross-provider ranking or
    disposition is imposed by any vector.

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
  "setup": [{ "op": "memory.capture", "request": {}, "expect": {}, "stage_from": {}, "hold": "pending" }],
  "op": "memory.recall",
  "request": {},
  "stage_from": { "asset": "…", "chunk_limit": 4096 },
  "advance": { "choose": "min", "instants": [{ "from_step": 0, "path": "limits.receipt_retention_seconds", "offset_seconds": 1 }] },
  "release": [1],
  "verify_export": { "receipt_step": 1 },
  "then": [{ "op": "memory.candidates.get", "request": {}, "expect": {} }],
  "requires": ["space configured with required_profile_kinds [personality]"]
}
```

- Engine cases carry `input` (scenario-specific, interpreted by the engine
  handler) and `expect` (scenario-specific observables).
- Wire cases carry `op` + `request` (the step under test), optional `setup`
  steps executed first, optional `then` steps executed after it, then the closed
  `expect` vocabulary:
  - `error_code` — wire code from §14.1, or the dispatch-layer
    `capability_missing` / `op_unsupported`.
  - `details` — object that must be contained in the error envelope `details`
    (for example `{"reason": "space_not_drained"}`).
  - `success` — the op returns a success payload.
  - `receipt_status` — `committed` | `pending` | `failed` on the returned receipt.
  - `same_receipt_as` — index into `setup`; the receipt id must equal that step's.
  - `state` — `{ "path": "dotted.path[0]", "equals": <json> }` equality
    assertions against the op-under-test response.
  - `absent` / `present` — dotted paths that must not / must exist.
  - `sizes` / `minimums` — `{ "path": "...", "equals": <int> }`: the array at
    `path` has exactly / at least that many elements.
  - `contains` / `excludes` — `{ "path": "...", "match": { ... } }`: some / no
    element of the array at `path` contains the partial object.
  - `unchanged` — `{ "path": "...", "from_step": <i>, "from_path": "..." }`: the
    value at `path` equals the value recorded at `from_path` in a prior step —
    used to compare an effect set before and after an operation.
  - `verified` — object of booleans the harness computes itself (currently only
    for `verify_export`: `all_chunk_digests`, `chunk_metadata_consistent`,
    `bundle_digest_matches_receipt`, `reassembled_bundle_verifies`).
  - `uniform` — `{ "path": "...", "field": "...", "min_items": <int> }`: all
    elements of the array at `path` share the same value at `field`.
- A `setup` or `then` step normally assumes success; when it carries `expect`,
  that expectation is asserted instead, which is how negative preconditions (a
  refused review, an absent subject) are expressed inside a case.
- `hold: "pending"` on a setup step — the harness admits the op but keeps its
  async completion uncommitted for the rest of the case, making a pending receipt
  a controlled precondition rather than a race. The step's response must read
  `pending`.
- `release` — indices of held setup steps whose receipts the harness completes
  (and waits to drain) **before** the op under test.
- `advance` — before the op under test, the harness sets its clock to one of the
  `instants` (`json_path(setup[from_step]) + offset_seconds`); with
  `"choose": "min"` it takes the smallest, so a replay instant can be derived to
  lie strictly inside a provider-declared window instead of being hardcoded.
  Negative offsets are allowed (for "one second before the window closes").
- `stage_from` — on an `imports.stage` step: the harness partitions `asset` into
  chunks of at most `chunk_limit` bytes (§6.13 rules) and stages them in ordinal
  order; `request` carries the op-level `conflict_policy` / `adopt_grants`, which
  apply to ordinal 0. The step's response is the final chunk's receipt.
- `verify_export` — on an `exports.get` step: after the first chunk is fetched,
  the harness fetches every remaining ordinal from
  `receipt_step`'s `result.export_id` / `result.chunk_count`, recomputes each
  `chunk_digest` (JCS of the chunk minus `chunk_digest`) and the reassembled
  bundle digest (JCS minus `digest`), and reports the `verified` booleans.
  Fabricated digests therefore fail the case.
- `requires` — declarative environment preconditions (negotiated capability set,
  configured `required_profile_kinds`, seeded grants/profiles/records, a held-pending
  capable harness, the polling obligation that `memory.receipts.get` is retried
  until the receipt is terminal). The harness must satisfy them; they are not
  assertions.

### Authoring directives

- `"$ref:<step>.<dotted.path>"` — late-bound value resolved against a prior step
  response. In `setup` the indices address `setup` steps; inside `then` they
  address the sequence `[op-under-test, then[0], then[1], …]`. Examples:
  `"$ref:0.result.candidate_ids.0"`, `"$ref:0.state_digest"`,
  `"$ref:4.next_cursor"`.
- `{"$repeat": {"unit": "mira ", "times": 210}}` — expands to `unit` repeated
  `times` times. Used only to express oversized-content vectors without a
  kilobyte-long literal; the engine runner expands it before use.

## Coverage of the minimum set

| Plan requirement | Where | Surface |
|------------------|-------|---------|
| Idempotency replay + dedup-tombstone replay (`RECEIPT_EXPIRED`, unchanged effect set) | `idempotency-replay.json` | wire |
| CAS conflict (`REVISION_CONFLICT`) | `cas-conflict.json` | wire |
| Candidate placement inheritance + consumption atomicity (scrub + `content_digest`, no orphan records) | `candidate-consumption.json` | wire (+ engine digest) |
| Grant coverage incl. `shared_access` / `subject_all` + non-amplification rejection | `grant-coverage.json` | wire |
| Space freeze allowlist + drain barrier (`space_not_drained` with a held pending receipt, success after release) | `space-freeze.json` | engine (allowlist) + wire (barrier) |
| Subject/context fencing (`SUBJECT_FENCED`, sibling contexts usable) | `subject-fencing.json` | engine (request rules) + wire (enforcement) |
| Cursor invalidation (`CURSOR_INVALID`) | `cursor-invalidation.json` | wire |
| Chunk round-trip + digest verification | `transfer-chunks.json` | engine (partition/digests) + wire (all-chunk round-trip, expiry) |
| Staged-import verification report + explicit-only `adopt_grants` (verified after activation) | `staging-import.json` | engine (report policy) + wire (staging, activation, adoption) |
| Imported-reflection `stale: true` unconditionally, with a basis that would otherwise match | `reflection-staleness.json` (`import-activation-stales-reflection-unconditionally`) | wire |
| Profile-admission three-state distinguishability | `profile-admission.json` | wire |
| Profile-update ⇒ reflection-stale trigger | `reflection-staleness.json` | wire (+ engine digest policy) |
| Classification/recall determinism per provider | `classification.json` (`classification-stable-within-a-provider`), `recall-budget.json` | wire (+ engine repeat) |
