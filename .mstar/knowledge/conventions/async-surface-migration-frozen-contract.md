---
module: spoke-operations
date: 2026-08-05
problem_type: convention
category: conventions
severity: medium
applies_when: ["migrating a library surface across a breaking change (pre-1.0)", "converting synchronous port interfaces and entrypoints to async in multiple languages", "changing cross-language API signatures that must stay in parity", "deciding whether to keep a legacy surface behind a shim", "coordinating tracked-spec and consumer-docs updates with a code migration"]
related_components: ["spoke-connect", "spoke-schemas", "fixtures/toy-world"]
tags: [async-migration, frozen-contract, behavior-parity, breaking-change, async-trait, ts-rust-parity, consumer-docs, surface-migration]
---

# Pure-async library-surface migration against a frozen contract

## Context

A library that delegates I/O to injected ports sometimes starts life with a synchronous facade ("adapters own async I/O behind this boundary"). That facade is leaky: adapters block anyway, and a network proxy adapter can never be a drop-in for sync ports (no synchronous await in JS). The honest shape is an **async-native surface**: every port method and every orchestration entrypoint returns an awaitable result, and the library itself stays I/O-free — it only awaits the injected ports.

This convention is the migration method used to convert spoke-operations (TS + Rust) from the sync `SpokeResult` surface to pure-async `Promise<SpokeResult<T>>` / `async fn … -> SpokeResult<T>`, across 8 port interfaces, 4 composed adapter aliases, 9 `orchestrate*` entrypoints, the `ToyWorldAdapter` reference, and both test suites — with the sync surface removed entirely.

## Guidance

### 1. Freeze the target contract before implementation

Before any code changes, write the exact target surface as an **architect-locked contract**: every port signature, every entrypoint signature, the async mechanism, and the invariants. Implementers code against the frozen file; **drift is a defect**. The contract is the SSOT that de-risks a mechanical migration — no signature is invented during implementation, and TS↔Rust parity questions are settled once.

Frozen invariants that make the migration safe:

- **Return envelope unchanged.** Application outcomes stay `SpokeResult<T>`; reject codes and error-envelope mapping untouched. Only the surface becomes awaitable.
- **Pure library.** `await` appears only on injected port method calls (and awaiting those inside orchestration); pure validation/OCC helpers stay synchronous; checker callbacks (product logic, not ports) stay synchronous `(input: CheckRunInput) => SpokeResult<Finding[]>` / `FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>`.
- **No union returns.** No `T | Promise<T>`, no sync variants, no compatibility shim.

### 2. Map the mechanism per language

| Surface | TypeScript | Rust |
|---------|------------|------|
| Port method | `(…args) => Promise<SpokeResult<T>>` | `#[async_trait] async fn …(&self, …) -> SpokeResult<T>` |
| Entrypoint | `export async function orchestrateX(…): Promise<SpokeResult<R>>` | `pub async fn orchestrate_x(…) -> SpokeResult<R>` |
| Checker callback | sync | `F: FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>` sync |

Rust specifics:

- Use the **`async-trait` crate** (proc-macro, no runtime as a normal dependency) rather than native `async fn` in traits when the traits must stay **object-safe** for dynamic availability probes (`as_computable`, `as_fork_timeline`). Native `async fn` in traits is not object-safe.
- Futures are **`Send`** (default `#[async_trait]`, never `?Send` on normative ports). Do **not** force `Send + Sync` onto the trait definitions themselves — that keeps single-threaded test stubs simple; document that production multi-threaded hosts require `Send + Sync` impls.
- Test-driving futures without a runtime in normal deps: `pollster::block_on` (dev-dependency) keeps every `#[test]` plain — no `#[tokio::test]` macro churn.
- Test-stub gotcha: `RefCell`-based stubs break the `Send`-future policy (`RefCell` is `!Sync`) — switch stubs to `Mutex` when futures must be `Send`.

### 3. Behavior parity is the gate — not "it compiles"

The migration is mechanical; the acceptance bar is **behavior parity**:

- Same request, adapter state, checker output → same `SpokeResult` shape, stable reject code, and persisted-effect semantics as pre-migration.
- The strongest evidence is **identical test counts and identical assertions**: the test diff should contain only `async`/`await` additions — zero assertion changes. Reject-path coverage (OCC `REVISION_CONFLICT` / `STORED_REVISION_STALE`, `CAPABILITY_PORT_MISSING`, extension round-trips) must be visibly retained.
- Keep the reference adapter (`ToyWorldAdapter`) as the integration canary across both languages.

### 4. Remove the old surface entirely (pre-1.0)

Do **not** keep parallel sync variants, sync/async union return types, or a silent sync→async compatibility shim. A shim preserves the leaky facade forever and doubles the surface to maintain. When the change is pre-1.0, the decision is: **one durable surface** — local adapters and remote proxies both async. Consumers migrate on their own schedule; the breaking change is accepted and documented.

### 5. Tracked specs update after the code ships (facts-only rule)

Do **not** pre-claim the new surface in tracked specs before the code exists. Land the spec rewrite as a plan task **after** the migration is green, stating the async surface as an affirmative current fact ("The async form is the only surface…"), with signature tables matching the shipped code and the frozen contract verbatim. Grep the spec for stale remnants ("Ports are synchronous", "future async surface", `Async*Port` names) before closing.

### 6. Sweep consumer docs in the same wave — the spec is not the only surface

The classic miss: the tracked spec is updated, but human-facing docs (root READMEs, package READMEs, tutorials, how-tos, EN/CN twins) still show the **removed** call shapes. Consequences are real: TS integrators silently call async entrypoints without `await` (a Promise is returned, errors vanish); Rust integrators get code that no longer compiles. Breakage of this kind was only caught at QA, after the spec fix wave. Include a **consumer-docs sweep** in the migration scope:

- every `orchestrate*(…)` call statement becomes `await orchestrate*(…)` inside an `async fn`/`async function`;
- every port-signature block becomes `Promise<SpokeResult<…>>` / `async fn … -> SpokeResult<T>`;
- verify with greps (no bare sync call statements left) plus the docs pipeline (twin-parity + deadlink checks + build).

### 7. Verify the blast radius

Before migrating, repo-wide grep for consumers of the changing surface (port methods, entrypoints) so the scope is known: which packages, fixtures, and tooling call into it, and which other libraries depend on the packages. A library that does not consume the surface (e.g. the connect packages before RemoteAdapter) stays untouched; the migration diff stays surgical.

## Why This Matters

A sync facade over async I/O is a standing invitation to block the event loop and a hard blocker for network proxies. The frozen contract turns a two-language, multi-surface migration into a mechanical transform with a measurable gate — behavior parity — instead of a design debate happening mid-implementation. Removing the old surface (rather than shimming) keeps one honest model. Missing the consumer-docs sweep silently breaks every integrator that follows the docs, which is worse than the migration itself. The spec-after-code sequencing keeps tracked facts truthful at every commit.

## When to Apply

- Converting sync port interfaces / entrypoints to async in a pure orchestration library (one or more languages).
- Any pre-1.0 breaking library-surface change where the old surface would otherwise be shimmed into permanence.
- Cross-language signature migrations where TS↔Rust parity must hold (freeze the mapping table first).
- Coordinating code, tracked-spec, and consumer-docs updates for a breaking change.

## Examples

### Before (sync facade — leaky)

```ts
// adapter blocks internally; a network adapter cannot be a drop-in
export function orchestrateUpsert(ports: BaselinePorts, request: UpsertRequest): SpokeResult<UpsertResponse> { … }
```

### After (async-native, only surface)

```ts
export async function orchestrateUpsert(ports: BaselinePorts, request: UpsertRequest): Promise<SpokeResult<UpsertResponse>> {
  const stored = await ports.getKnowledgeEntry(request.entry.id); // await only injected ports
  …
}
```

```rust
#[async_trait]
impl KnowledgeEntryPort for MemoryAdapter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> { … }
}

pub async fn orchestrate_upsert(ports: &impl BaselinePorts, request: UpsertRequest) -> SpokeResult<UpsertResponse> { … }
```

### Consumer call (both local and remote adapters)

```ts
const result = await orchestrateUpsert(localAdapter, request);
const remoteResult = await orchestrateUpsert(remoteAdapter, request); // same surface, network under the hood
```

## See also

- `.mstar/specs/spoke-operations.md` — current async port/orchestrator surface as normative facts
- `architecture-patterns/adapter-injection-orchestration.md` — the port-injection model this surface serves
- `architecture-patterns/encapsulated-remote-adapter-bridge.md` — the remote proxy that motivated async-native ports
- `architecture-patterns/rust-spoke-operations-parity.md` — Rust `SpokeResult` idiom and reject-code parity
- `architecture-patterns/consumer-readme-twin.md` — consumer-docs conventions for the docs sweep
