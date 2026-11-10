---
module: security / scanning governance
date: 2026-09-17
problem_type: tooling-decision
category: tooling-decisions
severity: low
plan_id: security-sweep
tags: [codeql, code-scanning, dependabot, dismissal, golden-vectors, test-only, hard-coded-crypto]
---

# Code scanning test-only hard-coded crypto values — dismiss, don't refactor

## Context

CodeQL flags `rust/hard-coded-cryptographic-value` (critical) wherever a literal lands in a variable that flows into a nonce/key parameter — including `#[cfg(test)]` modules and `tests/` helpers. A protocol repo dense with golden vectors and dummy nonces (`"nonce-1"`, `"raw-handshake-nonce-0000"`) accumulates these alerts even though every flagged site is test-only and unreachable from production.

## Guidance

**Refactoring test literals to dodge the scanner is the wrong move.** Data-flow analysis follows construction, not just literals — `format!`/concat rewrites often re-trigger, and they damage the direct readability that makes golden-vector tests auditable. The durable pattern:

1. **Verify the claim first**: open the flagged `path:line` and confirm the value sits inside `#[cfg(test)]` / `#[test]` / `tests/` and has no production caller. A dismissal without this check is how real leaks get papered over.
2. **Dismiss with reason `used in tests`** and a factual comment naming the golden-vector/dummy-nonce pattern. GitHub's API takes the reason as `'used in tests'` (space form) and the note in `dismissed_comment`.
3. **Expect recurrences**: any new test with a literal nonce re-raises the query. Handle each the same way after the same verification; do not batch-dismiss unverified alerts.
4. **File-level CodeQL `paths-ignore` is not an option** when the flagged file mixes production code with a `#[cfg(test)]` module (e.g. `src/core/nonce.rs`) — exclusion would blind the scanner to the production half.

## Why This Matters

The 2026-09 security sweep cleared 13 critical alerts without touching a single test: verification (test-only reachability) + dismissal kept the panel clean while preserving golden-vector auditability. The same sweep confirmed the transitive-dep side: a stale `pnpm-workspace.yaml` override (`js-yaml@>=4.0.0 <4.3.1: 4.3.1`) was itself pinning the vulnerable version — check existing overrides before assuming the lockfile is just stale.

## When to Apply

- Any `rust/hard-coded-cryptographic-value` (or sibling hard-coded-secret queries) alert pointing into test code.
- Any Dependabot transitive bump that "doesn't move" — look for a pinning override before adding a new one.
