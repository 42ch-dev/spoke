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
│   ├── host-capability-manifest.schema.json  # host roles, capabilities, namespaces
│   └── tool-descriptor.schema.json     # declared tool ABI (tools.<ns>.<tool_id>)
│   ├── memory-space.schema.json
│   ├── subject.schema.json
│   ├── principal.schema.json
│   ├── memory-grant.schema.json
│   ├── retained-record.schema.json
│   ├── memory-candidate.schema.json
│   ├── profile-document.schema.json
│   ├── reflection.schema.json
│   ├── memory-receipt.schema.json
│   ├── memory-export-bundle.schema.json
│   ├── memory-export-chunk.schema.json
│   ├── read-selection.schema.json
│   ├── provenance-filter.schema.json
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
    ├── compute-response.schema.json
    ├── extract-request.schema.json       # ke-extraction — referenced-source input
    └── extract-response.schema.json      # ke-extraction — provisional candidates + run
    ├── memory-describe-request.schema.json   # spoke-memory memory.describe
    ├── memory-spaces-list-request.schema.json   # spoke-memory memory.spaces.list
    ├── memory-spaces-freeze-request.schema.json   # spoke-memory memory.spaces.freeze
    ├── memory-spaces-unfreeze-request.schema.json   # spoke-memory memory.spaces.unfreeze
    ├── memory-subjects-ensure-request.schema.json   # spoke-memory memory.subjects.ensure
    ├── memory-subjects-get-request.schema.json   # spoke-memory memory.subjects.get
    ├── memory-subjects-fence-request.schema.json   # spoke-memory memory.subjects.fence
    ├── memory-grants-list-request.schema.json   # spoke-memory memory.grants.list
    ├── memory-grants-grant-request.schema.json   # spoke-memory memory.grants.grant
    ├── memory-grants-revoke-request.schema.json   # spoke-memory memory.grants.revoke
    ├── memory-capture-request.schema.json   # spoke-memory memory.capture
    ├── memory-candidates-list-request.schema.json   # spoke-memory memory.candidates.list
    ├── memory-candidates-get-request.schema.json   # spoke-memory memory.candidates.get
    ├── memory-candidates-dismiss-request.schema.json   # spoke-memory memory.candidates.dismiss
    ├── memory-candidates-review-request.schema.json   # spoke-memory memory.candidates.review
    ├── memory-records-create-request.schema.json   # spoke-memory memory.records.create
    ├── memory-records-get-request.schema.json   # spoke-memory memory.records.get
    ├── memory-records-update-request.schema.json   # spoke-memory memory.records.update
    ├── memory-records-delete-request.schema.json   # spoke-memory memory.records.delete
    ├── memory-records-list-request.schema.json   # spoke-memory memory.records.list
    ├── memory-records-promote-request.schema.json   # spoke-memory memory.records.promote
    ├── memory-recall-request.schema.json   # spoke-memory memory.recall
    ├── memory-profile-get-request.schema.json   # spoke-memory memory.profile.get
    ├── memory-profile-update-request.schema.json   # spoke-memory memory.profile.update
    ├── memory-reflections-state-request.schema.json   # spoke-memory memory.reflections.state
    ├── memory-reflections-list-request.schema.json   # spoke-memory memory.reflections.list
    ├── memory-reflections-generate-request.schema.json   # spoke-memory memory.reflections.generate
    ├── memory-receipts-get-request.schema.json   # spoke-memory memory.receipts.get
    ├── memory-exports-create-request.schema.json   # spoke-memory memory.exports.create
    ├── memory-exports-get-request.schema.json   # spoke-memory memory.exports.get
    ├── memory-imports-stage-request.schema.json   # spoke-memory memory.imports.stage
    ├── memory-imports-activate-request.schema.json   # spoke-memory memory.imports.activate
    ├── memory-imports-discard-request.schema.json   # spoke-memory memory.imports.discard
    ├── memory-describe-response.schema.json  # spoke-memory memory.describe
    ├── memory-spaces-list-response.schema.json  # spoke-memory memory.spaces.list
    ├── memory-spaces-freeze-response.schema.json  # spoke-memory memory.spaces.freeze
    ├── memory-spaces-unfreeze-response.schema.json  # spoke-memory memory.spaces.unfreeze
    ├── memory-subjects-ensure-response.schema.json  # spoke-memory memory.subjects.ensure
    ├── memory-subjects-get-response.schema.json  # spoke-memory memory.subjects.get
    ├── memory-subjects-fence-response.schema.json  # spoke-memory memory.subjects.fence
    ├── memory-grants-list-response.schema.json  # spoke-memory memory.grants.list
    ├── memory-grants-grant-response.schema.json  # spoke-memory memory.grants.grant
    ├── memory-grants-revoke-response.schema.json  # spoke-memory memory.grants.revoke
    ├── memory-capture-response.schema.json  # spoke-memory memory.capture
    ├── memory-candidates-list-response.schema.json  # spoke-memory memory.candidates.list
    ├── memory-candidates-get-response.schema.json  # spoke-memory memory.candidates.get
    ├── memory-candidates-dismiss-response.schema.json  # spoke-memory memory.candidates.dismiss
    ├── memory-candidates-review-response.schema.json  # spoke-memory memory.candidates.review
    ├── memory-records-create-response.schema.json  # spoke-memory memory.records.create
    ├── memory-records-get-response.schema.json  # spoke-memory memory.records.get
    ├── memory-records-update-response.schema.json  # spoke-memory memory.records.update
    ├── memory-records-delete-response.schema.json  # spoke-memory memory.records.delete
    ├── memory-records-list-response.schema.json  # spoke-memory memory.records.list
    ├── memory-records-promote-response.schema.json  # spoke-memory memory.records.promote
    ├── memory-recall-response.schema.json  # spoke-memory memory.recall
    ├── memory-profile-get-response.schema.json  # spoke-memory memory.profile.get
    ├── memory-profile-update-response.schema.json  # spoke-memory memory.profile.update
    ├── memory-reflections-state-response.schema.json  # spoke-memory memory.reflections.state
    ├── memory-reflections-list-response.schema.json  # spoke-memory memory.reflections.list
    ├── memory-reflections-generate-response.schema.json  # spoke-memory memory.reflections.generate
    ├── memory-receipts-get-response.schema.json  # spoke-memory memory.receipts.get
    ├── memory-exports-create-response.schema.json  # spoke-memory memory.exports.create
    ├── memory-exports-get-response.schema.json  # spoke-memory memory.exports.get
    ├── memory-imports-stage-response.schema.json  # spoke-memory memory.imports.stage
    ├── memory-imports-activate-response.schema.json  # spoke-memory memory.imports.activate
    └── memory-imports-discard-response.schema.json  # spoke-memory memory.imports.discard
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

**Total:** **113** hand-authored schema files (2 common + 23 data + 82 ops + 6 connect). 79 new `spoke-memory` schemas (13 data + 66 ops). `check-request` / `assemble-request` `$ref` shared `Scope`; all ops responses use `oneOf` success | error envelope. Optional `project` / `compute` ops under `l2-computable`; optional `extract` op under `ke-extraction`; `mind-state` under the opt-in `l5-mind` capability; connect envelopes under the opt-in `spoke-connect` capability. See [`spoke-protocol.md`](../.mstar/specs/spoke-protocol.md).

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
| 32 | `data/tool-descriptor.schema.json` | done |
| 33 | `ops/extract-request.schema.json` | done |
| 34 | `ops/extract-response.schema.json` | done |
| 35 | `data/memory-space.schema.json` | done |
| 36 | `data/subject.schema.json` | done |
| 37 | `data/principal.schema.json` | done |
| 38 | `data/memory-grant.schema.json` | done |
| 39 | `data/retained-record.schema.json` | done |
| 40 | `data/memory-candidate.schema.json` | done |
| 41 | `data/profile-document.schema.json` | done |
| 42 | `data/reflection.schema.json` | done |
| 43 | `data/memory-receipt.schema.json` | done |
| 44 | `data/memory-export-bundle.schema.json` | done |
| 45 | `data/memory-export-chunk.schema.json` | done |
| 46 | `data/read-selection.schema.json` | done |
| 47 | `data/provenance-filter.schema.json` | done |
| 48 | `ops/memory-describe-request.schema.json` | done |
| 49 | `ops/memory-describe-response.schema.json` | done |
| 50 | `ops/memory-spaces-list-request.schema.json` | done |
| 51 | `ops/memory-spaces-list-response.schema.json` | done |
| 52 | `ops/memory-spaces-freeze-request.schema.json` | done |
| 53 | `ops/memory-spaces-freeze-response.schema.json` | done |
| 54 | `ops/memory-spaces-unfreeze-request.schema.json` | done |
| 55 | `ops/memory-spaces-unfreeze-response.schema.json` | done |
| 56 | `ops/memory-subjects-ensure-request.schema.json` | done |
| 57 | `ops/memory-subjects-ensure-response.schema.json` | done |
| 58 | `ops/memory-subjects-get-request.schema.json` | done |
| 59 | `ops/memory-subjects-get-response.schema.json` | done |
| 60 | `ops/memory-subjects-fence-request.schema.json` | done |
| 61 | `ops/memory-subjects-fence-response.schema.json` | done |
| 62 | `ops/memory-grants-list-request.schema.json` | done |
| 63 | `ops/memory-grants-list-response.schema.json` | done |
| 64 | `ops/memory-grants-grant-request.schema.json` | done |
| 65 | `ops/memory-grants-grant-response.schema.json` | done |
| 66 | `ops/memory-grants-revoke-request.schema.json` | done |
| 67 | `ops/memory-grants-revoke-response.schema.json` | done |
| 68 | `ops/memory-capture-request.schema.json` | done |
| 69 | `ops/memory-capture-response.schema.json` | done |
| 70 | `ops/memory-candidates-list-request.schema.json` | done |
| 71 | `ops/memory-candidates-list-response.schema.json` | done |
| 72 | `ops/memory-candidates-get-request.schema.json` | done |
| 73 | `ops/memory-candidates-get-response.schema.json` | done |
| 74 | `ops/memory-candidates-dismiss-request.schema.json` | done |
| 75 | `ops/memory-candidates-dismiss-response.schema.json` | done |
| 76 | `ops/memory-candidates-review-request.schema.json` | done |
| 77 | `ops/memory-candidates-review-response.schema.json` | done |
| 78 | `ops/memory-records-create-request.schema.json` | done |
| 79 | `ops/memory-records-create-response.schema.json` | done |
| 80 | `ops/memory-records-get-request.schema.json` | done |
| 81 | `ops/memory-records-get-response.schema.json` | done |
| 82 | `ops/memory-records-update-request.schema.json` | done |
| 83 | `ops/memory-records-update-response.schema.json` | done |
| 84 | `ops/memory-records-delete-request.schema.json` | done |
| 85 | `ops/memory-records-delete-response.schema.json` | done |
| 86 | `ops/memory-records-list-request.schema.json` | done |
| 87 | `ops/memory-records-list-response.schema.json` | done |
| 88 | `ops/memory-records-promote-request.schema.json` | done |
| 89 | `ops/memory-records-promote-response.schema.json` | done |
| 90 | `ops/memory-recall-request.schema.json` | done |
| 91 | `ops/memory-recall-response.schema.json` | done |
| 92 | `ops/memory-profile-get-request.schema.json` | done |
| 93 | `ops/memory-profile-get-response.schema.json` | done |
| 94 | `ops/memory-profile-update-request.schema.json` | done |
| 95 | `ops/memory-profile-update-response.schema.json` | done |
| 96 | `ops/memory-reflections-state-request.schema.json` | done |
| 97 | `ops/memory-reflections-state-response.schema.json` | done |
| 98 | `ops/memory-reflections-list-request.schema.json` | done |
| 99 | `ops/memory-reflections-list-response.schema.json` | done |
| 100 | `ops/memory-reflections-generate-request.schema.json` | done |
| 101 | `ops/memory-reflections-generate-response.schema.json` | done |
| 102 | `ops/memory-receipts-get-request.schema.json` | done |
| 103 | `ops/memory-receipts-get-response.schema.json` | done |
| 104 | `ops/memory-exports-create-request.schema.json` | done |
| 105 | `ops/memory-exports-create-response.schema.json` | done |
| 106 | `ops/memory-exports-get-request.schema.json` | done |
| 107 | `ops/memory-exports-get-response.schema.json` | done |
| 108 | `ops/memory-imports-stage-request.schema.json` | done |
| 109 | `ops/memory-imports-stage-response.schema.json` | done |
| 110 | `ops/memory-imports-activate-request.schema.json` | done |
| 111 | `ops/memory-imports-activate-response.schema.json` | done |
| 112 | `ops/memory-imports-discard-request.schema.json` | done |
| 113 | `ops/memory-imports-discard-response.schema.json` | done |

**Total:** 113 schema files (34 pre-existing + 79 spoke-memory: 13 data models + 33 op request/response pairs).
