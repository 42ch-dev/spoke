---
module: morning-star harness coordination
date: 2026-08-16
problem_type: convention
category: tooling-decisions
severity: medium
tags: [status-json, atomic-write, jq, lockdir, harness-coordination]
applies_when: ["scripted mutations of .mstar/status.json", "any read-check-replace-verify file update under the harness write lock"]
---

# status.json mutations: python round-trip under the lockdir, never jq chains

## Context

`.mstar/status.json` is the coordination SSOT for the harness (plan rows, leases, residuals). During a plan-Done write, a `jq '<filters>' status.json > status.json.tmp && mv` pipeline produced an empty output file and the `mv` replaced the registry with a 0-byte file. No APFS snapshot was available; the file was reconstructed from on-disk artifacts (plans directory, iteration close metadata), losing historical notes and task_commits for past iterations.

## Guidance

- Write status.json mutations as a **small python script**: `json.load`, mutate, `json.dump` to the same path (optionally temp+rename after the dump succeeds inside the script). Python raises on filter/encoding mistakes instead of writing an empty file.
- Never pipe through `jq` assignment chains into the SSOT file. jq emits nothing on a filter typo — and an empty stdout overwrites the target on `mv`.
- Every mutation still runs under the same-host exclusive lock (`shlock`/lockdir on `.mstar/.status-write.lockdir/`), re-reading the file immediately before the write (read-check-replace-verify).
- Recovery if truncation happens anyway: reconstruct from `.mstar/plans/` (one file per plan row), `.mstar/iterations/<id>/` close metadata, and `.mstar/archived/`; record the incident in `notes.json` and a `status_reconstruction_note` in root metadata so future readers know which historical fields are partial.

## Why This Matters

The registry is the source of truth for lease ownership and residual state; a silent 0-byte overwrite invalidates every lease/residual decision made after it. The failure mode (jq chain → empty stdout → mv) is easy to write and gives no error at the moment of corruption.

## When to Apply

Any scripted harness-state mutation — status.json, notes.json, or future coordination files under `.mstar/`.

## Examples

Do:

```python
import json
p = ".mstar/status.json"
d = json.load(open(p))
row = next(r for r in d["plans"] if r["plan_id"] == PLAN_ID)
row["status"] = "Done"
json.dump(d, open(p, "w"), indent=2, ensure_ascii=False)
```

Don't: `jq '.plans[0].status="Done"' .mstar/status.json > .mstar/status.json.tmp && mv …` — a typo in the filter yields an empty `.tmp` and the `mv` destroys the registry.
