# SPOKE Protocol (umbrella)

> **Status:** Normative (v0.1)  
> **Document class:** Master  
> **Owns:** Cross-cutting protocol framing for data + ops layers

## Problem & user value

Story-AI products each invent local shapes for knowledge units, checker I/O, and context assembly. SPOKE provides a **shared wire dialect** so products can exchange KnowledgeEntry data and ops without sharing a runtime, database, or daemon.

**v0.1 delivers:** schema SSOT, generated language packages, and normative docs — not working adapters. **Operations library deepen + fixtures** delivered deepen helpers and protocol JSON at `fixtures/toy-world/`; harness ownership moves to `fixtures/toy-world/tests/` (boundary correction, 2026-07-23).

## Three columns (Thrust A)

SPOKE Thrust A spans **data wire**, **ops wire**, and a **hand-written operations behavior library** — see [`.mstar/roadmap.md`](../roadmap.md). v0.1 delivered columns 1–2; operations library first slice delivered column 3.

| Column | Responsibility | Normative doc | Artifact home |
|--------|----------------|---------------|---------------|
| **1. Data** | Seven required objects: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, **Rule**, **TimelineEvent** | [`spoke-data-model.md`](spoke-data-model.md) | `schemas/data/`, `schemas/common/` |
| **2. Ops wire** | Five baseline operations (10 request/response schemas): upsert, extract→promote, relate, check, assemble; optional `project` / `compute` under `l2-computable` (+4 schemas when shipped) | [`spoke-ops.md`](spoke-ops.md) | `schemas/ops/` |
| **3. Ops library** | Pure lifecycle invariants JSON Schema cannot express (promote gate, Finding transitions, extensions preserve, AssemblePacket builders) | [`spoke-operations.md`](spoke-operations.md) | `packages/spoke-operations/` (`@42ch/spoke-operations`) |

**Invariant:** generated `@42ch/spoke-schemas` types are wire truth; `@42ch/spoke-operations` is hand-written behavior on those types — not a third runtime, daemon, or transport binding.

**Protocol layers + Rule/TimelineEvent deepen (architect-locked):** `Rule` (L6) and `TimelineEvent` (L5) in `schemas/data/`; field tables in [`spoke-data-model.md`](spoke-data-model.md). Shared `Scope`, `TimelineScale`, and `ForkId` in `common.schema.json`; `check-request` / `assemble-request` `$ref` shared `Scope`; all ops responses use `oneOf` success | `{ error: ErrorEnvelope }` — see [`spoke-ops.md`](spoke-ops.md). **23** hand-authored schema files (baseline + optional `l2-computable` ops).

## Nine-layer model (L0–L8)

Normative chapter: [`spoke-protocol-layers.md`](spoke-protocol-layers.md). Integrators declare **baseline** (`spoke-baseline`) vs optional **`l2-computable`** / **`l5-fork`** capability flags. **`l2-computable`** covers optional `body.state` / `body.computable`, `TimelineEvent.computable_logs`, and optional `project` / `compute` ops (Session lifecycle via op `session_id` — no durable Session wire object). **`l5-fork`** covers optional `TimelineEvent.fork_id` / `parent_fork_id` and optional `Scope.fork_id` filter (`ForkId` in `common.schema.json`). L5 Timeline projection tiers use wire vocabulary **`brief` / `narrative` / `moment`** via optional `timeline_scale` — distinct from L8 **`AssemblePacket`** context assembly (see layers spec §L5 rule 4: L5 `moment` tier ≠ L8 `assemble` op).

**Schema file count:**

| Slice | Hand-authored files | Breakdown |
|-------|---------------------|-----------|
| **Protocol layers + Rule/TimelineEvent (committed)** | **19** | 2 common + 7 data + 10 ops — `rule-event` + `ops-harden` (shared `Scope`, `rules[]`, error-envelope on all responses) |
| **Operations library deepen + fixtures** | **19** (unchanged) | Deepen helpers + `fixtures/toy-world/` JSON on `main`; harness relocates to `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`) — see repository layout |
| **Optional `l2-computable` ops (`project` / `compute`)** | **23** | +4 ops schemas; optional capability — baseline integrators unchanged |

Update [`schemas/README.md`](../../schemas/README.md) checklist in the same commit as schema land.

## Extensions

Every durable data object MUST include:

```json
"extensions": { "<namespace>": { } }
```

| Rule | Requirement |
|------|-------------|
| Namespace keys | Product-chosen ids matching `^[a-z][a-z0-9_-]*$` |
| Values | Opaque JSON objects |
| Round-trip | Adapters MUST preserve unknown namespaces and unknown keys inside a namespace |
| Core fields | Protocol objects use `additionalProperties: false`; extensions are the sole product-specific bag |

## Schema URI convention

Committed schemas use `https://spoke42.invalid` in `$id` / `$ref` (RFC 6761 reserved; production domain TBD). Do not embed unresolved template placeholders inside JSON `$id` strings.

## Language packages

| Language | Package | Generator | Output path |
|----------|---------|-----------|-------------|
| TypeScript | `@42ch/spoke-schemas` | `json-schema-to-typescript` | `packages/spoke-schemas/src/generated/` |
| Rust | `spoke-schemas` | `typify` | `crates/spoke-schemas/src/generated/` |
| TypeScript (hand-written) | `@42ch/spoke-operations` | — (not codegen) | `packages/spoke-operations/src/` |

`schemas/` is the only hand-authored wire truth. Generated output is committed; drift fails `verify-codegen`.

### Codegen layout (v0.1)

```text
spoke/
├── package.json                 # scripts: codegen, verify-codegen
├── pnpm-workspace.yaml          # packages: ["packages/*", "tooling/*", "fixtures/*"]
├── Cargo.toml                   # workspace; members = ["crates/spoke-schemas"]
├── schemas/                     # SSOT (hand-authored)
├── tooling/codegen/             # orchestrates jstt + typify (private package)
├── packages/spoke-schemas/       # @42ch/spoke-schemas (published path TBD)
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts             # flat re-exports
│       └── generated/           # COMMITTED; mirrors schemas/ tree
│           ├── common/
│           ├── data/
│           └── ops/
└── crates/spoke-schemas/
    ├── Cargo.toml
    └── src/
        ├── lib.rs               # pub mod generated; flat re-exports
        └── generated/           # COMMITTED; mirrors schemas/ tree
            ├── mod.rs
            ├── common/
            ├── data/
            └── ops/
```

**Codegen rules:**

| Rule | Detail |
|------|--------|
| Trigger | `pnpm run codegen` from repo root |
| Verify | `pnpm run verify-codegen` → non-zero if generated tree differs from `schemas/` |
| Commit policy | Schema change + regenerated output in the **same commit** |
| Edit policy | Never hand-edit `*/generated/**` |
| Module mirror | Generated folder names mirror `schemas/{common,data,ops}` |
| Public API | Both packages re-export all leaf types from `index.ts` / `lib.rs` |

Detail: [`schemas/README.md`](../../schemas/README.md).

## Repository layout (v0.1)

| Path | v0.1 expectation |
|------|------------------|
| `.mstar/specs/` | Normative protocol docs (this file + data + ops + operations detail) |
| `schemas/` | JSON Schema SSOT |
| `tooling/codegen/` | Codegen runner (not published) |
| `packages/spoke-schemas/` | Generated TypeScript |
| `packages/spoke-operations/` | Hand-written operations library — pure helpers only; no fixture harness or AJV |
| `crates/spoke-schemas/` | Generated Rust |
| `fixtures/toy-world/` | Protocol conformance JSON + AJV/Vitest harness (`tests/`; workspace package `@42ch/spoke-fixture-toy-world`) — harness MUST NOT live under `packages/spoke-operations/` |
| `adapters/` | README purpose note only; product adapter packages deferred |

## v0.1 acceptance (umbrella)

Current wire bar: seven data objects (including `Rule` + `TimelineEvent`), five baseline ops plus optional `project` / `compute`, **23** schema files; normative vocabulary locks `KnowledgeEntry` / `TimelineEvent` in this tree and [`CONCEPTS.md`](../../CONCEPTS.md).

**CI + inventory (required):**

1. Spec trio (`spoke-protocol`, `spoke-data-model`, `spoke-ops`) aligned with `schemas/` tree for baseline data objects + five ops
2. **CI green on PR** — [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) runs on `pull_request` and on pushes to `main` / `iteration/**`; all four jobs must pass:
   - `verify-codegen` — `pnpm run verify-codegen` (schema drift fails the build)
   - `typescript` — `pnpm -F @42ch/spoke-schemas typecheck` + `build`; `@42ch/spoke-operations` typecheck + test
   - `rust` — `cargo check -p spoke-schemas`
   - `verify-version` — `pnpm run verify:version` (lockstep SemVer across manifests and README badges; see [`spoke-version-release.md`](spoke-version-release.md))
3. Same checks pass locally (`pnpm run verify-codegen`, package typecheck/build, `cargo check -p spoke-schemas`, `pnpm run verify:version`)
4. Extensions contract enforced in data schemas
5. Protocol conformance fixtures at `fixtures/toy-world/` (`adapters/README.md` only for adapters)

**Current data inventory (normative):**

| Object | Schema |
|--------|--------|
| KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket | `schemas/data/*.schema.json` |
| Rule | `schemas/data/rule.schema.json` — see [`spoke-data-model.md`](spoke-data-model.md) |
| TimelineEvent | `schemas/data/timeline-event.schema.json` — see [`spoke-data-model.md`](spoke-data-model.md) |

## Non-goals (v0.1)

| Out of scope | Rationale |
|--------------|-----------|
| Product ↔ SPOKE conversion packages | Adapter packages deferred |
| Required WASM / compute engines in protocol | Optional `l2-computable` shapes I/O only — engines are product-owned |
| Fork merge / rebase engines and world-history stores | Product-owned — protocol documents interchange fields only (`fork_id`, `parent_fork_id`, `Scope.fork_id`) |
| Shared runtime, daemon, or MCP server | Protocol repo only |
| npm/crates.io publish (including from CI) | Workspace-local packages suffice for v0.1 |

## Roadmap pointer

| Phase | Deliverable |
|-------|-------------|
| **v0.1 (delivered)** | Data + ops **wire** SSOT, `@42ch/spoke-schemas` / `spoke-schemas`, empty adapter dirs, CI gate |
| **Operations library first slice (delivered 2026-07-23)** | Hand-written `@42ch/spoke-operations` (column 3) + integrator README EN/CN — see [`spoke-operations.md`](spoke-operations.md) |
| **Protocol layers + Rule/TimelineEvent (delivered)** | Normative L0–L8 + capability levels; `Rule` + `TimelineEvent` field semantics; ops harden (Scope neutrality, Check≠Assemble, error-envelope R3) |
| **Operations library deepen + fixtures (delivered 2026-07-23)** | Deepen `@42ch/spoke-operations` helpers + `fixtures/toy-world/` conformance graph; AJV/Vitest harness at `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`) — **no adapters** |
| **Next** | Implementable adapter packages (product DTO ↔ SPOKE) |
| **North star** | Cross-product narrative **KnowledgeEntry** dialect for consistency-check and context-assembly I/O **without** a shared runtime |

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer, annotated tags, CI-gated GitHub Release (no registry publish) |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Nine layers L0–L8, capability levels, Domain Profile, layer ↔ artifact map |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects, extensions, open vocabulary, Rule/TimelineEvent (protocol layers deepen) |
| [`spoke-ops.md`](spoke-ops.md) | Five ops, error envelope, Scope neutrality, `assemble` wire-only boundary |
| [`spoke-operations.md`](spoke-operations.md) | Operations behavior library — `SpokeResult`, helper families (first slice + deepen), hard In/Out |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist (23 files committed) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | KnowledgeEntry / TimelineEvent vocabulary; dual-concern rule |
| [`STRATEGY.md`](../../STRATEGY.md) | Protocol-not-runtime positioning and v0.1 scope |
