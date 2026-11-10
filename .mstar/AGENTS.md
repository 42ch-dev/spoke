# SPOKE — Harness Layout

> Project identity & tech stack: root [`AGENTS.md`](../AGENTS.md).  
> Runtime lifecycle: upstream Morning Star `mstar-*` skills (not duplicated here).  
> Machine-readable path config: repo-root [`.mstarc`](../.mstarc) `[config]` (tracked).

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

The harness root (`.mstar/`) holds process state; the three tracked corpora live under `.agents/`.

| Class | Meaning | Git |
|-------|---------|-----|
| **Results** | Normative or reusable artifacts others must clone | **tracked** |
| **Process** | Local orchestration, scratch, coordination state | **ignored** |

| Path | Class | Purpose |
|------|-------|---------|
| `.mstarc` | Result | Path config (engine SSOT) |
| `.agents/specs/` | Result | Project current-state specs (`<class>/<topic>.md`) |
| `.agents/adr/` | Result | Decision records (`<bucket>/<class>/yyyy-mm-dd-topic.md`) |
| `.agents/knowledge/` | Result | Compounded cross-iteration knowledge |
| `.mstar/AGENTS.md` | Result | This harness contract |
| `.mstar/plans/` | Process | Main plans, checkboxes, gate summaries |
| `.mstar/iterations/` | Process | Compass, grill locks, iteration packages |
| `.mstar/sdd/` | Process | SDD scratch + QC/QA raw review bundles |
| `.mstar/archived/` | Process | Local / archived process snapshots |
| `.mstar/workflows/` | Process | Per-lifecycle `snapshot.json` + `notes.jsonl` |
| `.mstar/projects/` | Process | Project layer (`roadmap.md`, `residuals.json`, `research/`) — local only |
| `.mstar/store.db` | Process | Issue/catalog and execution authority |

**Rules:**

- Agents **may** read/write process paths locally for orchestration.
- **Git-shared results** (`specs/`, `adr/`, `knowledge/`, root `AGENTS.md`, etc.) MUST NOT contain Morning Star iteration ids (e.g. the `v0-iterNNN` family) — use capability or date naming; iteration ids stay in gitignored process paths only. Commit messages and PR titles/bodies follow the same ban (orchestration branch names may keep harness prefixes).
- **Facts only in results (HARD):** `{SPECS_DIR}`, `{ADR_DIR}`, `{KNOWLEDGE_DIR}`, and root `CONCEPTS.md` deposit **current protocol facts** — not deprecated names, rename archaeology, or iteration delivery narrative. Full rule: root [`AGENTS.md`](../AGENTS.md) § Long-term precipitation. `CONCEPTS.md` stays protocol-vocabulary only (no named external product binding tables).
- **Layer split (HARD):** `{SPECS_DIR}` states what the system is now; `{ADR_DIR}` records one decision, its real alternatives and its cost; `{KNOWLEDGE_DIR}` records reusable engineering experience. A spec carries no decision history; an ADR carries no state description.
- Do **not** `git add -f` ignored paths unless the user explicitly overrides for a one-off handoff.
- Root [`.gitignore`](../.gitignore) encodes the ignore list — keep it in sync with this table.
- Fresh clone: recreate process files from the harness templates as needed. Process paths are **not** clone-shared SSOT.

Wire/schema **code** SSOT remains repo-root `schemas/` (outside `.mstar/`). Language packages are under `packages/` and `crates/`.

## Path symbols

Directory locations are declared in tracked `.mstarc` `[config]` (engine SSOT, resolved relative to the file's directory). This table is git class only.

| Symbol | `.mstarc` key | Value | Git |
|--------|---------------|-------|-----|
| `{HARNESS_DIR}` | `harness_dir` | `.mstar` | mixed — see table above |
| `{SPECS_DIR}` | `specs_dir` | `.agents/specs` | tracked |
| `{ADR_DIR}` | `adr_dir` | `.agents/adr` | tracked |
| `{KNOWLEDGE_DIR}` | `knowledge_dir` | `.agents/knowledge` | tracked |
| `{WORKFLOW_DIR}` | `workflow_dir` | `.mstar/workflows` (default) | ignored |
| `{PROJECT_DIR}` | `project_dir` | `.mstar/projects` (default) | ignored |
| `{PLAN_DIR}` | `plan_dir` | `.mstar/plans` (default) | ignored |
| `{ITERATION_DIR}` | `iteration_dir` | `.mstar/iterations` (default) | ignored |
| `{SDD_DIR}` | `sdd_dir` | `.mstar/sdd/<plan-id>` (default) | ignored |

Plan `metadata.primary_spec` / `spec_refs` point at paths under `{SPECS_DIR}` (e.g. `.agents/specs/protocol/spoke-protocol.md`).

## Layout & write boundaries

| Path | Typical writers | Notes |
|------|-----------------|-------|
| `{SPECS_DIR}` | product-manager, architect, writing-specialist | Current state of the protocol; fixed skeleton (Scope / Current state / Invariants / Where it lives / Change obligations) |
| `{ADR_DIR}` | architect, product-manager | One decision per record; `implemented/` holds live authority, `proposed/` / `rejected/` / `archived/` hold the other fates |
| `{KNOWLEDGE_DIR}` | `mstar-compound` (iteration-close), writing-specialist (hygiene) | Patterns / conventions — **not** a second specs tree |
| `{PLAN_DIR}` | PM, implementers (checkboxes) | One `.md` per plan; never `plans/<plan-id>/` as a directory |
| `{ITERATION_DIR}` | PM, Phase 1 specialists | Compass + guides; local process |
| `{SDD_DIR}` | implementers, QC, QA | Ephemeral; durable QC/QA conclusions summarize into the main plan (locally) |

**Do not** put plan progress, residual prose, or QC narratives in root `AGENTS.md`.

**Do not** treat ignored process files as team handoff — share **results** (`specs/`, `adr/`, `knowledge/`) and product trees (`schemas/`, packages) via git.

## Anti-patterns

- Committing `status.json`, `workflows/`, `plans/`, `iterations/`, `sdd/`, `projects/`, or `store.db`
- Using `{KNOWLEDGE_DIR}` as a dumping ground for unfinished specs
- Putting decision history or acceptance lists in `{SPECS_DIR}`
- Duplicating wire contracts under `{SPECS_DIR}` that belong in root `schemas/`
- Force-adding ignored harness paths “for convenience”
