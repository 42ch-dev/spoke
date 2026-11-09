---
title: For agents
---

# For agents

This page is the reading contract for code assistants and automated integrators. The rest of the site is written for human integrators; the facts here tell an agent where to read, which source wins, and which mistakes generated code usually makes.

## Ground truth

When sources disagree, use this order:

| Rank | Source | Why |
|------|--------|-----|
| 1 | `schemas/` | Hand-authored JSON Schema. Field names, required sets, patterns and closed objects are defined here |
| 2 | `.mstar/specs/` | Normative protocol prose: layers, data model, ops, memory, connect, bindings |
| 3 | `/reference/*` | Field tables and envelopes derived from the schemas |
| 4 | `/how-to/*`, `/tutorials/*` | Procedures and code samples; they assume the reference facts |
| 5 | `/explanation/*` | Concepts and boundaries |
| 6 | `README.md` | Human entry point; not normative |

A docs page that contradicts `schemas/` is a docs bug. Do not generate code from the contradiction.

## Read order

1. `/reference/protocol` — schema inventory, capability flags, `extensions` and `modules` contract.
2. `/reference/data-model` — object fields. Read the object before writing a literal.
3. `/reference/ops` — request and response envelopes, `Scope`, `ErrorEnvelope`.
4. The capability reference you need: `/reference/memory`, `/reference/mind-state`, `/reference/connect`.
5. `/how-to/implement-adapter` and `/how-to/orchestrate-ops` — ports and `orchestrate*` sequences.
6. `/explanation/concepts` — layers and dual-concern pairs, when a name looks ambiguous.

## Task routing

| Task | Read first | Then |
|------|-----------|------|
| Persist KnowledgeEntry data | `/reference/data-model` | `/how-to/implement-adapter` |
| Call an op | `/reference/ops` | `/how-to/orchestrate-ops` |
| Add a capability flag | `/reference/protocol` | `/explanation/concepts` |
| Serve memory | `/reference/memory` | `/how-to/implement-adapter` |
| Open a cross-process session | `/reference/connect` | `/how-to/connect-ts-client` |
| Proxy adapter ports over a transport | `/how-to/connect-remote-adapter` | `/reference/connect` |
| Call or expose a remote tool | `/reference/connect` | `/how-to/connect-remote-tools` |
| Bind from another language | `/how-to/connect-native-bindings` | `/how-to/remote-adapter-native-binding` |
| Pin versions | `/packages/quick-start` | `/release/versioning` |

## Capability flags

`spoke-baseline` is the only baseline claim. Every other flag is additive and declared explicitly.

| Flag | Adds |
|------|------|
| `l2-computable` | `body.state` / `body.computable`, `TimelineEvent.computable_logs`, `project` and `compute` |
| `l5-fork` | `fork_id` / `parent_fork_id` on TimelineEvent, `Scope.fork_id` |
| `spoke-cognition` | `modules.cognition`, `Scope.cognition`, `modules.observation`, derivative `MindState` |
| `narrative-modules` | the `modules` bag on KnowledgeEntry, AssemblePacket and TimelineEvent |
| `ke-extraction` | the `extract` op family and `ExtractionPort` |
| `ke-ownership` | `KnowledgeEntry.owner` / `disclosure`, `Scope.viewpoint` |
| `spoke-connect` | the six connect envelopes |
| `spoke-memory` | 13 memory models and 33 `memory.*` ops; `spoke-memory-reflection` and `spoke-memory-transfer` are sub-flags |

## Common traps

- `Rule` is checker **input**; `Finding` is checker **output**. `entry_type: "rule"` is an ontology label, not the L6 object.
- `TimelineEvent` is the L5 when-axis object. `entry_type: "event"` is an ontology label; one concept may map to both.
- `MindState` is derivative. The authority stays on the holder KnowledgeEntry `modules.cognition` and `modules.mental`.
- Connect envelopes carry `signature` in the committed schema, which encodes the protocol_version 2 wire shape.
- Sequences are per direction and start at 0. A response echoes `session_id`, `sequence` and `request_id`.
- Ops results are `SpokeResult`. Branch on `ok` and `SpokeRejectCode`, never on thrown exception types.
- Adapter writes are optimistic-concurrency controlled: `expectedBaseRevision: null` means create; a number must equal the stored revision.
- Op dispatch rejects before any handler runs: a known requirement that is not negotiated answers `capability_missing`; an op with no dispatch row or no registered handler answers `op_unsupported`. Do not emulate a family you did not declare.

## Machine-readable bundle

- [`/llms.txt`](/llms.txt) — index with one line per page.
- [`/llms-full.txt`](/llms-full.txt) — every English page concatenated in reading order, generated from the page sources by `tooling/docs/build-llms.mjs`.
