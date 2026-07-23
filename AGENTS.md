# SPOKE — AGENTS.md

**SPOKE** = Standardized Programmable Ontology Keyblock Engine.

Greenfield protocol repository: JSON Schema SSOT for narrative Keyblock **data** and **ops** wire shapes, with generated TypeScript (`@42ch/spoke-schema`) and Rust (`spoke-schema`) packages.

Normative entry: [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md).

## Harness

Morning Star consumer. Harness SSOT: [`.mstar/AGENTS.md`](.mstar/AGENTS.md).

**Process vs results:** process paths under `.mstar/` (plans, iterations, status, notes, sdd, archived) are gitignored; shared results are `.mstar/specs/` and `.mstar/knowledge/`. See harness AGENTS for the full table.

Do not put plan progress or residual detail in this file.

## Tech direction (v0.1)

- **SSOT:** `schemas/`
- **Codegen:** `json-schema-to-typescript` + `typify`
- **Extensions:** `extensions.<namespace>` only; core fields closed
- **Adapters:** `adapters/*` empty placeholders until a later iteration

## Conflict priority

1. Current user instruction  
2. This file  
3. `.mstar/AGENTS.md`  
4. `mstar-*` skills  
