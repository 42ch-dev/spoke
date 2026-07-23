# Ops response error-envelope oneOf

**Category:** architecture-patterns  
**Source:** compound 2026-07-23 (ops-harden)  
**Status:** durable

## Problem

Ops success responses and failure envelopes must not coexist. Partial patterns (error only on assemble) cause integrator drift (residual R3).

## Decision

Every ops `*-response.schema.json` uses JSON Schema `oneOf`:

1. Success branch — op-specific required fields; `additionalProperties: false`
2. Failure branch — `{ "error": ErrorEnvelope }` only

`upsert.rejected[]` stays on the **success** branch (per-item rejects ≠ transport failure).

Codegen note: titled schemas that are `oneOf` + sibling `definitions` must export `export type { Title }` only in the TS barrel — `export *` duplicates inlined defs.

## Related

- Spec: `.mstar/specs/spoke-ops.md`
- Schemas: `schemas/ops/*-response.schema.json`
- Codegen: `tooling/codegen/src/run.mjs`
