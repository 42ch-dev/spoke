# HostCapabilityManifest collaboration

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-27 (host capability collaboration)  
> **Packages:** `@42ch/spoke-schemas` / `spoke-schemas` (wire); `@42ch/spoke-operations` / `spoke-operations` (port); reference fixtures under `fixtures/toy-world/`

## Problem

In-process multi-adapter composition needs a machine-valid way for each host to advertise **who it is**, **which roles it plays**, **which capability flags it supports**, and **which `extensions` namespaces it owns** — without stuffing that metadata into product `KnowledgeEntry.extensions.<ns>` bags or inventing a shared runtime.

## Decision

1. **First-class wire object** — `HostCapabilityManifest` is a peer data object to KnowledgeEntry (schema under `schemas/data/`). Required fields: `schema_version`, `host_id`, `roles`, `capabilities`, `namespaces`, `extensions` (`ExtensionMap`, may be `{}`). Optional closed `authority` with required `scope_key` when present.
2. **Open role / capability strings** — `roles` and `capabilities` are open `string[]` (min 1, unique). Core role vocabulary: `data-store`, `input-source`, `checker`, `assembler`, `computable-engine`. Capability flags reuse existing strings (`spoke-baseline`, `l2-computable`, `l5-fork`). Baseline adapters include `spoke-baseline`; `computable-engine` ∈ `roles` pairs with `l2-computable` ∈ `capabilities`.
3. **Five-role map (closed-loop)** — `data-store` owns OCC write-back via `KnowledgeEntryPort` / upsert-promote; `checker` emits findings via `RuleQueryPort` / `FindingPort` / `orchestrateCheck`; **`assembler`** aggregates scoped knowledge via `ScopeQueryPort` / `orchestrateAssemble` (closed-loop core, not optional vocabulary); `input-source` is product-defined ingest; `computable-engine` is optional behind `l2-computable`.
4. **Namespace exclusivity** — `namespaces[]` lists namespaces this `host_id` owns in one collaboration context; each `ns` appears on at most one manifest. KE `extensions.<ns>` remain product bags; host roles/capabilities/namespaces are **core manifest fields**.
5. **Baseline-required port** — `HostManifestPort` (`getHostCapabilityManifest` / `listPeerHostCapabilityManifests`) is the sixth required `BaselinePorts` family. Peer list is product/adapter memory: exclude self, dedupe by `host_id`, UTF-8 ascending sort, `[]` valid. Existing `orchestrate*` entrypoints do not auto-fetch manifests.
6. **Reference proof** — `fixtures/toy-world/` ships ≥2 AJV-valid manifests with disjoint namespaces; `ToyWorldAdapter` (TS + Rust) returns self + in-memory peer.

## Gotchas

- Growing `BaselinePorts` is an additive break for external implementers (pre-1.0). In-repo reference adapters and CI fixture crates must implement the new family in the **same** change set, or `spoke-fixture-toy-world` fails to compile while CI still runs it.
- Peer sorting must use **UTF-8 byte order** (`str::cmp` / `Buffer.compare`), not locale-sensitive string compare.
- Manifest `extensions` is deployment metadata only — do not duplicate `roles` / `capabilities` / `namespaces` inside it.
- When `data-store` ∈ `roles` and `authority` is omitted, integrators treat this manifest's `host_id` as the implicit write authority for its collaboration scope.

## See also

- `.mstar/specs/spoke-data-model.md` — §HostCapabilityManifest
- `.mstar/specs/spoke-operations.md` — §Host collaboration / `HostManifestPort`
- `architecture-patterns/adapter-injection-orchestration.md` — ports + `BaselinePorts` composition
- `architecture-patterns/spoke-codegen-pipeline.md` — schema inventory + verify-codegen
- `fixtures/toy-world/` — `host_tw_primary.json`, `host_tw_peer.json`, `ToyWorldAdapter`

## Adopter path

1. Author a valid `HostCapabilityManifest` (required `namespaces[]`, baseline `spoke-baseline` when claiming baseline).
2. Implement `HostManifestPort` on the same adapter type that satisfies `BaselinePorts` / `BaselineAdapter`.
3. Supply peer manifests from product memory (static fixtures or a registry); enforce exclusive `namespaces[]` across the collaboration context.
4. Call the port explicitly for discovery; keep OCC writes on the `data-store` authority only.
