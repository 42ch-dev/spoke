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
expect vocabulary, reject-code vocabulary, step/directive shape, reference
namespaces) is asserted separately.

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
profile at revision 3, and one reflection exported with `stale: false`. Because
§7.31 assigns fresh target revisions and remaps record ids at activation, the
imported reflection's pinned basis is a *new* basis on the target — the exported
currency flag therefore says nothing about the target, and only the
unconditional revalidation rule can make the activated reflection read
`stale: true`. It is not a vector file and is excluded from corpus loading (only
top-level `*.json` files are vectors).

## Cross-step response namespaces

A case's responses live in three namespaces, and every cross-step reference is
namespaced so it can never be ambiguous:

| Reference form | Means |
|----------------|-------|
| `$ref:setup.<n>.<path>` | value from `setup[n]`'s response |
| `$ref:op.<path>` | value from the op-under-test response |
| `$ref:then.<n>.<path>` | value from `then[n]`'s response |
| `setup.<n>` / `op` / `then.<n>` (bare step reference) | the response itself, used by `same_receipt_as`, `unchanged.from`, `advance.instants[].from`, `release`, `verify_export.receipt` |

`$ref:` values must carry a path; bare step references must not. The integrity
test rejects un-namespaced references anywhere in a request payload or directive.

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
  - loopback runner: a `variable` wire case is repeated against equivalent state
    and the observed values must be identical. The classification case captures
    the content, dismisses the resulting candidate to return the subject to an
    equivalent pending-free state, captures the identical content again, and
    compares the two dispositions. If the second capture produces no candidate
    (dedup is permitted by §7.11) the case is recorded **inconclusive** via
    `inconclusive_when` — never a failure and never a synthesized disposition.
    No cross-provider ranking or disposition is imposed by any vector.

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
  "advance": { "choose": "min", "instants": [{ "from": "setup.0", "path": "limits.receipt_retention_seconds", "offset_seconds": 1 }] },
  "release": ["setup.1"],
  "verify_export": { "receipt": "setup.1" },
  "inconclusive_when": { "path": "then.3.result.candidate_ids", "isEmpty": true },
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
  - `same_receipt_as` — bare step reference; the receipt id must equal that step's.
  - `state` — `{ "path": "dotted.path[0]", "equals": <json> }` equality
    assertions against the op-under-test response.
  - `absent` / `present` — dotted paths that must not / must exist.
  - `sizes` / `minimums` — array-length assertions at `path`: exactly /
    at least that many elements.
  - `atLeast` — `{ "path": "...", "equals": <number> }` **numeric** lower bound
    on a response value (for example `spaces.0.pending_receipts >= 1`).
  - `contains` / `excludes` — `{ "path": "...", "match": { ... } }`: some / no
    element of the array at `path` contains the partial object.
  - `unchanged` — `{ "path": "...", "from": "<step reference>", "from_path": "..." }`:
    the value at `path` equals the value recorded at `from_path` in that earlier
    response — used to compare an effect set before and after an operation.
  - `verified` — object of booleans the harness computes itself (currently only
    for `verify_export`: `all_chunk_digests`, `chunk_metadata_consistent`,
    `bundle_digest_matches_receipt`, `reassembled_bundle_verifies`).
  - `uniform` — `{ "path": "...", "field": "...", "min_items": <int> }`: all
    elements of the array at `path` share the same value at `field`.
- A `setup` or `then` step normally assumes success; when it carries `expect`,
  that expectation is asserted instead, which is how negative preconditions (a
  refused review, an absent subject, an empty reflection list) are expressed
  inside a case.
- `hold: "pending"` on a setup step — the harness admits the op but keeps its
  async completion uncommitted for the rest of the case, making a pending receipt
  a controlled precondition rather than a race. The step's response must read
  `pending`.
- `release` — bare step references of held setup steps whose receipts the harness
  completes (and waits to drain) **before** the op under test.
- `advance` — before the op under test, the harness sets its clock to one of the
  `instants` (`path` of the `from` step's response `+ offset_seconds`); with
  `"choose": "min"` it takes the smallest, so a replay instant can be derived to
  lie strictly inside a provider-declared window instead of being hardcoded.
  Negative offsets are allowed (for "one second before the window closes").
- `stage_from` — on an `imports.stage` step: the harness partitions `asset` into
  chunks of at most `chunk_limit` bytes (§6.13 rules) and stages them in ordinal
  order; `request` carries the op-level `conflict_policy` / `adopt_grants`, which
  apply to ordinal 0. The step's response is the final chunk's receipt.
- `verify_export` — on an `exports.get` step: after the first chunk is fetched,
  the harness fetches every remaining ordinal from the referenced receipt's
  `result.export_id` / `result.chunk_count`, recomputes each `chunk_digest` (JCS
  of the chunk minus `chunk_digest`) and the reassembled bundle digest (JCS minus
  `digest`), and reports the `verified` booleans. Fabricated digests therefore
  fail the case.
- `inconclusive_when` — `{ "path": "...", "isEmpty": true }`: if the named
  response value is an empty array, the case is recorded `inconclusive` and its
  remaining steps/assertions are not evaluated. Used only where the protocol
  explicitly permits an empty outcome.
- `requires` — declarative environment preconditions (negotiated capability set,
  configured `required_profile_kinds`, seeded grants/profiles/records, a
  held-pending capable harness, the polling obligation that `memory.receipts.get`
  is retried until the receipt is terminal). The harness must satisfy them; they
  are not assertions.

### Authoring directives

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
| Staged-import verification report + explicit-only `adopt_grants` (verified after activation, alongside the staging principal's own grant) | `staging-import.json` | engine (report policy) + wire (staging, activation, adoption) |
| Imported-reflection `stale: true` unconditionally | `reflection-staleness.json` (`import-activation-stales-reflection-unconditionally`) | wire |
| Profile-admission three-state distinguishability | `profile-admission.json` | wire |
| Profile-update ⇒ reflection-stale trigger | `reflection-staleness.json` | wire (+ engine digest policy) |
| Classification/recall determinism per provider | `classification.json` (`classification-stable-within-a-provider`), `recall-budget.json` | wire (+ engine repeat) |
