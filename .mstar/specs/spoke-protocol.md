# SPOKE Protocol (umbrella)

> **Status:** Normative (v0.1)  
> **Document class:** Master  
> **Owns:** Cross-cutting protocol framing for data + ops layers

## Problem & user value

Story-AI products each invent local shapes for knowledge units, checker I/O, and context assembly. SPOKE provides a **shared wire dialect** so products can exchange Keyblock data and ops without sharing a runtime, database, or daemon.

**v0.1 delivers:** schema SSOT, generated language packages, and normative docs — not working adapters or conformance tests.

## Three columns (Thrust A)

SPOKE Thrust A spans **data wire**, **ops wire**, and a **hand-written operations behavior library** — see [`.mstar/roadmap.md`](../roadmap.md). v0.1 delivered columns 1–2; v0-iter002 delivered column 3.

| Column | Responsibility | Normative doc | Artifact home |
|--------|----------------|---------------|---------------|
| **1. Data** | Seven required objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket, **Rule**, **Event** | [`spoke-data-model.md`](spoke-data-model.md) | `schemas/data/`, `schemas/common/` |
| **2. Ops wire** | Five operations (10 request/response schemas): upsert, extract→promote, relate, check, assemble | [`spoke-ops.md`](spoke-ops.md) | `schemas/ops/` |
| **3. Ops library** | Pure lifecycle invariants JSON Schema cannot express (promote gate, Finding transitions, extensions preserve, AssemblePacket builders) | [`spoke-operations.md`](spoke-operations.md) | `packages/spoke-operations/` (`@42ch/spoke-operations`) |

**Invariant:** generated `@42ch/spoke-schemas` types are wire truth; `@42ch/spoke-operations` is hand-written behavior on those types — not a third runtime, daemon, or transport binding.

**v0-iter003 data deepen (architect-locked target wire):** `Rule` (L6) and `Event` (L5) field tables are normative in [`spoke-data-model.md`](spoke-data-model.md); committed `rule.schema.json` / `event.schema.json` land in sibling plan **`rule-event`**. Shared `Scope` + `TimelineScale` defs are architect-locked in `common.schema.json` (no extra common files) — committed wire lands in sibling plan **`ops-harden`**. **Committed today:** 17 schema files (v0.1 baseline).

## Nine-layer model (L0–L8)

Normative chapter: [`spoke-protocol-layers.md`](spoke-protocol-layers.md) (v0-iter003). Integrators declare **baseline** (`spoke-baseline`) vs optional **`l2-computable`** / **`l5-fork`** capability flags. L5 Timeline projection tiers use wire vocabulary **`brief` / `narrative` / `moment`** via optional `timeline_scale` — distinct from L8 **`AssemblePacket`** context assembly (see layers spec §L5 rule 4: L5 `moment` tier ≠ L8 `assemble` op).

**Schema file count:**

| Slice | Hand-authored files | Breakdown |
|-------|---------------------|-----------|
| **Committed (today)** | **17** | 2 common + 5 data + 10 ops — v0.1 baseline |
| **Post–`rule-event` + `ops-harden` target** | **19** | + `data/rule.schema.json`, `data/event.schema.json` (`rule-event` plan); `common.schema.json` gains `Scope` + `TimelineScale` defs (`ops-harden` plan; still 2 common files) |

Update [`schemas/README.md`](../../schemas/README.md) checklist in the same commit as schema land.

## Extensions

Every durable data object MUST include:

```json
"extensions": { "<namespace>": { } }
```

| Rule | Requirement |
|------|-------------|
| Namespace keys | Product ids (`nexus`, `creader`, …) |
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
├── pnpm-workspace.yaml          # packages: ["packages/*", "tooling/*"]
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
| `packages/spoke-operations/` | Hand-written operations library (delivered v0-iter002) |
| `crates/spoke-schemas/` | Generated Rust |
| `adapters/` | README purpose note only; product adapter packages deferred |

## v0.1 acceptance (umbrella)

Historical v0.1 close criteria (wire bootstrap). v0-iter002 delivered column 3 — see [`spoke-operations.md`](spoke-operations.md) acceptance section.

1. Spec trio (`spoke-protocol`, `spoke-data-model`, `spoke-ops`) aligned with `schemas/` tree (5 data objects + 5 ops; `Rule` excluded)
2. **CI green on PR** — [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) runs on `pull_request` and on pushes to `main` / `iteration/**`; all three jobs must pass:
   - `verify-codegen` — `pnpm run verify-codegen` (schema drift fails the build)
   - `typescript` — `pnpm -F @42ch/spoke-schemas typecheck` + `build`; `@42ch/spoke-operations` typecheck + test
   - `rust` — `cargo check -p spoke-schemas`
3. Same checks pass locally (`pnpm run verify-codegen`, package typecheck/build, `cargo check -p spoke-schemas`)
4. Extensions contract enforced in data schemas
5. `Rule` deferral documented; no orphan `rule.schema.json`
6. No adapter packages or `fixtures/` yet (`adapters/README.md` only)

## Non-goals (v0.1)

| Out of scope | Rationale |
|--------------|-----------|
| Real Nexus ↔ SPOKE or Creader ↔ SPOKE conversion | Adapter packages deferred to next iteration |
| Conformance fixtures / golden toy-world round-trips | No `fixtures/` this iteration |
| `Rule` wire schema | Deferred in v0.1 — superseded by v0-iter003 (see data model) |
| WASM / Computable Keyblock / Fork semantics | Not required protocol surface yet |
| Shared runtime, daemon, or MCP server | Protocol repo only |
| npm/crates.io publish (including from CI) | Workspace-local packages suffice for v0.1 |

## Roadmap pointer

| Phase | Deliverable |
|-------|-------------|
| **v0.1 (delivered)** | Data + ops **wire** SSOT, `@42ch/spoke-schemas` / `spoke-schemas`, empty adapter dirs, CI gate |
| **v0-iter002 (delivered 2026-07-23)** | Hand-written `@42ch/spoke-operations` (column 3) + integrator README EN/CN — see [`spoke-operations.md`](spoke-operations.md) |
| **v0-iter003 (in progress)** | Normative L0–L8 + capability levels; `Rule` + `Event` field semantics; ops harden spec (Scope neutrality, Check≠Assemble, error-envelope R3). Committed schemas: sibling plans **`rule-event`** + **`ops-harden`** — **no adapters** |
| **Next** | Implementable adapter packages (product DTO ↔ SPOKE), conformance fixtures |
| **North star** | Cross-product narrative Keyblock dialect for consistency-check and context-assembly I/O **without** a shared runtime |

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Nine layers L0–L8, capability levels, Domain Profile, layer ↔ artifact map |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects, extensions, open vocabulary, Rule/Event (v0-iter003) |
| [`spoke-ops.md`](spoke-ops.md) | Five ops, error envelope, Scope neutrality, `assemble` wire-only boundary |
| [`spoke-operations.md`](spoke-operations.md) | Operations behavior library — `SpokeResult`, four helper families, hard In/Out |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist (committed 17; post–`rule-event` + `ops-harden` target 19) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Keyblock vocabulary; Keyblock ≠ World KB ≠ Author Memory |
| [`STRATEGY.md`](../../STRATEGY.md) | Protocol-not-runtime positioning and v0.1 scope |
| [`delivery-compass.md`](../iterations/v0.1/delivery-compass.md) | v0.1 iteration close checklist (process artifact; optional) |
