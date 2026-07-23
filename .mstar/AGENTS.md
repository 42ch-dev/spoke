# SPOKE harness (`.mstar/`)

> Project rules: root [`AGENTS.md`](../AGENTS.md). Runtime: upstream `mstar-*` skills.

## Path symbols

| Symbol | Path |
|--------|------|
| `{HARNESS_DIR}` | `.mstar/` |
| `{PLAN_DIR}` | `plans/` |
| `{SDD_DIR}` | `sdd/<plan-id>/` (gitignored) |
| `{ITERATION_DIR}` | `iterations/` |
| `{KNOWLEDGE_DIR}` | `knowledge/` |
| `{SPECS_DIR}` | repo-root `specs/` |

## Content boundaries

| Path | Holds |
|------|-------|
| `plans/` | Main plans + durable gate summaries |
| `iterations/` | Compass + iteration package |
| `knowledge/` | Compounded reusable notes (iteration-close) |
| `specs/` (repo root) | Normative protocol specs / ADRs |
| `status.json` | Structured plan/residual metadata only |

Do not put dynamic QC prose in root `AGENTS.md`. Do not commit `.mstar/sdd/`.
