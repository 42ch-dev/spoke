---
module: spoke-operations / extraction axis
date: 2026-09-17
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
tags: [ke-extraction, extract, ExtractionPort, orchestrateExtract, runExtractor, run-id, coverage-hint, provisional, promote-gates, input-source]
---

# Extract op — injected extraction surface over a provisional-candidate gate

## Context

The ops wire named its promote operation "extract→promote" but shipped only the promote half: `candidate: KnowledgeEntry` assumed the caller had already extracted. Cross-product "extract with A's engine over B's corpus" had no wire shape. The `ke-extraction` capability closed this with one optional op that mirrors the check pattern.

## Guidance

- **Standalone port, never composed**: `ExtractionPort` / `orchestrateExtract` do not join `BaselinePorts` / `FullPorts` or adapter aliases — a host that does not extract simply omits the capability. Same for the capability gate: `CAPABILITY_PORT_MISSING` carries `details.capability = "ke-extraction"`.
- **Async callback is deliberate**: `runExtractor` is injected and awaited (`F: FnOnce(ExtractRunInput) -> Fut, Fut: Future + Send` in Rust; `RunExtractor` returning a promise in TS). Existing check callbacks stay sync — no `T | Promise<T>` shim to unify them.
- **Correlation without durability**: required `run_id` (exact echo in `run.run_id`) is the only batch identity. No durable ExtractionRun wire object, no coverage reconciliation protocol — mirrors connect's "no durable Session wire object".
- **Provisional-only output**: `orchestrateExtract` gates every candidate through the same provisional/terminal checks promote enforces (never calls promote, never touches revisions or persistence). `CANDIDATE_NOT_PROVISIONAL` rejections carry `details.entry_id` for batch context.
- **`coverage_hint` is opaque and advisory**: `unknown` / `serde_json::Value`; **null and absent are equivalent** — Rust serde maps null → `None` while TS passes null through; both are "no hint". Do not claim byte-level null pass-through in specs.
- **Role pairing**: the existing `input-source` host role pairs with the capability — no new role.

## Why This Matters

The check family already proved "cognition as an injected service, I/O on the wire". Extraction completes the symmetric half; keeping candidates provisional means an external extraction engine can never bypass the promote admission gates or the human-in-loop invariant.

## When to Apply

Any future "product engine as a service" op (extraction was the first). Reuse the standalone-port + injected-callback + baseline-gate-unchanged shape; add a connect routing row only when a consumer demands it (out of scope for this axis).

## Examples

- `.mstar/specs/ke-extraction-adr.md`; `packages/spoke-operations/src/adapter/ports.ts` + `orchestrate.ts`; `crates/spoke-operations/src/adapter/{ports,orchestrate}.rs`.
- Related: `adapter-injection-orchestration.md` (the general ports+orchestrate pattern), `mind-axis-ops-extraction-gate.md` (mind-axis extraction boundary — the ≥2-consumer discipline this op satisfied).
