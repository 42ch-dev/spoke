# Rust spoke-operations behavioral parity

**Category:** architecture-patterns  
**Source:** compound 2026-07-25 (rust-ops-parity)  
**Status:** durable

## Problem

Rust integrators already pin `spoke-schemas` on crates.io but had no published pure helper crate. Re-implementing lifecycle gates in each product duplicates reject vocabulary and drifts from the TypeScript behavioral SSOT.

## Decision

1. **Crate location** — hand-authored `crates/spoke-operations/`; workspace member; depends on `spoke-schemas` only for wire types (+ serde stack). Public `lib.rs` flat re-exports mirror `@42ch/spoke-operations` `index.ts` (Rust snake_case identifiers; same behavioral contracts).
2. **`SpokeResult` idiom** — dedicated `enum SpokeResult<T> { Ok(T), Reject(SpokeReject) }` with `spoke_ok` / `spoke_reject` constructors. **Not** `std::result::Result`. Expected lifecycle rejects never panic.
3. **Reject-code parity** — single `result` module exports all stable `SpokeRejectCode` values; `as_str()` returns **identical** wire literals as TypeScript `SpokeRejectCode` (e.g. `CANDIDATE_NOT_PROVISIONAL`, `REVISION_CONFLICT`). Error-envelope mapping uses code string only.
4. **typify wire helpers** — closed L2 `body` declares `summary`, `tags`, `attributes`, and optional `state` / `computable` maps; `ComputableFieldMap` values remain open JSON objects. Assemble and similar paths MUST preserve raw `body` wire JSON when helpers read fields typify does not round-trip faithfully:
   - Integrators deserializing from wire: `KnowledgeEntryForAssemble::from_wire_json(wire)` — extracts `body` via `body_wire_from_entry_wire` before typify deserialize.
   - Programmatic construction: `KnowledgeEntryForAssemble::from_entry(entry)` — serializes known body fields to JSON for reads.
   - Private `util` module: field access only (`extract_snippet_from_body_wire`, `validate_revision_wire` with `None` → `0` for OCC); **no** parallel wire DTO structs.
   - Trait reads: `list_body_attributes` / `filter_body_attributes_by_trait_type` / `find_body_attribute` in `body.rs` — skip malformed items; never panic on missing `attributes`.
5. **crates.io dependency pin** — `spoke-operations` manifest MUST declare `spoke-schemas` with **both** `version` and `path`:

   ```toml
   spoke-schemas = { version = "X.Y.Z", path = "../spoke-schemas" }
   ```

   Path-only fails `cargo publish` on crates.io. Lockstep bump rewrites the `version` key via `formatOpsSpokeSchemasDependency` in `tooling/release/lockstep-surfaces.mjs`.
6. **Publish order** — stable tags: `cargo publish -p spoke-schemas` then `cargo publish -p spoke-operations` (after npm publish jobs). `-rc.` tags skip registry publish.

## What not to do

- Do not invent a second reject-code vocabulary in Rust.
- Do not deserialize assemble inputs through typify alone when helpers read `body.summary`, `ComputableFieldMap` domain keys, or other fields that require full wire JSON.
- Do not add I/O, storage, HTTP, LLM, ranking, retrieval, or fixture harness inside the crate.
- Do not use path-only `spoke-schemas` dependency in the ops crate manifest.

## Related

- Normative: `.mstar/specs/spoke-operations.md`
- TS + Rust pure-actions pattern: `architecture-patterns/spoke-operations-pure-actions.md`
- Adapter ports + injection orchestration: `architecture-patterns/adapter-injection-orchestration.md`
- Adapter modules: `crates/spoke-operations/src/adapter.rs` (`ports` / `orchestrate`) with OQ-4 parity export checklist test
- Lockstep + publish: `architecture-patterns/lockstep-semver-release.md`
- Crate README: `crates/spoke-operations/README.md`
- CI verify: `cargo test -p spoke-operations` in `.github/workflows/ci.yml` and `release.yml`
