# SPOKE Protocol (umbrella)

> **Status:** Normative (v0.1)  
> **Document class:** Master  
> **Owns:** Cross-cutting protocol framing for data + ops layers

## Problem & user value

Story-AI products each invent local shapes for knowledge units, checker I/O, and context assembly. SPOKE provides a **shared wire dialect** so products can exchange KnowledgeEntry data and ops on a common protocol surface.

**v0.1 delivers:** schema SSOT, generated language packages, a hand-written operations library (pure helpers plus adapter ports/orchestration), and normative docs. Protocol conformance JSON and the AJV/Vitest harness live under `fixtures/toy-world/` (`@42ch/spoke-fixture-toy-world`).

## Three columns (Thrust A)

SPOKE Thrust A spans **data wire**, **ops wire**, and a **hand-written operations behavior library**.

| Column | Responsibility | Normative doc | Artifact home |
|--------|----------------|---------------|---------------|
| **1. Data** | Ten data objects: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, **HostCapabilityManifest**, **Rule**, **TimelineEvent**, **MindState**, ToolDescriptor | [`spoke-data-model.md`](spoke-data-model.md) | `schemas/data/`, `schemas/common/` |
| **2. Ops wire** | Five baseline operations (10 request/response schemas): upsert, extract→promote, relate, check, assemble; optional `project` / `compute` under `l2-computable` (+4 schemas when shipped) | [`spoke-ops.md`](spoke-ops.md) | `schemas/ops/` |
| **3. Ops library** | Pure lifecycle invariants and injected adapter orchestration that JSON Schema cannot express | [`spoke-operations.md`](spoke-operations.md) | `packages/spoke-operations/` (`@42ch/spoke-operations`); `crates/spoke-operations/` (`spoke-operations`) |

**Invariant:** generated `@42ch/spoke-schemas` / `spoke-schemas` types are wire truth; `@42ch/spoke-operations` / `spoke-operations` are hand-written behavior on those types, including capability-sliced adapter ports and injection orchestration. TypeScript package is behavioral SSOT; Rust crate is a port at lockstep SemVer. Adapter interfaces are defined in [`spoke-operations.md` §Adapter Interfaces](spoke-operations.md#adapter-interfaces-normative); per-operation orchestration sequences in [`§Injection Orchestration`](spoke-operations.md#injection-orchestration-normative).

**Connect family (opt-in `spoke-connect`):** cross-process interaction envelopes — hello (signed manifest exchange), session, invoke request/response, auth challenge/response — live in `schemas/connect/` under the optional `spoke-connect` capability flag. Connect reuses `HostCapabilityManifest` (hello embeds it by `$ref`) and `error-envelope` (invoke failures), wraps existing ops envelopes as opaque JSON, and keeps peer identity opaque (`peer_id`). Normative interaction semantics: [`spoke-connect.md`](spoke-connect.md).

**Protocol layers (Rule + TimelineEvent + HostCapabilityManifest + ToolDescriptor):** `Rule` (L6) and `TimelineEvent` (L5) in `schemas/data/`; `HostCapabilityManifest` for in-process host collaboration (baseline `HostManifestPort`); `ToolDescriptor` for self-describing tool ABIs on the manifest; field tables in [`spoke-data-model.md`](spoke-data-model.md). Shared `Scope`, `TimelineScale`, and `ForkId` in `common.schema.json`; `check-request` / `assemble-request` `$ref` shared `Scope`; all ops responses use `oneOf` success | `{ error: ErrorEnvelope }` — see [`spoke-ops.md`](spoke-ops.md). **32** hand-authored schema files (26 data/ops + 6 opt-in connect envelopes).

## Nine-layer model (L0–L8)

Normative chapter: [`spoke-protocol-layers.md`](spoke-protocol-layers.md). Integrators declare **baseline** (`spoke-baseline`) vs optional **`l2-computable`** / **`l5-fork`** / **`spoke-connect`** capability flags. **`l2-computable`** covers optional `body.state` / `body.computable`, `TimelineEvent.computable_logs`, and optional `project` / `compute` ops (Session lifecycle via op `session_id` — no durable Session wire object). **`l5-fork`** covers optional `TimelineEvent.fork_id` / `parent_fork_id` and optional `Scope.fork_id` filter (`ForkId` in `common.schema.json`). L5 Timeline projection tiers use wire vocabulary **`brief` / `narrative` / `moment`** via optional `timeline_scale` — distinct from L8 **`AssemblePacket`** context assembly (see layers spec §L5 rule 4: L5 `moment` tier ≠ L8 `assemble` op). **`spoke-connect`** covers the opt-in interaction envelopes in `schemas/connect/` — see [`spoke-connect.md`](spoke-connect.md).

**Schema file count:**

| Inventory | Count | Breakdown |
|-----------|-------|-----------|
| **Committed `*.schema.json` files** | **32** | 2 common + 10 data (9 baseline + 1 optional `l5-mind` `mind-state`) + 14 ops (10 baseline + 4 optional `l2-computable` `project` / `compute`) + 6 connect (opt-in `spoke-connect`) |

Protocol wire inventory is **32** files. `schemas/README.md`, `EXPECTED_SCHEMA_COUNT`, and generated output must match in the same commit as schema changes.

Shared defs in `common.schema.json` include `Scope`, `TimelineScale`, `ForkId`, `OpaqueJson`, `ComputableFieldMap`, `ComputableLogEntry`, and `BodyAttribute`. All ops responses use `oneOf` success branch or `{ "error": ErrorEnvelope }`. Baseline integrators use **21** schema files (2 common + 9 data + 10 baseline ops); optional `l2-computable` adds four ops schemas, the opt-in `spoke-connect` family adds six connect envelopes, and the optional `l5-mind` capability adds `mind-state.schema.json` for **32** total.

Update [`schemas/README.md`](../../schemas/README.md) checklist in the same commit as schema changes.

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
| Rust (hand-written) | `spoke-operations` | — (not codegen) | `crates/spoke-operations/src/` |

`schemas/` is the only hand-authored wire truth. Generated output is committed; drift fails `verify-codegen`.

### Codegen layout (v0.1)

```text
spoke/
├── package.json                 # scripts: codegen, verify-codegen
├── pnpm-workspace.yaml          # packages: ["packages/*", "tooling/*", "fixtures/*"]
├── Cargo.toml                   # workspace; members include crates/* and fixtures/toy-world/rust (private)
├── schemas/                     # SSOT (hand-authored)
├── tooling/codegen/             # orchestrates jstt + typify (private package)
├── packages/spoke-schemas/       # @42ch/spoke-schemas (npm; stable-tag releases)
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts             # flat re-exports
│       └── generated/           # COMMITTED; mirrors schemas/ tree
│           ├── common/
│           ├── data/
│           ├── ops/
│           └── connect/
└── crates/spoke-schemas/
    ├── Cargo.toml
    └── src/
        ├── lib.rs               # pub mod generated; flat re-exports
        └── generated/           # COMMITTED; mirrors schemas/ tree
            ├── mod.rs
            ├── common/
            ├── data/
            ├── ops/
            └── connect/
└── crates/spoke-operations/
    ├── Cargo.toml
    └── src/
        ├── lib.rs               # flat re-exports; hand-written pure helpers
        └── *.rs                 # family modules (result, extensions, finding, …)
└── fixtures/toy-world/
    ├── src/adapter/             # TS ToyWorldAdapter (private; not published)
    ├── rust/                    # spoke-fixture-toy-world crate (publish = false)
    └── tests/                   # AJV/Vitest conformance + adapter orchestration tests
```

**Codegen rules:**

| Rule | Detail |
|------|--------|
| Trigger | `pnpm run codegen` from repo root |
| Verify | `pnpm run verify-codegen` → non-zero if generated tree differs from `schemas/` |
| Commit policy | Schema change + regenerated output in the **same commit** |
| Edit policy | Never hand-edit `*/generated/**` |
| Module mirror | Generated folder names mirror `schemas/{common,data,ops,connect}` |
| Public API | Both packages re-export all leaf types from `index.ts` / `lib.rs` |
| Schema inventory | **32** `*.schema.json` files under `schemas/`; `EXPECTED_SCHEMA_COUNT` in `tooling/codegen/assert-schema-count.mjs` and rust-gen must match |
| Opaque JSON fields | Wire shape: `#/definitions/OpaqueJson` (empty schema `{}`) with `$ref` from consuming properties (e.g. `ComputableLogChange.previous` / `.next`). Generators MUST emit any-JSON types (`unknown` / `OpaqueJson` in TS; `serde_json::Value` in Rust) — not object-index maps |
| Duplicate generated types | typify and jstt may emit duplicate nominal types across `common/` and `data/` modules after `$ref` dereference. Integrators import canonical types from `generated/common` (TS barrel or `spoke_schemas::generated::common` / crate root re-exports). Duplicates are generator output, not separate wire shapes |
| Release script tests | `pnpm run test:release` exercises `tooling/release/` assert/bump scripts (pure fixtures; no registry I/O). CI runs it in the `typescript` job |

Detail: [`schemas/README.md`](../../schemas/README.md).

## Repository layout (v0.1)

| Path | v0.1 expectation |
|------|------------------|
| `.mstar/specs/` | Normative protocol docs (this file + data + ops + operations detail) |
| `schemas/` | JSON Schema SSOT |
| `tooling/codegen/` | Codegen runner (not published) |
| `packages/spoke-schemas/` | Generated TypeScript |
| `packages/spoke-operations/` | Hand-written TypeScript operations library — pure helpers plus injected adapter ports/orchestration |
| `packages/spoke-connect-ts/` | TypeScript connect client — published as `@42ch/spoke-connect` |
| `crates/spoke-schemas/` | Generated Rust wire types |
| `crates/spoke-operations/` | Hand-written Rust operations library — behavioral port of TS helpers plus injected adapter traits/orchestration |
| `crates/spoke-connect/` | Rust connect reference — session core, libp2p transport, uniffi binding surface (`bindings/`); published as `spoke-connect` |
| `fixtures/toy-world/` | Protocol conformance JSON + AJV/Vitest harness (`tests/`; `@42ch/spoke-fixture-toy-world`) + reference `ToyWorldAdapter` (TS: `src/adapter/`; Rust: `rust/` crate `spoke-fixture-toy-world`, `publish = false`) |

## v0.1 acceptance (umbrella)

Current wire bar: ten data objects (including `HostCapabilityManifest`, `Rule`, `TimelineEvent`, `MindState`, `ToolDescriptor`), five baseline ops plus optional `project` / `compute`, six opt-in connect envelopes (`spoke-connect`), **32** schema files; normative vocabulary locks `KnowledgeEntry` / `TimelineEvent` in this tree and [`CONCEPTS.md`](../../CONCEPTS.md). Baseline adapters implement `HostManifestPort` per [`spoke-operations.md`](spoke-operations.md).

**CI + inventory (required):**

1. Spec trio (`spoke-protocol`, `spoke-data-model`, `spoke-ops`) aligned with `schemas/` tree for baseline data objects + five ops
2. **CI green on PR** — [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) runs on `pull_request` and on pushes to `main` / `iteration/**`; all four jobs must pass:
   - `verify-codegen` — `pnpm run verify-codegen` (schema drift fails the build)
   - `typescript` — `pnpm -F @42ch/spoke-schemas typecheck` + `build`; `@42ch/spoke-operations` typecheck + test; `pnpm run test:fixtures`; `pnpm run test:release`
   - `rust` — `cargo check -p spoke-schemas`; `cargo test -p spoke-operations`; `cargo test -p spoke-fixture-toy-world` (private fixture crate; not published)
   - `verify-version` — `pnpm run verify:version` (lockstep SemVer across manifests and README badges; see [`spoke-version-release.md`](spoke-version-release.md))
3. Same checks pass locally (`pnpm run verify-codegen`, package typecheck/build, `cargo check -p spoke-schemas`, `cargo test -p spoke-operations`, `cargo test -p spoke-fixture-toy-world`, `pnpm run verify:version`)
4. Extensions contract enforced in data schemas
5. Protocol conformance fixtures and reference Adapter examples at `fixtures/toy-world/` (`@42ch/spoke-fixture-toy-world` harness under `tests/`)

**Current data inventory (normative):**

| Object | Schema |
|--------|--------|
| KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket | `schemas/data/*.schema.json` |
| HostCapabilityManifest | `schemas/data/host-capability-manifest.schema.json` — see [`spoke-data-model.md`](spoke-data-model.md) §HostCapabilityManifest |
| Rule | `schemas/data/rule.schema.json` — see [`spoke-data-model.md`](spoke-data-model.md) |
| TimelineEvent | `schemas/data/timeline-event.schema.json` — see [`spoke-data-model.md`](spoke-data-model.md) |
| ToolDescriptor | `schemas/data/tool-descriptor.schema.json` — see [`spoke-data-model.md`](spoke-data-model.md) §Tools |

## Non-goals (v0.1)

| Out of scope | Rationale |
|--------------|-----------|
| Product ↔ SPOKE binding packages in this repo | Reference `ToyWorldAdapter` in `fixtures/toy-world/`; consumer-repo product bindings when scheduled |
| Required WASM / compute engines in protocol | Optional `l2-computable` shapes I/O only — engines are product-owned |
| Fork merge / rebase engines and world-history stores | Product-owned — protocol documents interchange fields only (`fork_id`, `parent_fork_id`, `Scope.fork_id`) |
| Shared runtime, daemon, or MCP server | Protocol repo only |
| Ad-hoc or unversioned registry publish | Registry publish happens only from stable tags via the top-level `release.yml` (Trusted Publishing); fixture and codegen packages remain private |

## Roadmap pointer

| Phase | Deliverable |
|-------|-------------|
| **v0.1 (delivered)** | Data + ops **wire** SSOT, `@42ch/spoke-schemas` / `spoke-schemas`, codegen packages, CI verify gate |
| **Operations library first slice** | Hand-written `@42ch/spoke-operations` (column 3) + integrator README EN/CN — see [`spoke-operations.md`](spoke-operations.md) |
| **Protocol layers + Rule/TimelineEvent** | Normative L0–L8 + capability levels; `Rule` + `TimelineEvent` field semantics; ops harden (Scope neutrality, Check≠Assemble, error-envelope R3) |
| **Operations library deepen + fixtures** | Deepen `@42ch/spoke-operations` helpers + `fixtures/toy-world/` conformance graph; AJV/Vitest harness at `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`) — **no adapters** |
| **Rust operations library** | Hand-written `spoke-operations` crate — behavioral port of TS package at lockstep SemVer — see [`spoke-operations.md`](spoke-operations.md) |
| **Adapter aliases (delivered)** | `*Adapter` composed-port aliases in `@42ch/spoke-operations` and `spoke-operations`; integrator path via operations packages + `fixtures/toy-world/` reference examples — see [`spoke-operations.md`](spoke-operations.md#adapter-aliases-normative) § Adapter aliases |
| **Host collaboration (normative)** | `HostCapabilityManifest` wire + baseline-required `HostManifestPort`; five host roles; namespace exclusivity — see [`spoke-data-model.md`](spoke-data-model.md) §HostCapabilityManifest |
| **Next** | `ToyWorldAdapter` multi-host manifest proof; then product adapter packages in consumer repos (product DTO ↔ SPOKE) |
| **North star** | Cross-product narrative **KnowledgeEntry** dialect for consistency-check and context-assembly I/O on a shared protocol surface |

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer, annotated tags, CI-gated GitHub Release and registry publish |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Nine layers L0–L8, capability levels, Domain Profile, layer ↔ artifact map |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects, extensions, open vocabulary, Rule/TimelineEvent |
| [`spoke-ops.md`](spoke-ops.md) | Five ops, error envelope, Scope neutrality, `assemble` wire-only boundary |
| [`spoke-operations.md`](spoke-operations.md) | Operations behavior library — pure helpers; [adapter interfaces](spoke-operations.md#adapter-interfaces-normative); [injection orchestration](spoke-operations.md#injection-orchestration-normative) |
| [`spoke-connect.md`](spoke-connect.md) | Connect envelope family — session ordering, auth model, discovery boundary (opt-in `spoke-connect`) |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Narrative-structure Domain Profile — Beat mapping, `precedes`, `structural_role` |
| [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) | Lore-activation Domain Profile — `modules.activation` (capability-flagged) |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist (32 files committed) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | KnowledgeEntry / TimelineEvent vocabulary; dual-concern rule |
| [`STRATEGY.md`](../../STRATEGY.md) | Protocol-not-runtime positioning and v0.1 scope |
