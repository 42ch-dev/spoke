# SPOKE Protocol (umbrella)

> **Status:** Draft (iteration v0.1)  
> **Document class:** Master  
> **Owns:** Cross-cutting protocol framing for data + ops layers

## Problem & user value

Story-AI products each invent local shapes for knowledge units, checker I/O, and context assembly. SPOKE provides a **shared wire dialect** so products can exchange Keyblock data and ops without sharing a runtime, database, or daemon.

**v0.1 delivers:** schema SSOT, generated language packages, and normative docs — not working adapters or conformance tests.

## Two layers

| Layer | Responsibility | Normative doc | Schema home |
|-------|----------------|---------------|-------------|
| **Data** | Five required objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket | [`spoke-data-model.md`](spoke-data-model.md) | `schemas/data/`, `schemas/common/` |
| **Ops** | Five operations (10 request/response schemas): upsert, extract→promote, relate, check, assemble | [`spoke-ops.md`](spoke-ops.md) | `schemas/ops/` |

**Deferred data object:** `Rule` — see [`spoke-data-model.md` §Rule deferral](spoke-data-model.md#rule-deferral-v01-decision). No `rule.schema.json` in v0.1.

**Schema file count (v0.1):** 17 hand-authored files (2 common + 5 data + 10 ops) — checklist in [`schemas/README.md`](../schemas/README.md).

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
| TypeScript | `@42ch/spoke-schema` | `json-schema-to-typescript` | `packages/spoke-schema/src/generated/` |
| Rust | `spoke-schema` | `typify` | `crates/spoke-schema/src/generated/` |

`schemas/` is the only hand-authored wire truth. Generated output is committed; drift fails `verify-codegen`.

### Codegen layout (v0.1)

```text
spoke/
├── package.json                 # scripts: codegen, verify-codegen
├── pnpm-workspace.yaml          # packages: ["packages/*", "tooling/*"]
├── Cargo.toml                   # workspace; members = ["crates/spoke-schema"]
├── schemas/                     # SSOT (hand-authored)
├── tooling/codegen/             # orchestrates jstt + typify (private package)
├── packages/spoke-schema/       # @42ch/spoke-schema (published path TBD)
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts             # flat re-exports
│       └── generated/           # COMMITTED; mirrors schemas/ tree
│           ├── common/
│           ├── data/
│           └── ops/
└── crates/spoke-schema/
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

Detail: [`schemas/README.md`](../schemas/README.md).

## Repository layout (v0.1)

| Path | v0.1 expectation |
|------|------------------|
| `specs/` | Normative protocol docs (this file + data + ops detail) |
| `schemas/` | JSON Schema SSOT |
| `tooling/codegen/` | Codegen runner (not published) |
| `packages/spoke-schema/` | Generated TypeScript |
| `crates/spoke-schema/` | Generated Rust |
| `adapters/nexus/`, `adapters/creader/` | Empty placeholders (`.gitkeep` only) |

## v0.1 acceptance (umbrella)

Executable checks for iteration close — detail in [delivery compass](../.mstar/iterations/v0.1/delivery-compass.md):

1. Spec trio (`spoke-protocol`, `spoke-data-model`, `spoke-ops`) aligned with `schemas/` tree (5 data objects + 5 ops; `Rule` excluded)
2. Codegen + verify-codegen green on CI
3. Extensions contract enforced in data schemas
4. `Rule` deferral documented; no orphan `rule.schema.json`
5. Adapter dirs placeholder-only; no `fixtures/`

## Non-goals (v0.1)

| Out of scope | Rationale |
|--------------|-----------|
| Real Nexus ↔ SPOKE or Creader ↔ SPOKE conversion | Adapter packages deferred to next iteration |
| Conformance fixtures / golden toy-world round-trips | No `fixtures/` this iteration |
| `Rule` wire schema | Deferred — see data model §Rule deferral |
| WASM / Computable Keyblock / Fork semantics | Not required protocol surface yet |
| Shared runtime, daemon, or MCP server | Protocol repo only |
| npm/crates.io publish | Workspace-local packages suffice for v0.1 |

## Roadmap pointer

- **v0.1 (now):** Protocol bootstrap — schemas, codegen, language packages, empty adapter dirs
- **Next iteration:** Implementable adapter packages (product object ↔ SPOKE), optional `Rule` schema, conformance fixtures
- **North star:** Cross-product narrative Keyblock dialect for consistency-check and context-assembly I/O

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-data-model.md`](spoke-data-model.md) | Five data objects, extensions, open vocabulary, `Rule` deferral |
| [`spoke-ops.md`](spoke-ops.md) | Five ops, error envelope, `assemble` wire-only boundary |
| [`schemas/README.md`](../schemas/README.md) | 17-file schema tree and authoring rules |
| [`delivery-compass.md`](../.mstar/iterations/v0.1/delivery-compass.md) | v0.1 acceptance criteria |
