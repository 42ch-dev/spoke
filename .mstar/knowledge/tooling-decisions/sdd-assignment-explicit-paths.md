---
module: sdd dispatch
date: 2026-08-17
problem_type: tooling_decision
category: tooling-decisions
severity: medium
symptoms:
  - "per-task Assignment files carried the previous task's task-N-brief.md / task-N-report.md paths while headers and scene text updated to the new task"
  - "mstar-harness dispatch validate accepted the wrong-scoped files"
  - "three consecutive assignment files silently kept stale path tokens before an audit caught the third pre-write"
root_cause: "per-task Assignment files were produced by sed-deriving from a sibling task's file; once the source path tokens had themselves been derived (task-2 → task-3 → ...), each substitution pattern matched nothing and sed passed the previous task's lines through unchanged — a coherent-looking but wrong-scoped file that field-presence validation cannot detect"
resolution_type: workflow_improvement
applies_when:
  - "preparing per-task Assignment files for an SDD wave"
  - "writing or reviewing files that carry task-N-brief.md / task-N-report.md path tokens"
tags:
  - sdd
  - dispatch
  - assignment
  - sed
  - path-tokens
  - validation
  - workflow
---

# SDD assignment files: explicit per-task paths, never sed-derived from a sibling

## Problem

During an SDD wave, per-task Assignment files were produced by `sed`-deriving from a sibling task's assignment instead of being written from the plan's task list. Because the source file's path tokens had themselves been derived in earlier steps (task-2 → task-3 → …), each new substitution matched nothing, and the `sed` output fell back to the unmodified previous-task lines. Three consecutive assignment files silently kept the previous task's `task-N-brief.md` / `task-N-report.md` paths while their headers and scene text updated to the new task.

## Symptoms

- An Assignment file describing task N named `task-(N-1)-brief.md` / `task-(N-1)-report.md` as its input/output paths.
- The file looked coherent: task-N headers and scene, task-(N-1) paths — no missing-field or formatting error.
- `mstar-harness dispatch validate` accepted the wrong-scoped files (field-presence checks only; content semantics unchecked).
- Two implementers self-corrected before writing by comparing the dispatch prompt's inline paths with the Assignment file; a third was cancelled pre-write after an audit caught the mismatch.

## What Didn't Work

- **Sed-derive chains from a sibling file**: substituting path tokens in an already-derived file compounds silently. When the pattern stops matching, `sed` passes the original line through unchanged, so the failure mode is a valid-looking wrong value, not an error.
- **Field-presence validation (`mstar-harness dispatch validate`)**: it checks that required fields exist, not that their content matches the task's own scope; a wrong-scoped file with all fields present passes.

## Solution

Write each task's Assignment file from the plan's task list, with that task's own `task-N-brief.md` / `task-N-report.md` path tokens written explicitly — never by transforming a sibling task's file. Keep the dispatch prompt's inline paths as an independent copy of the same facts.

```text
# task-4 assignment derived by sed from task-3 — WRONG
Brief:  task-3-brief.md
Report: task-3-report.md

# task-4 assignment written from the plan task list — RIGHT
Brief:  task-4-brief.md
Report: task-4-report.md
```

## Why This Works

An explicit write has no silent failure mode: the path tokens come from the plan's task list (the single source of truth for task numbering and file naming) and are visibly task-scoped on inspection — a file whose paths name a different task is caught by reading it. The redundancy between the Assignment file and the dispatch prompt's inline paths is deliberate: two channels stating the same facts let an implementer notice a mismatch before touching the wrong files. In this incident the redundancy was the only thing that saved two of the three affected tasks.

## Prevention

- Derive per-task files from the plan's task list in one pass (task N → `task-N-*`); never chain a transform across sibling files.
- If a transform is used anyway, fail it on no-match: assert the output still contains the expected `task-N-` tokens (for example, grep the result) instead of trusting `sed`'s silent passthrough.
- Keep `dispatch validate` for field presence and treat content semantics as a manual gate: checking each Assignment file against its own task is part of dispatch, not optional review.
- Preserve the Assignment-file / dispatch-prompt inline-path redundancy; it is the cheapest second channel for scope errors.
