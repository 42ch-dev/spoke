# Memory conformance vectors

Provider-neutral conformance corpus for the SPOKE memory protocol (§17 anchors),
plus the runner contract that consumes it. One JSON file per scenario; each case
names the invariant under test, whether the outcome is provider-identical or
algorithm-variable, its inputs, and the expected observable.

Two runners share the corpus:

| Runner | Location | Executes |
|--------|----------|----------|
| Engine | `crates/spoke-memory/tests/conformance.rs` (`cargo +nightly test -p spoke-memory conformance`) | `surface: "engine"` cases — classification, recall ranking/budget, reflection digests/staleness, transfer chunking/verification/report, and the P2 pure helpers (grant coverage, freeze allowlist, fence validation, JCS digest) |
| Loopback | `crates/spoke-connect/tests/` `memory_conformance` selector | `surface: "wire"` cases — provider-enforced admission, CAS, idempotency, fencing, freeze, cursor invalidation, staged-import adoption, and dispatch-layer capability handling |

The engine runner never fakes a wire case: an `engine` case in a scenario without
an engine handler fails the run, and corpus integrity (required scenarios,
envelope fields, closed expect vocabulary, reject-code vocabulary) is asserted
separately. Wire cases are structurally validated here and executed over the
wire.

## Scenario files

| File | Invariants | Cases (engine/wire) |
|------|-----------|---------------------|
| `idempotency-replay.json` | MEM-R008, MEM-R010, MEM-R021, MEM-R022, §6.10, §14.1 | 0/4 |
| `cas-conflict.json` | MEM-R007, §14.1 | 0/3 |
| `candidate-consumption.json` | §6.7, MEM-R009, MEM-R025, §7.15 | 1/3 |
| `grant-coverage.json` | §6.5, MEM-R006, MEM-R017, §7.9 | 12/0 |
| `space-freeze.json` | §8.2, MEM-R023, MEM-R011, §12.1 | 8/4 |
| `subject-fencing.json` | §8.3, MEM-R024, §7.7 | 2/3 |
| `cursor-invalidation.json` | MEM-R026, §7.0.7 | 0/2 |
| `transfer-chunks.json` | §6.13, §6.11, MEM-R015 | 4/0 |
| `staging-import.json` | §7.31, §12.3, MEM-R025 | 4/0 |
| `profile-admission.json` | §4.1, MEM-R020, prototype r3 addendum | 0/3 |
| `reflection-staleness.json` | MEM-R013, §6.9, §7.25, prototype r3 addendum | 5/0 |
| `classification.json` | §6.7, §10 | 7/0 |
| `recall-budget.json` | §9, §7.22, MEM-R011 | 5/0 |

`assets/tw-minimal-export.json` is a sealed `MemoryExportBundle` (real JCS
`digest`) shared by the transfer and staging scenarios; it is not a vector file
and is excluded from corpus loading (only top-level `*.json` files are vectors).

## Identical-outcome vs algorithm-variable (§17)

`outcome` marks what a second provider must reproduce:

- `identical` — admission, CAS, idempotency, isolation, export-digest surfaces:
  any conforming provider must produce the same observable. The engine runner
  still asserts its exact value, because the reference engine owns that policy.
- `variable` — classification dispositions and recall order: providers may
  differ, but each must be deterministic for the same state, request, and
  `engine_policy_version`. The engine runner asserts the reference engine's
  deterministic output; the loopback runner asserts determinism, never a
  cross-provider ranking in these cases.

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
  "setup": [{ "op": "memory.capture", "request": {}, "expect": {} }],
  "op": "memory.recall",
  "request": {},
  "advance_seconds": 604800,
  "requires": ["space configured with required_profile_kinds [personality]"]
}
```

- Engine cases carry `input` (scenario-specific, interpreted by the engine
  handler) and `expect` (scenario-specific observables).
- Wire cases carry `op` + `request` (the step under test), optional `setup`
  steps executed first, and a closed `expect` vocabulary:
  - `error_code` — wire code from §14.1, or the dispatch-layer
    `capability_missing` / `op_unsupported`.
  - `success` — the op returns a success payload.
  - `receipt_status` — `committed` | `pending` | `failed` on the returned receipt.
  - `same_receipt_as` — index into `setup`; the receipt id must equal that step's.
  - `state` — list of `{ "path": "dotted.path[0]", "equals": <json> }` assertions
    evaluated against the op-under-test response.
  - `absent` — list of dotted paths that must not exist in that response.
- A `setup` step normally assumes success; when it carries `expect`, that
  expectation is asserted instead, which is how negative preconditions (for
  example a refused review) are expressed inside an atomicity case.
- `advance_seconds` — the harness advances its clock by this many seconds before
  the op under test (retention/expiry vectors).
- `requires` — declarative environment preconditions (negotiated capability set,
  configured `required_profile_kinds`, seeded profile state). The harness must
  satisfy them; they are not assertions.

### Authoring directives

- `"$ref:<step>.<dotted.path>"` — late-bound value resolved against a prior step
  response, e.g. `"$ref:0.result.candidate_ids.0"` or `"$ref:4.next_cursor"`.
  Applied to `request` payloads of the op under test and of setup steps.
- `{"$repeat": {"unit": "mira ", "times": 210}}` — expands to `unit` repeated
  `times` times. Used only to express oversized-content vectors without a
  kilobyte-long literal; the engine runner expands it before use.

## Coverage of the minimum set

| Plan requirement | Where |
|------------------|-------|
| Idempotency replay + dedup-tombstone expiry (`RECEIPT_EXPIRED`) | `idempotency-replay.json` |
| CAS conflict (`REVISION_CONFLICT`) | `cas-conflict.json` |
| Candidate placement inheritance + consumption atomicity (scrub + `content_digest`) | `candidate-consumption.json` |
| Grant coverage incl. `shared_access` / `subject_all` + non-amplification rejection | `grant-coverage.json` |
| Space freeze allowlist + drain barrier (`pending_receipts == 0`) | `space-freeze.json` |
| Subject/context fencing (`SUBJECT_FENCED`, sibling contexts usable) | `subject-fencing.json` |
| Cursor invalidation (`CURSOR_INVALID`) | `cursor-invalidation.json` |
| Chunk round-trip + digest verification | `transfer-chunks.json` |
| Staged-import verification report + explicit-only `adopt_grants` + imported-reflection stale | `staging-import.json`, `reflection-staleness.json` (`import-activation-stales-reflection`) |
| Profile-admission three-state distinguishability | `profile-admission.json` |
| Profile-update ⇒ reflection-stale trigger | `reflection-staleness.json` |
