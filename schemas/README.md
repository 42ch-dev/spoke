# `schemas/` — JSON Schema SSOT

SPOKE wire contracts. **Hand-authored only** — TypeScript and Rust types are generated into `packages/spoke-schema/` and `crates/spoke-schema/`.

Normative docs: [`specs/spoke-protocol.md`](../specs/spoke-protocol.md) (umbrella), [`specs/spoke-data-model.md`](../specs/spoke-data-model.md) (data layer), [`specs/spoke-ops.md`](../specs/spoke-ops.md) (ops layer).

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

**Explicitly absent in v0.1:** `schemas/data/rule.schema.json` (deferred — see [data model §Rule deferral](../specs/spoke-data-model.md#rule-deferral-v01-decision)).

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
| `schemas/common/*.schema.json` | `packages/spoke-schema/src/generated/common/` | `crates/spoke-schema/src/generated/common/` |
| `schemas/data/*.schema.json` | `.../generated/data/` | `.../generated/data/` |
| `schemas/ops/*.schema.json` | `.../generated/ops/` | `.../generated/ops/` |

Commands (from repo root, after Task 3 wiring):

```bash
pnpm run codegen          # regenerate TS + Rust
pnpm run verify-codegen   # exit 1 on drift
```

## File checklist (Task 2)

| # | Path | Status |
|---|------|--------|
| 1 | `common/common.schema.json` | pending |
| 2 | `common/error-envelope.schema.json` | pending |
| 3 | `data/keyblock.schema.json` | pending |
| 4 | `data/relation.schema.json` | pending |
| 5 | `data/source-anchor.schema.json` | pending |
| 6 | `data/finding.schema.json` | pending |
| 7 | `data/assemble-packet.schema.json` | pending |
| 8–17 | `ops/*` (10 files) | pending |

**Total:** 17 schema files for v0.1.
