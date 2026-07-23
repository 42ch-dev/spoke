# `schemas/` — JSON Schema SSOT

SPOKE wire contracts. **Hand-authored only** — TypeScript and Rust types are generated into `packages/spoke-schemas/` and `crates/spoke-schemas/`.

Normative docs: [`spoke-protocol.md`](../.mstar/specs/spoke-protocol.md) (umbrella), [`spoke-protocol-layers.md`](../.mstar/specs/spoke-protocol-layers.md) (L0–L8), [`spoke-data-model.md`](../.mstar/specs/spoke-data-model.md) (data layer), [`spoke-ops.md`](../.mstar/specs/spoke-ops.md) (ops layer).

## Layout (v0.1)

```text
schemas/
├── README.md                           # this file
├── common/
│   ├── common.schema.json              # SchemaVersion, ExtensionMap, shared ids
│   └── error-envelope.schema.json      # shared ops error shape
├── data/
│   ├── keyblock.schema.json
│   ├── relation.schema.json
│   ├── source-anchor.schema.json
│   ├── finding.schema.json
│   └── assemble-packet.schema.json
└── ops/
    ├── upsert-request.schema.json
    ├── upsert-response.schema.json
    ├── promote-request.schema.json     # extract→promote
    ├── promote-response.schema.json
    ├── relate-request.schema.json
    ├── relate-response.schema.json
    ├── check-request.schema.json
    ├── check-response.schema.json
    ├── assemble-request.schema.json
    └── assemble-response.schema.json
```

**Explicitly absent in v0.1:** `schemas/data/rule.schema.json`, `schemas/data/event.schema.json` (see [data model §Rule deferral (superseded)](../.mstar/specs/spoke-data-model.md#rule-deferral-v01-decision--superseded)).

**v0-iter003 target:** + `rule.schema.json`, `event.schema.json`; `common.schema.json` gains `Scope` + `TimelineScale` definitions (19 files total — see [`spoke-protocol.md`](../.mstar/specs/spoke-protocol.md)).

## Naming conventions

| Rule | Example |
|------|---------|
| Kebab-case filenames | `assemble-packet.schema.json` |
| Suffix | `.schema.json` |
| One primary type per file | `title` matches generated type name |
| Ops pairing | `<op>-request.schema.json` + `<op>-response.schema.json` |
| Promote op basename | `promote-*` (not `extract-promote-*`) |

## `$id` / `$ref`

- Base URI: `https://spoke42.invalid/schemas/`
- Example: `"$id": "https://spoke42.invalid/schemas/data/keyblock.schema.json"`
- Cross-file refs use absolute spoke42.invalid URIs (codegen resolves consistently)

## Authoring rules

1. **Draft-07** — `"$schema": "http://json-schema.org/draft-07/schema#"`
2. **Closed protocol objects** — `additionalProperties: false` on data envelopes and ops top-level objects
3. **Extensions required** — every data object includes `extensions` in `required`; use `ExtensionMap` from common
4. **Open vocabulary** — `block_type`, Keyblock `status`, etc. are `type: string` without `enum`; document core vocabulary in `description`
5. **No transport fields** — no HTTP paths, methods, or auth headers in schemas
6. **`$ref` over duplication** — ops reference `schemas/data/*`; do not inline Keyblock copies

## Codegen mapping

| Input | TypeScript output | Rust output |
|-------|-------------------|-------------|
| `schemas/common/*.schema.json` | `packages/spoke-schemas/src/generated/common/` | `crates/spoke-schemas/src/generated/common/` |
| `schemas/data/*.schema.json` | `.../generated/data/` | `.../generated/data/` |
| `schemas/ops/*.schema.json` | `.../generated/ops/` | `.../generated/ops/` |

Commands (from repo root):

```bash
pnpm run codegen          # regenerate TS + Rust
pnpm run verify-codegen   # exit 1 on drift
```

CI gate: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs `verify-codegen` (plus `typescript` and `rust` package checks) on every pull request.

## File checklist (Task 2)

| # | Path | Status |
|---|------|--------|
| 1 | `common/common.schema.json` | done |
| 2 | `common/error-envelope.schema.json` | done |
| 3 | `data/keyblock.schema.json` | done |
| 4 | `data/relation.schema.json` | done |
| 5 | `data/source-anchor.schema.json` | done |
| 6 | `data/finding.schema.json` | done |
| 7 | `data/assemble-packet.schema.json` | done |
| 8 | `ops/upsert-request.schema.json` | done |
| 9 | `ops/upsert-response.schema.json` | done |
| 10 | `ops/promote-request.schema.json` | done |
| 11 | `ops/promote-response.schema.json` | done |
| 12 | `ops/relate-request.schema.json` | done |
| 13 | `ops/relate-response.schema.json` | done |
| 14 | `ops/check-request.schema.json` | done |
| 15 | `ops/check-response.schema.json` | done |
| 16 | `ops/assemble-request.schema.json` | done |
| 17 | `ops/assemble-response.schema.json` | done |

**Total:** 17 schema files for v0.1.
