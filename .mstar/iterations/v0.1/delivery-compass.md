---
iteration_id: v0.1
start_date: 2026-07-23
status: locked
iteration_base_branch: main
target_branch: main
plans:
  - 2026-07-23-v0.1-spoke-core-bootstrap
---

# v0.1 Delivery Compass

## Scope

Bootstrap the SPOKE (Standardized Programmable Ontology Keyblock Engine) repository as a **protocol SSOT** with two standardized layers:

1. **Data standardization** — five required Keyblock wire objects (Keyblock, Relation, SourceAnchor, Finding, AssemblePacket) with an explicit `extensions` namespace bag for product-specific payloads (no information loss across adapters). **`Rule` wire object deferred** to next iteration (see [`specs/spoke-data-model.md` §Rule deferral](../../../specs/spoke-data-model.md#rule-deferral-v01-decision)).
2. **Behavioral / operations standardization** — five transport-agnostic ops (`upsert`, extract→promote, `relate`, `check`, `assemble`; 10 request/response schemas). `assemble` is wire-only — compute optional / out of scope for v0.1 implementation.

Deliver generated language packages from JSON Schema SSOT:

- TypeScript: `@42ch/spoke-schema`
- Rust: `spoke-schema`

Adapter trees are **empty directory placeholders only** this iteration (`adapters/nexus/`, `adapters/creader/`).

Direction locked from cross-product research (Nexus **KeyBlock** + Creader KB/Guardian — product spellings) and interactive grill ([`guides/grill-lock-2026-07-23.md`](guides/grill-lock-2026-07-23.md), 2026-07-23). SPOKE protocol spelling is **Keyblock**.

## Plans

| plan_id | Name | Status | Notes |
|---------|------|--------|-------|
| 2026-07-23-v0.1-spoke-core-bootstrap | SPOKE core protocol bootstrap | Todo | Single plan: specs + schemas + codegen + packages + empty adapter dirs |

## Milestones

| Milestone | Target date | Status | Exit signal |
|-----------|-------------|--------|---------------|
| Spec freeze (data + ops + extensions) | 2026-07-24 | pending | Spec trio reviewed; 5 data objects + 5 ops + 17 schema files match planned `schemas/` tree (`Rule` excluded) |
| Codegen + packages green | 2026-07-25 | pending | `pnpm run codegen` + `verify-codegen` pass on clean tree |
| QC / QA complete | 2026-07-26 | pending | Plan tasks done; no open HIGH residuals |
| Iteration close | 2026-07-26 | pending | Squash-merge `iteration/v0.1` → `main` |

## Acceptance Criteria

Each item is verifiable at iteration close:

- [ ] **Spec trio** — [`specs/spoke-protocol.md`](../../../specs/spoke-protocol.md), [`specs/spoke-data-model.md`](../../../specs/spoke-data-model.md), and [`specs/spoke-ops.md`](../../../specs/spoke-ops.md) exist; five data objects and five ops enumerated in specs match the `schemas/data/` and `schemas/ops/` file set (`Rule` excluded)
- [ ] **Extensions contract** — every data envelope schema uses `additionalProperties: false` on protocol fields and declares `extensions` as a **required** object keyed by product namespace; round-trip preservation rules stated in specs
- [ ] **Rule deferral** — no `schemas/data/rule.schema.json`; deferral documented in [data model spec](../../../specs/spoke-data-model.md#rule-deferral-v01-decision)
- [ ] **Assemble boundary** — ops spec states wire-only scope; no compute/ranking fields in protocol schemas
- [ ] **Schema SSOT** — `schemas/` is draft-07 compatible; `schemas/README.md` documents layout; each required data object and op has a schema file
- [ ] **Codegen** — `pnpm run codegen` generates `@42ch/spoke-schema` (TS) and `spoke-schema` (Rust) via `json-schema-to-typescript` + `typify`; outputs committed alongside schemas
- [ ] **Verify gate** — `pnpm run verify-codegen` exits 0 on a clean working tree and non-zero when generated output drifts from `schemas/`
- [ ] **Adapter placeholders** — `adapters/nexus/.gitkeep` and `adapters/creader/.gitkeep` exist; `adapters/README.md` states deferred implementation; **no** conversion code under `adapters/`
- [ ] **No fixtures** — no `fixtures/` directory, golden files, or conformance harness committed
- [ ] **Greenfield docs** — `CONCEPTS.md` and `STRATEGY.md` exist with v0.1 stubs (Keyblock vocabulary, protocol-not-runtime positioning); root `README.md` explains SPOKE in ≤15 lines

## Non-Goals

Explicitly excluded from v0.1 — if present in the tree, iteration fails acceptance:

| Non-goal | v0.1 boundary |
|----------|---------------|
| Real Nexus ↔ SPOKE or Creader ↔ SPOKE conversion | No adapter package code; empty dirs only |
| Adapter dependency packages | Directories + README only |
| Conformance fixtures / golden round-trips | No `fixtures/`; no harness |
| `assemble` compute / ranking / retrieval | Wire shape only; algorithms product-local |
| WASM / Computable Keyblock / Fork semantics | Not required protocol fields |
| Shared runtime, daemon, or MCP server | Protocol repo only |
| npm/crates.io publish | Workspace-local packages |

## Roadmap Position

| Phase | Deliverable | Trigger to start | Owner |
|-------|-------------|------------------|-------|
| **v0.1 (current)** | Spec trio + `schemas/` SSOT + codegen packages + empty `adapters/*` | Grill lock 2026-07-23 | Protocol maintainers |
| **Next iteration** | Implementable adapter packages (`nexus`, `creader`) mapping product objects ↔ SPOKE; optional conformance fixtures | v0.1 packages consumable in product repos; adapter specs drafted | Protocol maintainers + product teams |
| **North star** | Cross-product narrative Keyblock dialect for consistency-check and context-assembly I/O without shared runtime | Adapters + fixtures prove round-trip | Ecosystem |

## Delivery Branch Policy

| Field | Value |
|-------|-------|
| `iteration_base_branch` | `main` |
| `spec_integration_branch` | `iteration/v0.1` |
| `target_branch` | `main` |

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Schema enum drift vs Nexus/Creader | Med | Med | Open string types + documented core vocabulary; map in later adapter specs |
| Codegen tool impedance (jstt vs typify) | Med | Low | Keep schemas draft-07-friendly; CI verify both outputs |
| Over-scoping adapters | Low | High | Non-goal enforced: empty dirs only; plan Task 4 is hygiene-only |
| Spec/schema drift | Med | Med | Acceptance requires spec object/op lists match `schemas/` tree |

## Iteration package

| Path | Purpose |
|------|---------|
| [`delivery-compass.md`](delivery-compass.md) | Iteration SSOT |
| [`guides/`](guides/) | Grill decisions, research pointers |
| Repo-root [`specs/`](../../../specs/) | Durable normative protocol specs (promoted from iteration drafts when ready) |

## Grill decisions (direction lock)

| Decision | Locked value |
|----------|--------------|
| Branch policy | base=`main`, integration=`iteration/v0.1`, target=`main` |
| Plan count | Single plan |
| Adapters | Empty directories only |
| Package name | `spoke-schema` (`@42ch/spoke-schema` / crate `spoke-schema`) |
| Extension slot | Explicit `extensions` map keyed by product namespace |
| Codegen | `json-schema-to-typescript` + `typify` |
| Fixtures | Out of scope |

## Quality Gate Summary

> Filled at iteration-close.

| plan_id | QC decision | QA gate | Residuals | Durable summary |
|---------|-------------|---------|-----------|-----------------|
| 2026-07-23-v0.1-spoke-core-bootstrap | N/A | N/A | none | TBD |

## Compound Round Summary

> Filled at iteration-close.

- 结晶文档数：TBD
- 新增 CONCEPTS.md 条目：TBD
- 触发 compound-refresh：TBD

## Iteration Retrospective (minimal)

> Filled at iteration-close.

- 做得好的：
- 可改进的：
- 下迭代建议：
