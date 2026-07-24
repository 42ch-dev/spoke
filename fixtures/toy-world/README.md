# Mira at Harbor — protocol toy-world fixtures

Minimal **protocol-owned** JSON graph for integrators to validate parsers and codegen against committed `schemas/`. No product adapters or DTO maps.

**Story:** Cartographer Mira arrives at Harbor Town at dawn; a consistency rule flags an open finding; an AssemblePacket scopes context for the scene. A dual-concern pair links ontology `entry_type: "event"` KnowledgeEntry `kb_tw_harbor_dawn_event` to TimelineEvent `evt_tw_harbor_dawn`. Harbor Town carries optional l2-computable `body.state` / `body.computable` (tide and cargo); the moment-scale timeline event records `computable_logs` for those field changes.

## Files

| File | Schema | Id |
|------|--------|-----|
| `kb_tw_mira.json` | KnowledgeEntry | `kb_tw_mira` |
| `kb_tw_harbor.json` | KnowledgeEntry (l2-computable `body.state` / `body.computable`) | `kb_tw_harbor` |
| `kb_tw_harbor_dawn_event.json` | KnowledgeEntry (`entry_type: "event"`) | `kb_tw_harbor_dawn_event` |
| `anchor_tw_manuscript.json` | SourceAnchor | (provenance example) |
| `rel_tw_mira_harbor.json` | Relation | `rel_tw_mira_harbor` |
| `evt_tw_harbor_dawn.json` | TimelineEvent (`timeline_scale: "moment"`, `computable_logs`) | `evt_tw_harbor_dawn` |
| `rule_tw_consistency.json` | Rule | `rule_tw_consistency` |
| `fnd_tw_open.json` | Finding | `fnd_tw_open` |
| `pkt_tw_scope.json` | AssemblePacket | `pkt_tw_scope` |

`kb_tw_mira` carries two distinct `extensions.<namespace>` bags with preserve-unknown keys (fixture namespaces are illustrative only).

## Validate locally

```bash
pnpm run test:fixtures
```

Or from this directory:

```bash
pnpm test
```

CI runs the AJV/Vitest harness via `@42ch/spoke-fixture-toy-world` (`fixtures/toy-world/tests/`).
