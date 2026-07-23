# SPOKE — AGENTS.md

**SPOKE** = Standardized Programmable Ontology Keyblock Engine.

Greenfield protocol repository: JSON Schema SSOT for narrative Keyblock **data** and **ops** wire shapes, with generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) packages.

Normative entry: [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md).

## Harness

Morning Star consumer. Harness SSOT: [`.mstar/AGENTS.md`](.mstar/AGENTS.md).

**Process vs results:** process paths under `.mstar/` (plans, iterations, status, notes, sdd, archived) are gitignored; shared results are `.mstar/specs/` and `.mstar/knowledge/`. See harness AGENTS for the full table.

Do not put plan progress or residual detail in this file.

## README audience (HARD)

Root `README.md` / `README_CN.md` are **for humans** (protocol consumers / integrators), not for agent steering.

| In README | Out of README (agent / harness only — this file or `.mstar/`) |
|-----------|----------------------------------------------------------------|
| What the protocol and packages **do** | Iteration IDs, ship banners, delivery/changelog narrative |
| How to install, consume, contribute | Anti-pattern / boundary rhetoric (“not a runtime”, “does not include…”, “out of scope…”, “never…”) |
| Positive capability list | In/Out or “library does not…” tables meant to constrain agents |

**Rule:** describe product state affirmatively. Negation, exclusion lists, and “do not confuse X with Y” prose belong here (or in specs for normative invariants) — **never** in consumer READMEs. Keep the EN/CN twin outline.

### Boundaries agents must enforce (not README copy)

- SPOKE is a **protocol repo**, not a product runtime, daemon, or shared database.
- `adapters/` holds **README purpose text only** for now — no product subdirs, packages, or mapping code until an iteration schedules them.
- Core interchange owns wire shapes only — world history, fork semantics, checker engines, ranking, and retrieval stay in products.
- `@42ch/spoke-operations` is pure: no I/O, storage, LLM, HTTP, MCP, ranking, retrieval, or silent auto-promote.
- Packages are workspace-private (not published to npm); consume via workspace or `file:` path.
- Finding is checker output, not Keyblock `body`.

## Tech direction (v0.1)

- **SSOT:** `schemas/`
- **Codegen:** `json-schema-to-typescript` + `typify`
- **TS package:** `@42ch/spoke-schemas` → `packages/spoke-schemas/`
- **Rust crate:** `spoke-schemas` → `crates/spoke-schemas/`
- **Extensions:** `extensions.<namespace>` only; core fields closed
- **Adapters:** deferred — `adapters/README.md` only until scheduled

## Conflict priority

1. Current user instruction  
2. This file  
3. `.mstar/AGENTS.md`  
4. `mstar-*` skills  
