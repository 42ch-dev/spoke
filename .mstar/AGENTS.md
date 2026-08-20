# SPOKE — Harness Directory (`.mstar/`)

> Project identity & tech stack: root [`AGENTS.md`](../AGENTS.md).  
> Runtime lifecycle: upstream Morning Star `mstar-*` skills (not duplicated here).  
> Machine-readable path config: repo-root `.mstarc` `[config]` (gitignored).

This file is the **harness-layer SSOT** for git visibility and write boundaries in SPOKE.

## Conflict resolution

On conflicts (unless the user overrides):

1. Current user instruction  
2. Root [`AGENTS.md`](../AGENTS.md)  
3. **This file**  
4. Upstream `mstar-*` skills  

**Read order (not precedence):** `mstar-harness-core` first; then other `mstar-*` on demand.

## Principle — process vs results

**Process stays out of git. Results are shared with the team.**

| Class | Meaning | Git |
|-------|---------|-----|
| **Results** | Normative or reusable artifacts others must clone | **tracked** |
| **Process** | Local orchestration, scratch, coordination state | **ignored** |

| Path (under `.mstar/`) | Class | Purpose |
|------------------------|-------|---------|
| `AGENTS.md` | Result | This harness contract |
| `specs/` | Result | Frozen normative protocol specs / ADRs |
| `knowledge/` | Result | Compounded cross-iteration knowledge |
| `plans/` | Process | Main plans, checkboxes, gate summaries |
| `iterations/` | Process | Compass, grill locks, iteration packages |
| `sdd/` | Process | SDD scratch + QC/QA raw review bundles |
| `archived/` | Process | Local / archived process snapshots |
| `status.json` | Process | Active lifecycle register (v2) |
| `workflows/` | Process | Per-lifecycle `snapshot.json` + `notes.jsonl` |
| `projects/` | Process | Project layer (`roadmap.md`, `residuals.json`, `research/`) — local only |

**Rules:**

- Agents **may** read/write process paths locally for orchestration.
- **Git-shared results** (`specs/`, `knowledge/`, root `AGENTS.md`, etc.) MUST NOT contain Morning Star iteration ids (e.g. the `v0-iterNNN` family) — use capability or date naming; iteration ids stay in gitignored process paths only. Commit messages and PR titles/bodies follow the same ban (orchestration branch names may keep harness prefixes).
- **Facts only in results (HARD):** `{SPECS_DIR}`, `{KNOWLEDGE_DIR}`, and root `CONCEPTS.md` deposit **current protocol facts** — not deprecated names, rename archaeology, or iteration delivery narrative. Full rule: root [`AGENTS.md`](../AGENTS.md) § Long-term precipitation. `CONCEPTS.md` stays protocol-vocabulary only (no named external product binding tables).
- Do **not** `git add -f` ignored paths unless the user explicitly overrides for a one-off handoff.
- Root [`.gitignore`](../.gitignore) encodes the ignore list — keep it in sync with this table.
- Fresh clone: recreate process files from `mstar-plan-conventions` / `mstar-plan-artifacts` templates as needed. Process paths are **not** clone-shared SSOT.

Wire/schema **code** SSOT remains repo-root `schemas/` (outside `.mstar/`). Language packages are under `packages/` and `crates/`.

## Path symbols

Directory locations are declared in local `.mstarc` `[config]` (engine SSOT, gitignored). This table is git class only.

| Symbol | `.mstarc` key | Git |
|--------|---------------|-----|
| `{HARNESS_DIR}` | `harness_dir` | mixed — see table above |
| `{SPECS_DIR}` | `specs_dir` | tracked |
| `{KNOWLEDGE_DIR}` | `knowledge_dir` | tracked |
| `{WORKFLOW_DIR}` | `workflow_dir` | ignored |
| `{PROJECT_DIR}` | `project_dir` | ignored |
| `{PLAN_DIR}` | `plan_dir` | ignored |
| `{ITERATION_DIR}` | `iteration_dir` | ignored |
| `{SDD_DIR}` | `sdd_dir` | ignored |

Plan `metadata.primary_spec` / `spec_refs` should point at paths under `{SPECS_DIR}` (e.g. `.mstar/specs/spoke-protocol.md`).

## Layout & write boundaries

| Path | Typical writers | Notes |
|------|-----------------|-------|
| `{SPECS_DIR}` | product-manager, architect, writing-specialist | Long-lived normative text only — not iteration scratch |
| `{KNOWLEDGE_DIR}` | `mstar-compound` (iteration-close), writing-specialist (hygiene) | Patterns / conventions — **not** a second specs tree |
| `{PLAN_DIR}` | PM, implementers (checkboxes) | One `.md` per plan; never `plans/<plan-id>/` as a directory |
| `{ITERATION_DIR}` | PM, Phase 1 specialists | Compass + guides; local process |
| `{SDD_DIR}` | implementers, QC, QA | Ephemeral; durable QC/QA conclusions summarize into the main plan (locally) |

**Do not** put plan progress, residual prose, or QC narratives in root `AGENTS.md`.

**Do not** treat ignored process files as team handoff — share **results** (`specs/`, `knowledge/`) and product trees (`schemas/`, packages) via git.

## Anti-patterns

- Committing `.mstarc`, `status.json`, `workflows/`, `plans/`, `iterations/`, `sdd/`, or `projects/`
- Using `{KNOWLEDGE_DIR}` as a dumping ground for unfinished specs
- Duplicating wire contracts under `{SPECS_DIR}` that belong in root `schemas/`
- Force-adding ignored harness paths “for convenience”