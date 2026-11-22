# Adoption conformance vectors

Provider-neutral conformance corpus for the optional
`spoke-knowledge-adoption` capability (`spoke-knowledge-adoption` wire
contract), plus the runner contract that consumes it. One JSON file per
scenario; each case names the contract section under test, its seeded
state, the step under test, and the expected observable.

Two runners share the corpus — the same JSON case produces the same wire
outcome on both stacks:

| Runner | Location | Executes |
|--------|----------|----------|
| Rust loopback | `crates/spoke-connect/tests/adoption_conformance.rs` (`cargo +nightly test -p spoke-connect --features remote-adapter adoption_conformance`) | `surface: "wire"` cases through a real responder ↔ `RemoteAdapter` loopback; `surface: "service"` cases through `AdoptionPortService::invoke` directly |
| TypeScript loopback | `packages/spoke-connect-ts/tests/remote/adoption-face.test.ts` (`pnpm -F @42ch/spoke-connect test`) | The identical corpus through `connectResponder` ↔ `RemoteAdapter` and `AdoptionPortService.invoke` |

The runner's scripted `AdoptionPort` implements the provider-owned
contract semantics over the seeded tables — owner-private admission
(`disclosure: "owner-private"` is visible only when `owner` equals the
admitted `Scope.viewpoint`), the shared `resolve_retirement_chain`
helper over the seeded `successors` map, the copy rules (body /
canonical_name / entry_type / modules minus `cognition` preserved,
`source_anchor` cleared, `extensions` minus `mindcore_adoption`
verbatim, owner/disclosure reset to `input.destination_governance`),
create-only OCC, and the anchor repeat classification. Capability
gating, the compound `spoke-knowledge-adoption` + `spoke-knowledge`
gate, request decode, the connect idempotency dedup, replay
normalization and served-side response validation are therefore
measured on the shipped `AdoptionPortService`, responder dispatch and
`RemoteAdapter`, never on the fixture.

The fixture's scoped store (the `RemoteKnowledgeFace` backing store and
the adoption destination map, shared so the written copy is observable
through `port.knowledge.get`) applies the shared
`assert_adoption_anchor_preserved` gate before its OCC check — the
`anchor-immutable` vectors measure the shipped helper, the knowledge
face admission path, and the refusal's wire round-trip.

## Scenario files

| File | Cases | Purpose |
|------|-------|---------|
| `adopt-basic.json` | 6 | `adopted` outcome + populated `adoption_anchor`, copy-rule observables (anchor write, `source_anchor` cleared, `modules.cognition` dropped, `mindcore_adoption` dropped, governance reset), `destination_entry_id` override, compound capability gate, absent-face dispatch-deny, service decode boundary |
| `idempotent-repeat.json` | 4 | same-key recorded-outcome replay normalized to `already_present`, same-key different-digest `IDEMPOTENCY_CONFLICT`, anchor-repeat `already_present` under a different key, `RECEIPT_EXPIRED` after bounded-store eviction |
| `revision-conflict.json` | 2 | `REVISION_CONFLICT` + `details.adopted_revision` on a different `source_revision`, unrelated-occupant create-only refusal without `adopted_revision` |
| `private-source.json` | 4 | owner-private mismatched-viewpoint `NOT_FOUND` (recorded and re-executed repeats produce the same absence), missing-source identical absence, out-of-admitted-scope identical absence, owner-private visible to its owner |
| `retirement-chain.json` | 6 | merged→live terminal resolution (anchor records the terminal triple), cycle, zero/multiple successors, deleted terminal, 33rd-edge limit — each `SOURCE_RETIRED` with the exact `details.retirement_chain` |
| `anchor-immutable.json` | 3 | `put` mutating a stored `adoption_anchor` → `ADOPTION_ANCHOR_IMMUTABLE` + `details.entry_id` (replace and remove), identical-anchor put passes |

Totals: 25 cases — 24 wire, 1 service.

## Case envelope

```json
{
  "id": "unique-case-id",
  "invariant": "contract section anchor (e.g. §Repeat and idempotency)",
  "surface": "wire | service",
  "outcome": "identical",
  "what": "one line naming the observable",
  "input": {
    "sources": [
      {
        "entry_id": "ke_src",
        "successors": ["ke_next"],
        "entry": { "...": "KnowledgeEntry wire object" }
      }
    ],
    "destinations": [{ "...": "KnowledgeEntry wire object" }],
    "admitted_viewpoint": "vp_caller",
    "admitted_scopes": ["spc_shared", "spc_local"],
    "destination_governance": { "owner": "vp_caller", "disclosure": "owner-private" },
    "client_capabilities": ["spoke-knowledge", "spoke-knowledge-adoption"],
    "adoption_face": true,
    "dedup_capacity": 1024
  },
  "setup": [{ "op": "knowledge.adopt", "request": {} }],
  "op": "knowledge.adopt",
  "request": { "source_scope_id": "...", "...": "KnowledgeAdoptRequest wire object" },
  "expect": { "...": "see below" },
  "then": [{ "op": "port.knowledge.get", "request": {}, "expect": {} }],
  "requires": ["declarative precondition"]
}
```

- `input.sources[]` seeds the scripted provider's source table under
  `source_scope_id`; `successors` is the storage-owned successor map a
  `merged` node resolves through (`resolve_retirement_chain`'s
  `successors_of`). Every source entry is stored under the case's
  `spc_shared` scope.
- `input.destinations[]` seeds the destination store (and thus the
  knowledge face's scoped store) under `spc_local`.
- `input.admitted_viewpoint` becomes the admitted `Scope.viewpoint` the
  provider compares an owner-private source's `owner` against; absent
  means no viewpoint (an owner-private source is then invisible).
- `input.admitted_scopes` bounds the provider's source/destination
  scope admission; it defaults to `["spc_shared", "spc_local"]`.
- `input.destination_governance` supplies the copy's `owner` /
  `disclosure` values (the contract's "destination's own values");
  absent means neither field is written.
- `input.client_capabilities` overrides the dialing manifest's
  `capabilities` (default `["spoke-knowledge", "spoke-knowledge-adoption"]`);
  the host always advertises both.
- `input.adoption_face: false` dials a responder whose ports composite
  has no adoption service (the documented dispatch-deny branch).
- `input.dedup_capacity` injects a bounded `InMemoryHistoryDedupStore`
  (the adoption dedup seam reuses the history store shape).
- `surface: "service"` runs the steps through `AdoptionPortService::invoke`
  directly (requests the typed client cannot emit, like an absent
  `idempotency_key`).
- `setup` steps execute first and assume success unless they carry
  their own `expect`; `then` steps execute after the op under test. A
  `then` step's `expect.provider_calls` asserts the cumulative provider
  call count, which is how "replay without a second port call" is
  measured.
- `expect` vocabulary (closed):
  - `error_code` — a `SpokeRejectCode` name from the refusal
    vocabulary, or the dispatch-layer `capability_missing` /
    `op_unsupported` wire codes (asserted on the client as
    `CAPABILITY_PORT_MISSING` with `details.wire_code`).
  - `details` — object that must be contained in the reject `details`.
  - `absent_details` — member names that must NOT appear in the reject
    `details` (the generic-absence rule: `NOT_FOUND` carries no
    source-identifying members).
  - `success` — the step returns a success payload; for
    `knowledge.adopt` the observed shape is `{ "outcome": ..., "entry":
    ... }`, for `port.knowledge.get` / `port.knowledge.put` the entry.
  - `state` — `{ "path": "dotted.path[0]", "equals": <json> }` equality
    assertions against the success value.
  - `present` — dotted paths that must exist (explicit-empty members
    survive the round-trip).
  - `absent` — dotted paths that must NOT exist on the success value
    (cleared `source_anchor`, dropped `modules.cognition`,
    dropped `extensions.mindcore_adoption`).
  - `provider_calls` — the cumulative scripted-port call count.
- `requires` — declarative preconditions; not assertions.
