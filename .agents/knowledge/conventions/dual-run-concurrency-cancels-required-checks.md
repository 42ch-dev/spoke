---
module: CI / required status checks
date: 2026-11-23
problem_type: convention
category: conventions
severity: high
tags: [github-actions, concurrency, required-checks, push-vs-pull-request, branch-protection]
---

# A branch that triggers both push and pull_request runs cancels itself and poisons required checks

## Context

Three workflows (`ci.yml`, `cpp-connect.yml`, `xcframework.yml`) triggered on `push` for every branch, `iteration/**` included, and on `pull_request`. Each workflow pinned `concurrency: <name>-${{ github.ref }}` with `cancel-in-progress: true`. A single `git push` to a branch with an open pull request therefore started two runs sharing one concurrency group.

The later run cancelled the earlier one. The cancelled run's check suite records `CANCELLED` and every context it had already started is recorded as `FAILURE`. Because required status checks are evaluated across the head SHA's check runs, and the cancelled suite's failures completed *after* the surviving run's successes, the merge gate reported `9 of 10 required status checks are failing` on a pull request whose surviving run was green. `--admin` merge failed with the same message; nothing visible in the PR checks list explained it.

Recovery attempts made it worse: re-running the surviving run deleted its previous success check runs and replaced them with queued ones, so the failure instances became the only evidence again.

## Guidance

- **Never let one branch feed two event paths into the same concurrency group.** Either scope `push.branches` to the branches whose merges actually need a post-merge run (`main`), or include `${{ github.event_name }}` in the concurrency group so a push and its pull-request run cannot cancel each other.
- **Treat a cancelled check suite as unfixable evidence.** Once a suite cancels, its `FAILURE` contexts stay on the head SHA; re-running the surviving run does not supersede them reliably, and cancelling the other run yourself creates them in the first place.
- **When a required-check verdict disagrees with the visible checks, read `statusCheckRollup` grouped by context name and compare `checkSuite.conclusion` and `completedAt`.** A `CANCELLED` suite with `FAILURE` contexts next to a later `SUCCESS` is the signature.
- **Prefer fixing the trigger over retrying.** The only durable repair is a new head SHA produced by a push that starts exactly one run.

## Verification

After removing `iteration/**` from `push.branches` and adding the event name to each concurrency group, one push to the pull-request branch started a single `pull_request` run; all ten required contexts reported success and the merge completed without an admin override.
