# `schemas/` — JSON Schema SSOT

SPOKE wire contracts. **Hand-authored only** — TypeScript and Rust types are generated into `packages/spoke-schemas/` and `crates/spoke-schemas/`.

Normative docs: [`spoke-protocol.md`](../.mstar/specs/spoke-protocol.md) (umbrella), [`spoke-protocol-layers.md`](../.mstar/specs/spoke-protocol-layers.md) (L0–L8), [`spoke-data-model.md`](../.mstar/specs/spoke-data-model.md) (data layer), [`spoke-ops.md`](../.mstar/specs/spoke-ops.md) (ops layer).

## Layout (protocol layers deepen)

```text
schemas/
├── README.md                           # this file
├── common/
│   ├── common.schema.json              # SchemaVersion, ExtensionMap, Scope, TimelineScale, ForkId, BodyAttribute, ComputableFieldMap, ComputableLogChange, ComputableLogEntry, shared ids
│   └── error-envelope.schema.json      # shared ops error shape
├── data/
│   ├── knowledge-entry.schema.json
│   ├── relation.schema.json
│   ├── source-anchor.schema.json
│   ├── finding.schema.json
│   ├── assemble-packet.schema.json
│   ├── rule.schema.json                # L6 declarative constraint input
│   ├── timeline-event.schema.json      # L5 when-axis temporal object
│   ├── mind-state.schema.json          # L5 temporal mental-state record (l5-mind)
│   └── host-capability-manifest.schema.json  # host roles, capabilities, namespaces
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
    ├── assemble-response.schema.json
    ├── project-request.schema.json       # l2-computable — init / projection
    ├── project-response.schema.json
    ├── compute-request.schema.json       # l2-computable — apply / settle
    └── compute-response.schema.json
```

Plus the opt-in **connect** family (`spoke-connect` flag):

```text
connect/
├── connect-hello.schema.json          # signed manifest exchange (noise-peerid)
├── connect-session.schema.json        # ordered invocation context snapshot
├── connect-invoke-request.schema.json # remote op call (wraps ops envelopes)
├── connect-invoke-response.schema.json # oneOf payload | error
├── connect-auth-challenge.schema.json # extensible auth challenge
└── connect-auth-response.schema.json  # method-specific proof
```

**Total:** **31** hand-authored schema files (2 common + 9 data + 14 ops + 6 connect). `check-request` / `assemble-request` `$ref` shared `Scope`; all ops responses use `oneOf` success | error envelope. Optional `project` / `compute` ops under `l2-computable`; `mind-state` under the opt-in `l5-mind` capability; connect envelopes under the opt-in `spoke-connect` capability. See [`spoke-protocol.md`](../.mstar/specs/spoke-protocol.md).

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
- Example: `"$id": "https://spoke42.invalid/schemas/data/knowledge-entry.schema.json"`
- Cross-file refs use absolute spoke42.invalid URIs (codegen resolves consistently)

## Authoring rules

1. **Draft-07** — `"$schema": "http://json-schema.org/draft-07/schema#"`
2. **Closed protocol objects** — `additionalProperties: false` on data envelopes and ops top-level objects
3. **Extensions required** — every data object includes `extensions` in `required`; use `ExtensionMap` from common
4. **Open vocabulary** — `entry_type`, KnowledgeEntry `status`, etc. are `type: string` without `enum`; document core vocabulary in `description`
5. **No transport fields** — no HTTP paths, methods, or auth headers in schemas
6. **`$ref` over duplication** — ops reference `schemas/data/*`; do not inline KnowledgeEntry copies

## Codegen mapping

| Input | TypeScript output | Rust output |
|-------|-------------------|-------------|
| `schemas/common/*.schema.json` | `packages/spoke-schemas/src/generated/common/` | `crates/spoke-schemas/src/generated/common/` |
| `schemas/data/*.schema.json` | `.../generated/data/` | `.../generated/data/` |
| `schemas/ops/*.schema.json` | `.../generated/ops/` | `.../generated/ops/` |
| `schemas/connect/*.schema.json` | `.../generated/connect/` | `.../generated/connect/` |

Commands (from repo root):

```bash
pnpm run codegen          # regenerate TS + Rust
pnpm run verify-codegen   # exit 1 on drift
```

CI gate: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs `verify-codegen` (plus `typescript` and `rust` package checks) on every pull request.

## File checklist

| # | Path | Status |
|---|------|--------|
| 1 | `common/common.schema.json` | done |
| 2 | `common/error-envelope.schema.json` | done |
| 3 | `data/knowledge-entry.schema.json` | done |
| 4 | `data/relation.schema.json` | done |
| 5 | `data/source-anchor.schema.json` | done |
| 6 | `data/finding.schema.json` | done |
| 7 | `data/assemble-packet.schema.json` | done |
| 8 | `data/rule.schema.json` | done |
| 9 | `data/timeline-event.schema.json` | done |
| 10 | `data/mind-state.schema.json` | done |
| 11 | `data/host-capability-manifest.schema.json` | done |
| 12 | `ops/upsert-request.schema.json` | done |
| 13 | `ops/upsert-response.schema.json` | done |
| 14 | `ops/promote-request.schema.json` | done |
| 15 | `ops/promote-response.schema.json` | done |
| 16 | `ops/relate-request.schema.json` | done |
| 17 | `ops/relate-response.schema.json` | done |
| 18 | `ops/check-request.schema.json` | done |
| 19 | `ops/check-response.schema.json` | done |
| 20 | `ops/assemble-request.schema.json` | done |
| 21 | `ops/assemble-response.schema.json` | done |
| 22 | `ops/project-request.schema.json` | done |
| 23 | `ops/project-response.schema.json` | done |
| 24 | `ops/compute-request.schema.json` | done |
| 25 | `ops/compute-response.schema.json` | done |
| 26 | `connect/connect-hello.schema.json` | done |
| 27 | `connect/connect-session.schema.json` | done |
| 28 | `connect/connect-invoke-request.schema.json` | done |
| 29 | `connect/connect-invoke-response.schema.json` | done |
| 30 | `connect/connect-auth-challenge.schema.json` | done |
| 31 | `connect/connect-auth-response.schema.json` | done |

**Total:** 31 schema files (mind-state + connect envelope families landed).
