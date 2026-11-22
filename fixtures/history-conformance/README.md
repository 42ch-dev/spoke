# History conformance vectors

Provider-neutral conformance corpus for the optional `spoke-history`
capability (`spoke-history` wire contract), plus the runner contract that
consumes it. One JSON file per scenario; each case names the contract
section under test, its seeded state, the step under test, and the expected
observable.

One runner shares the corpus:

| Runner | Location | Executes |
|--------|----------|----------|
| Loopback | `crates/spoke-connect/tests/history_conformance.rs` (`cargo +nightly test -p spoke-connect --features remote-adapter history_conformance`) | `surface: "wire"` cases through a real responder ↔ `RemoteAdapter` loopback; `surface: "service"` cases through `HistoryPortService::invoke` directly |

The runner's scripted `HistoryPort` is deliberately dumb: it replays the
seeded snapshot re-stamped to the reconstructable revision, applies the
contract §Checkpoints pruning rule to seeded checkpoint sets, and answers
the refusal vocabulary — it enforces no connect-layer behavior. Capability
gating, admission, effective-scope resolution, decode, dedup and served
validation are therefore measured on the shipped `HistoryPortService`,
responder dispatch and `RemoteAdapter`, never on the fixture.

The TS loopback leg (`packages/spoke-connect-ts/tests/remote/history-face.test.ts`)
asserts the same serving matrix with direct cases; the canonical snapshot
vectors are also re-checked there for JCS byte parity.

## Scenario files

| File | Cases | Purpose |
|------|-------|---------|
| `canonical-snapshot.json` | 5 | Replay byte-identity: nested entity revision equals envelope revision, present-empty members survive, extensions round-trip, relation kind |
| `prune-floor.json` | 6 | `{1,3,5}` + `keep=2` worked example, below-floor `HISTORY_TRUNCATED`, at-floor reconstruction, no-op prune (`keep ≥ count`, `keep=0`) |
| `refusals.json` | 8 | Capability gate without a port call, absent face, `INVALID_INPUT` matrix, `HISTORY_ENTITY_UNSUPPORTED`, `HISTORY_NOT_RETAINED`, scope-mismatch refusals |
| `dedup-replay.json` | 4 | §9.1 recorded-outcome replay, `IDEMPOTENCY_CONFLICT`, `RECEIPT_EXPIRED` after eviction, absent `idempotency_key` |

Totals: 24 cases — 20 wire, 4 service.

## Case envelope

```json
{
  "id": "unique-case-id",
  "invariant": "contract section anchor (e.g. §RevisionSnapshot, §9.1)",
  "surface": "wire | service",
  "outcome": "identical",
  "what": "one line naming the observable",
  "input": {
    "entities": [
      {
        "entity_kind": "knowledge_entry",
        "entity_id": "ke_1",
        "checkpoints": [1, 3, 5],
        "stored_snapshot": { "...": "RevisionSnapshot wire object" }
      }
    ],
    "client_capabilities": ["spoke-history"],
    "history_face": true,
    "dedup_capacity": 1024
  },
  "setup": [{ "op": "history.revisions.prune", "request": {} }],
  "op": "history.revisions.get",
  "request": { "entity_kind": "...", "entity_id": "...", "revision": 0 },
  "expect": { "...": "see below" },
  "then": [{ "op": "history.revisions.get", "request": {}, "expect": {} }],
  "requires": ["declarative precondition"]
}
```

- `input.entities[]` seeds the scripted provider: `checkpoints` is the
  retained checkpoint set `C` (sorted oldest→newest), `stored_snapshot` is
  the `RevisionSnapshot` wire object the provider replays (re-stamped to
  the reconstructable revision for revisions without their own checkpoint).
- `input.client_capabilities` overrides the dialing manifest's
  `capabilities` (default `["spoke-baseline", "spoke-history"]`); the host
  always advertises `["spoke-baseline", "spoke-history"]`.
- `input.history_face: false` dials a responder whose ports composite has
  no history service (the documented dispatch-deny branch).
- `input.dedup_capacity` injects a bounded `InMemoryHistoryDedupStore`.
- `surface: "service"` runs the steps through `HistoryPortService::invoke`
  instead of a dialed session (for requests the typed client cannot emit,
  like an absent `idempotency_key`).
- `setup` steps execute first and assume success unless they carry their
  own `expect`; `then` steps execute after the op under test. A `then`
  step's `expect.provider_calls` asserts the cumulative provider call
  count, which is how "replay without a second port call" is measured.
- `expect` vocabulary (closed):
  - `error_code` — a `SpokeRejectCode` name from the refusal vocabulary,
    or the dispatch-layer `capability_missing` / `op_unsupported` wire
    codes (asserted on the client as `CAPABILITY_PORT_MISSING` with
    `details.wire_code`).
  - `details` — object that must be contained in the reject `details`.
  - `success` — the step returns a success payload; the observed shape is
    `{ "snapshot": <RevisionSnapshot> }` for get and
    `{ "deleted": n, "floor_revision": n | null }` for prune.
  - `state` — `{ "path": "dotted.path[0]", "equals": <json> }` equality
    assertions against the success value.
  - `present` — dotted paths that must exist, including explicitly empty
    members (the wire-presence rule).
  - `canonical_equal` — `{ "path": "...", "from": "stored_snapshot" }`:
    RFC 8785 canonical bytes of the response member must equal the
    canonical bytes of the seeded `stored_snapshot`.
  - `provider_calls` — the cumulative scripted-port call count.
- `requires` — declarative preconditions; not assertions.
