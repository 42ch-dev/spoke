# Mira at Harbor — protocol toy-world fixtures

Minimal **protocol-owned** JSON graph for integrators to validate parsers and codegen against committed `schemas/`. No product adapters or DTO maps.

**Story:** Cartographer Mira arrives at Harbor Town at dawn; a consistency rule flags an open finding; an AssemblePacket scopes context for the scene.

## Files

| File | Schema | Id |
|------|--------|-----|
| `kb_tw_mira.json` | Keyblock | `kb_tw_mira` |
| `kb_tw_harbor.json` | Keyblock | `kb_tw_harbor` |
| `anchor_tw_manuscript.json` | SourceAnchor | (provenance example) |
| `rel_tw_mira_harbor.json` | Relation | `rel_tw_mira_harbor` |
| `evt_tw_harbor_dawn.json` | Event | `evt_tw_harbor_dawn` |
| `rule_tw_consistency.json` | Rule | `rule_tw_consistency` |
| `fnd_tw_open.json` | Finding | `fnd_tw_open` |
| `pkt_tw_scope.json` | AssemblePacket | `pkt_tw_scope` |

`kb_tw_mira` carries `extensions.nexus` and `extensions.creader` with distinct preserve-unknown keys.

## Validate locally

```bash
pnpm run test:fixtures
```

Or from this directory:

```bash
pnpm test
```

CI runs the AJV/Vitest harness via `@42ch/spoke-fixture-toy-world` (`fixtures/toy-world/tests/`).
