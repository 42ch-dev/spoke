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

## Remote and FFI exposure (connect)

Exposing `extract` over connect follows the **service-shaped op** rule, not the port-method rule:

- `extract` is a core operation, not a `port.*` method: the wire carries the existing `ExtractRequest` in and the `ExtractResponse` success branch out, and the peer runs the whole local orchestration (`loadExtractionInput` → `runExtractor`) itself. The loaded value and the extractor are never transport parameters — a loader-only canary asserted absent from captured envelopes in both directions is the proof, and a wire-visible positive control keeps the assertion non-vacuous.
- The responder probes the injected ports structural face (TS `typeof ports.extract === "function"`; Rust `RemoteServePorts::as_extract()`) **after** the capability gate. Compose that face **explicitly** (Rust `RemoteServePortsComposite::with_extract(...)`) — a default/blanket `as_extract() -> None` silently masks a service that is actually provided, which reads as an unreachable feature rather than a misconfiguration.
- Three unavailability paths must stay distinguishable in every channel: unnegotiated capability deny and absent-provider probe deny both carry `wire_code = op_unsupported`, while an application refusal from a present callback carries `wire_code = None` with exactly one call. Collapsing them makes "misconfigured host" indistinguishable from "caller not authorized".

**Proving TS/Rust agreement.** Build a paired-observation matrix over the frozen scenarios and compare *conclusions* — reject code, presence/absence of `details.wire_code`, and provider call counts — never table shapes or snapshot names. That is what surfaces real divergence: an identical-looking pair of dispatch tables still disagreed on a schema-invalid-but-viewpoint-bearing `Scope` (Rust rejected it before the provider, TypeScript forwarded it), and on the closed key set of the extract request (Rust's generated `deny_unknown_fields` refused unknown top-level keys, TypeScript accepted them until it mirrored the closure).

**Host-side integration constraint.** `orchestrate_extract` takes `&dyn ExtractionPort` without a `Send` bound, so its future is `!Send`, while a connect-owned service trait that the responder drives under `tokio::spawn` must be `Send + Sync`. A Rust host therefore cannot `await` the orchestrator directly inside its service implementation; it bridges (for example a blocking worker). Treat this as a documented host recipe constraint, not a wire-contract defect.

## Examples

- `.mstar/specs/ke-extraction-adr.md`; `packages/spoke-operations/src/adapter/ports.ts` + `orchestrate.ts`; `crates/spoke-operations/src/adapter/{ports,orchestrate}.rs`.
- Related: `adapter-injection-orchestration.md` (the general ports+orchestrate pattern), `mind-axis-ops-extraction-gate.md` (mind-axis extraction boundary — the ≥2-consumer discipline this op satisfied).
