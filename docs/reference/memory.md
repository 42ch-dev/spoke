---
title: Memory
---

# Memory (`spoke-memory`)

Per-subject long-term memory as a wire family: spaces, subjects, grants, capture, candidate review, retained records, recall, author profiles, reflections, and versioned export/import. Memory is **subjective** — an impression the host captured for one subject — and it lives beside canonical KnowledgeEntry data as a separate family. Required on every durable memory object: `schema_version`, `extensions`.

The family is optional. A host claims it by declaring `spoke-memory` in `HostCapabilityManifest.capabilities`, and the wire contract is the thirteen models plus 33 `memory.*` operations below.

## What the family carries

| Concern | Wire artifacts |
|---------|----------------|
| Where memory lives | `MemorySpace` — one authoritative provider per space, `revision`, computed `pending_receipts` |
| Who memory is about | `Subject` — stable opaque `subject_id` plus optional namespaced `kind`; flat, one subject per record |
| Who may do what | `MemoryGrant` — capability set + optional subject scope + optional `context_ids` + `shared_access` + expiry |
| Capture and review | `memory.capture` → `MemoryCandidate` (immutable `capture_scope`, advisory `classification`) → `memory.candidates.review` / `.dismiss` |
| Retained memory | `RetainedRecord` — untrusted `content` (`text` + optional opaque `structured`), placement `scope`, provenance, `revision`, optional `tombstone` |
| Reading memory | `ReadSelection` (placement modes) + `ProvenanceFilter` (source-context projection) + `memory.recall` budget/truncation |
| Authored basis | `ProfileDocument` — trusted author-maintained content kinds (`personality`, `preferences`) |
| Derived synthesis | `Reflection` — generated over a pinned basis, `stale` computed at read time |
| Outcomes | `MemoryReceipt` — durable, payload-bound, content-free (`committed` / `pending` / `failed`) |
| Provider replacement | `MemoryExportBundle` + `MemoryExportChunk`, plus the staged-import activation flow |

## Capability flags

| Flag | Gates | Ops |
|------|-------|-----|
| `spoke-memory` | Family core | 25 core `memory.*` ops |
| `spoke-memory-reflection` | Reflection generation, state, listing | `memory.reflections.state`, `.list`, `.generate` |
| `spoke-memory-transfer` | Export/import | `memory.exports.create`, `.get`, `memory.imports.stage`, `.activate`, `.discard` |

A host declaring a sub-flag also declares `spoke-memory`; a host serving only `spoke-memory` answers the sub-flag ops with `capability_missing` before any handler runs. Flags are negotiated per session over connect, and the dispatch gate evaluates the negotiated set only.

## Operations

| Op | Intent |
|----|--------|
| `memory.describe` | Provider self-description: protocol version, served flags, limits, required profile kinds, engine policy version |
| `memory.spaces.list` | Spaces visible to the caller, with `pending_receipts` (the drain-barrier read) |
| `memory.spaces.freeze` | Freeze a space for maintenance or migration |
| `memory.spaces.unfreeze` | Re-open a frozen space |
| `memory.subjects.ensure` | Idempotent get-or-create of a subject |
| `memory.subjects.get` | One subject record |
| `memory.subjects.fence` | Fence, unfence, or archive a subject; fence or unfence one placement context |
| `memory.grants.list` | Grants in a space (admin view or the caller's own) |
| `memory.grants.grant` | Issue a grant within the issuer's own coverage |
| `memory.grants.revoke` | Terminally revoke a grant |
| `memory.capture` | Capture raw material into pending candidates |
| `memory.candidates.list` | Pending and consumed candidates |
| `memory.candidates.get` | One candidate |
| `memory.candidates.dismiss` | Consume a candidate without a review decision |
| `memory.candidates.review` | Promote, fragment, or drop — the only candidate → record path |
| `memory.records.create` | Direct authored write |
| `memory.records.get` | One retained record, re-authorized |
| `memory.records.update` | Update content or kind (placement changes use `promote`) |
| `memory.records.delete` | Remove content, keep a tombstone |
| `memory.records.list` | Selection + filter listing with `total_authorized` |
| `memory.records.promote` | Explicit placement change between contexts or to shared |
| `memory.recall` | Authorized, ranked, budgeted execution-time read |
| `memory.profile.get` | Authored profile documents (absent kinds are honest-empty) |
| `memory.profile.update` | Create or update an authored profile document |
| `memory.receipts.get` | Poll a receipt, resolve an uncertain outcome |
| `memory.reflections.state` | Current basis digest, staleness, data sufficiency, missing required kinds |
| `memory.reflections.list` | Reflections, newest first, staleness computed at read time |
| `memory.reflections.generate` | Explicit cost-bearing generation over a pinned basis |
| `memory.exports.create` | Produce a chunked logical export |
| `memory.exports.get` | Fetch one export chunk |
| `memory.imports.stage` | Stage chunks in strict ordinal order and verify on the final chunk |
| `memory.imports.activate` | Atomic visibility flip; bumps `authority_generation` |
| `memory.imports.discard` | Pre-activation rollback of staged data |

Read ops return their payload; every mutating op returns a `MemoryReceipt`. Only reflection generation, export creation, the final staging chunk, and import activation may report `pending`, and completion is observed through `memory.receipts.get`.

## Data models

| Model | Role |
|-------|------|
| `MemorySpace` | One authoritative store; unit of freeze, export, and import activation |
| `Subject` | The entity memory is about; `active` / `fenced` / `archived` |
| `Principal` | Authenticated caller identity, established by admission (never asserted in a payload) |
| `MemoryGrant` | Provider-enforced authorization record |
| `RetainedRecord` | Reviewed or directly authored durable memory |
| `MemoryCandidate` | Provisional capture awaiting review |
| `ProfileDocument` | Trusted authored personality/preferences basis |
| `Reflection` | Derived synthesis over a pinned basis |
| `MemoryReceipt` | Content-free operation receipt (the single async model) |
| `ReadSelection` | Placement selection mode for reads |
| `ProvenanceFilter` | Source-context projection filter, independent of placement |
| `MemoryExportBundle` | Versioned, digested logical export |
| `MemoryExportChunk` | Bounded wire unit of an export |

## Provider and adapter guidance

`MemoryPort` is the standalone optional boundary — a TypeScript interface and a Rust trait, one typed method per `memory.*` op, with the authenticated principal passed explicitly. Illustrative shape (request/response types are the generated `memory-*` wire types):

```ts
import type { MemoryPort } from "@42ch/spoke-operations";
import type { Principal } from "@42ch/spoke-schemas";

const port: MemoryPort = {
  describe(principal: Principal, request: MemoryDescribeRequest) { /* … */ },
  capture(principal: Principal, request: MemoryCaptureRequest) { /* … */ },
  // … 33 typed methods, one per memory.* op
};
```

- **Availability probe.** `MemoryPorts.asMemory()` (TS) / `MemoryPorts::as_memory()` (Rust) reports whether an adapter serves the family. The port is not part of `BaselinePorts`, so a product that does not offer memory simply omits it.
- **Provider duties stay in the provider.** Grants and non-amplification, `revision` CAS, durable idempotency keyed on `(space_id, principal, op, idempotency_key)`, receipts and dedup tombstones, space freeze and subject/context fencing, and atomic candidate review all live in the provider's own transaction layer.
- **Serving over connect.** The serving host exposes the connect-owned `RemoteMemoryService` seam; `MemoryPortService` implements it over a provider `MemoryPort` plus an admission context that maps the authenticated peer to a `Principal`. The 33 typed operations are the decode surface — no second serving trait.
- **Algorithm policy is separable.** The `spoke-memory` Rust reference crate holds deterministic, I/O-free policy (candidate classification, recall ranking and budget, reflection assembly and staleness, transfer partitioning/digests/translation). It is not a protocol participant: providers may run it, run their own policy, or run none, and stay wire-compatible.
- **Profiles are authored, records are captured.** Profile content is trusted author material and grants no authorization by itself; record and candidate content is untrusted data. Reflection generation is admitted only when the provider's `required_profile_kinds` are present, and an absent non-required kind is an honest-empty result.

## Field tables and spec

Wire-level detail lives in the spec corpus (SSOT) — not duplicated here:

- [Memory family spec](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-memory.md) — capability flags, invariants `MEM-R001`–`MEM-R027`, data-model field tables, per-op contracts, errors and retry classes, wire mapping, ProfileDocument substance.
- [Memory capability ADR](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-memory-capability-adr.md) — optional-family decision, single-authority model, receipt async model, engine/provider split.
- [Memory data schemas](https://github.com/42ch-dev/spoke/tree/main/schemas/data) and [memory op schemas](https://github.com/42ch-dev/spoke/tree/main/schemas/ops) — the committed wire truth.
- [Connect reference](/reference/connect) — the envelopes that carry `memory.*` payloads.

## Related

- [Protocol reference](/reference/protocol) — the three columns, capability flags, and schema inventory.
- [Ops wire reference](/reference/ops) — baseline and optional op families, `ErrorEnvelope`, shared rules.
- [Concepts](/explanation/concepts) — how memory sits beside canonical knowledge.
- [Implement an adapter](/how-to/implement-adapter) — the port families per capability, including the optional memory provider.
