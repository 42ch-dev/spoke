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
profile, and one reflection exported with `stale: false`. Every input its
reflection pins is at **revision 1**: the three records at revision 1, the
profile at revision 1, and `basis.profile_revisions = { "personality": 1 }`. A
target provider that allocates fresh revisions starting at 1 — the common
convention — can therefore reproduce the pinned basis exactly, which is what
makes the imported-staleness case discriminating instead of merely plausible.
It is not a vector file and is excluded from corpus loading (only top-level
`*.json` files are vectors).

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
  - loopback runner: a `variable` wire case is evaluated twice against
    **identical state**. The classification case saves a full provider snapshot,
    captures the content, records the disposition, restores the snapshot, and
    re-executes the **byte-identical request** (same subject, same content, same
    `idempotency_key` — a fresh execution because the restore erased the dedup
    record), then compares the two observed dispositions. Either capture
    returning no candidate (§7.11 dedup is permitted) — or a harness without
    snapshot support — makes the case **inconclusive** through
    `inconclusive_when`, never a failure and never a synthesized disposition. No
    cross-provider ranking or disposition is imposed by any vector.

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
  "stage_from": {
    "asset": "assets/tw-minimal-export.json",
    "chunk_limit": 4096,
    "idempotency_key_pattern": "{export_id}:chunk:{ordinal}",
    "replay": { "chunk": 0 }
  },
  "advance": { "choose": "min", "instants": [{ "from": "setup.0", "path": "limits.receipt_retention_seconds", "offset_seconds": 1 }] },
  "release": ["setup.1"],
  "verify_export": { "receipt": "setup.1" },
  "pinned_basis_check": { "reflection": "op.reflections.0", "inconclusive_on_mismatch": true },
  "inconclusive_when": [{ "path": "then.1.result.candidate_ids", "isEmpty": true }],
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
    response — used to compare an effect set before and after an operation or
    between two equivalent subjects.
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
  order. `idempotency_key_pattern` is mandatory and is expanded per chunk with
  `{export_id}` (the exported bundle's `export_id`) and `{ordinal}` (the chunk
  ordinal), giving each chunk a deterministic distinct key that is reused
  verbatim on replay. `request` carries the op-level `conflict_policy` /
  `adopt_grants`, which apply to ordinal 0; the step's response is the final
  chunk's receipt. Optional `replay: { "chunk": <n> }` re-sends that chunk with
  the identical key immediately after its first send and asserts the same
  `result.chunks_received` (no double-count) before continuing.
- `verify_export` — on an `exports.get` step: after the first chunk is fetched,
  the harness fetches every remaining ordinal from the referenced receipt's
  `result.export_id` / `result.chunk_count`, recomputes each `chunk_digest` (JCS
  of the chunk minus `chunk_digest`) and the reassembled bundle digest (JCS minus
  `digest`), and reports the `verified` booleans. Fabricated digests therefore
  fail the case.
- `remap_policy_check` — `{ "report": "<step reference to staged_report>", "maps": ["record_id_map", "candidate_id_map"], "policy": "identity", "inconclusive_on_violation": true }`.
  §7.31 requires the target to **reassign** record/candidate ids, not to change
  their values, so a co-operative provider may adopt an identity remap on an
  empty target space. The harness reads the staged report it already received
  and verifies the declared policy: every entry of the named maps must satisfy
  `value == key`. A violation records `inconclusive` with that reason (honest for
  third parties) and stops the remaining assertions.
- `pinned_basis_check` — after the op under test returns reflections, the harness
  calls `memory.reflections.state` for the same space/subject with the
  reflection's pinned `selection` (and `filter`, when present) and compares the
  recomputed `state_digest` with the reflection's `basis.state_digest`. The
  imported-staleness case is **PROVEN only** when all three hold:
  1. the provider-internal control passes — the same reflection on the source
     space reads `stale: false` while its basis is unmoved. This step is a
     required assertion, so a provider that hardcodes `stale: true` **fails** the
     case rather than passing it;
  2. the recomputed target `state_digest` **equals** the pinned digest (the
     translated basis reproduces; the fixture pins every basis input at revision
     1 so a provider allocating fresh revisions from 1 can reproduce it);
  3. the imported reflection still reads `stale: true`.

  If (2) fails the case is recorded `inconclusive`
  (`inconclusive_on_mismatch: true`) — never proven, never a failure.

  For the imported-staleness case the policy is **declared**, so the rule is
  stricter (`inconclusive_on_mismatch: false`): once the declared-policy checks
  pass — identity remap verified from the staged report, the post-activation
  record revision read back as 1, and the policy version equal — the translated
  pinned basis is exactly reproducible and equality **must** hold. A mismatch
  under those conditions means the provider is non-compliant with its own
  declared policy, so the case **fails** rather than degrading to inconclusive.
  Inconclusive remains only for third parties that declare no policy or fail a
  precondition, and the coverage map states that distinction.

  **Black-box ceiling:** this is the strongest discrimination an outside observer
  can reach without provider internals — a provider that both re-digests the
  translated basis identically and still reports `stale: true` has demonstrated
  the §7.31 rule, while a provider that cannot reproduce the digest stays
  `inconclusive` rather than being falsely certified.
- `policy_version_check` — `{ "observed": "<step reference>", "pinned": "<step reference>", "inconclusive_on_mismatch": true }`.
  For the import case it compares the provider's `memory.describe`
  `engine_policy_version` with the imported reflection's
  `generator.engine_policy_version`: a provider that reports a different policy
  version cannot reproduce the pinned digest, so the case is recorded
  `inconclusive` with that reason instead of silently failing or passing.
- `inconclusive_when` — array of `{ "path": "...", "isEmpty": true }`: if any
  named response value is an empty array, the case is recorded `inconclusive`
  and its remaining assertions are not counted. Used only where the protocol
  explicitly permits an empty outcome (for example a zero-candidate capture).
- `harness.save_snapshot` / `harness.restore_snapshot` — **harness capability
  steps**, not protocol ops:
  `{ "op": "harness.save_snapshot" | "harness.restore_snapshot", "request": { "space_id": … } }`.
  `save_snapshot` captures the complete provider state (records, candidates,
  receipts, dedup/idempotency records, profiles, reflections); `restore_snapshot`
  restores it completely. Because the restore erases the dedup record, a request
  repeated afterwards is a **fresh execution, not a replay**, which is what lets
  a case re-run a byte-identical request — same `idempotency_key` included —
  against genuinely identical state. The Task 3 harness must implement both; a
  harness that does not must record the case `inconclusive` — never a failure and
  never a cross-request rationalization. Steps may otherwise carry only
  `memory.*` ops.
- `requires` — declarative environment preconditions (negotiated capability set,
  configured `required_profile_kinds`, seeded grants/profiles/records, an unmoved
  source-space basis, a held-pending capable harness, the polling obligation that
  `memory.receipts.get` is retried until the receipt is terminal). The harness
  must satisfy them; they are not assertions.

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
| Chunk round-trip + digest verification, per-chunk staging keys and replay | `transfer-chunks.json`, `staging-import.json` | engine (partition/digests) + wire (round-trip, staging) |
| Staged-import verification report + explicit-only `adopt_grants` (verified after activation, alongside the staging principal's own grant) | `staging-import.json` | engine (report policy) + wire (staging, activation, adoption) |
| Imported-reflection `stale: true` unconditionally — `proven` deterministically for providers declaring the identity-remap / revision-1 / matching-policy contract (all three verified in-case; a digest mismatch under those preconditions fails as non-compliant); third parties that declare no policy or fail a precondition are recorded `inconclusive` with reason, never `proven` | `reflection-staleness.json` (`import-activation-stales-reflection-unconditionally`) | wire |
| Profile-admission three-state distinguishability | `profile-admission.json` | wire |
| Profile-update ⇒ reflection-stale trigger | `reflection-staleness.json` | wire (+ engine digest policy) |
| Classification/recall determinism per provider (classification requires `harness.save_snapshot`/`restore_snapshot`, else inconclusive) | `classification.json` (`classification-stable-within-a-provider`), `recall-budget.json` | wire (+ engine repeat) |
