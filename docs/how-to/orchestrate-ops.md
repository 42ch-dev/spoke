---
title: Orchestrate operations
---

# Orchestrate operations

The operations library exposes one **orchestrator per op family**. Each orchestrator takes your adapter (the port implementation from [Implement an adapter](/how-to/implement-adapter)) plus the wire request, runs the protocol gates, loads and persists data through your ports, and returns a `SpokeResult` — never a throw for expected rejects. Every orchestrator is an async entrypoint: call it with `await` (TypeScript) or `.await` inside an `async fn` (Rust).

## The orchestrators

| Orchestrator | Request | Response | What it runs |
|--------------|---------|----------|--------------|
| `orchestrateUpsert(ports, request)` | `UpsertRequest` | `UpsertResponse` | validate → status gate → batch uniqueness → OCC put |
| `orchestratePromote(ports, request)` | `PromoteRequest` | `PromoteResponse` | promote acceptance gates → merge target → OCC put |
| `orchestrateRelate(ports, request)` | `RelateRequest` | `RelateResponse` | validate → create/update OCC put |
| `orchestrateCheck(ports, request, runChecker)` | `CheckRequest` | `CheckResponse` | resolve rules → load scope → run checker → persist findings |
| `orchestrateAssemble(ports, request)` | `AssembleRequest` | `AssembleResponse` | load scope → filter → build `AssemblePacket` |
| `orchestrateProject(ports, request)` — `l2-computable` | `ProjectRequest` | `ProjectResponse` | validate → `ComputablePort.project` |
| `orchestrateCompute(ports, request)` — `l2-computable` | `ComputeRequest` | `ComputeResponse` | validate → `ComputablePort.compute` |
| `orchestrateForkCheck` / `orchestrateForkAssemble` — `l5-fork` | fork-scoped requests | same response shapes | require `scope.fork_id` → fork timeline reads |

## Upsert — create or update entries

```ts
import { orchestrateUpsert } from "@42ch/spoke-operations";
import type { UpsertRequest } from "@42ch/spoke-schemas";

async function runUpsert() {
  const result = await orchestrateUpsert(adapter, {
    knowledge_entries: [mira, harbor],
  });

  if (result.ok) {
    console.log(result.value.knowledge_entries.map((e) => e.entry_id));
  }
}
```

`UpsertRequest` carries 1..n entries plus an optional `idempotency_key` (an opaque hint — wire semantics are product-side). The orchestrator validates each entry (`MISSING_REQUIRED_FIELD`, `EMPTY_CANONICAL_NAME`, …), gates status transitions when the entry already exists, checks active-uniqueness against the batch, and persists with the correct expected base revision.

## Promote — extract to durable

```ts
import { orchestratePromote } from "@42ch/spoke-operations";
import type { PromoteRequest } from "@42ch/spoke-schemas";

async function runPromote() {
  const result = await orchestratePromote(adapter, {
    candidate: provisionalEntry,      // typically status "provisional"
    target_entry_id: "kb_existing",   // optional merge target
  });
}
```

Promote runs the acceptance gates (`CANDIDATE_NOT_PROVISIONAL`, `CANDIDATE_TERMINAL_STATUS`, …) and the revision gate, applies the acceptance transition, and persists through `putKnowledgeEntry`. With a `target_entry_id`, the response carries `superseded_id` for the merged-away entry.

## Relate — typed directed edges

```ts
import { orchestrateRelate } from "@42ch/spoke-operations";
import type { RelateRequest } from "@42ch/spoke-schemas";

async function runRelate() {
  const result = await orchestrateRelate(adapter, {
    relation: {
      schema_version: 1,
      relation_id: "rel_mira_harbor",
      relation_type: "located_in",
      from_id: "kb_mira",
      to_id: "kb_harbor",
      extensions: {},
    },
  });
}
```

Relation validation distinguishes create vs update (`RELATION_SELF_EDGE`, `RELATION_MISSING_ENDPOINT`, …), and the OCC-aware put handles revision assignment in your adapter.

## Check — run a checker over a scope

`orchestrateCheck` loads the scoped rules and data first, then hands you a `CheckRunInput` — your checker callback returns `Finding[]`, and the orchestrator persists them:

```ts
import { orchestrateCheck, spokeOk, type CheckRunInput } from "@42ch/spoke-operations";
import type { CheckRequest } from "@42ch/spoke-schemas";

async function runCheck() {
  const result = await orchestrateCheck(adapter, checkRequest, (input: CheckRunInput) => {
    // input: { request, entries, events, rules }
    const findings = myChecker(input.entries, input.rules);
    return spokeOk(findings); // or spokeReject(SpokeRejectCode.INVALID_INPUT, "...")
  });
}
```

Rules resolve from `rule_refs` via `RuleQueryPort`, with embedded `rules[]` overriding by `rule_id`. `check` returns findings only — use `assemble` for context packets.

## Assemble — build a context packet

```ts
import { orchestrateAssemble } from "@42ch/spoke-operations";
import type { AssembleRequest } from "@42ch/spoke-schemas";

async function runAssemble() {
  const result = await orchestrateAssemble(adapter, {
    scope: { scope_id: "book-harbor", entry_types: ["character"] },
    max_entries: 20, // optional entry limit hint
  });
}
```

The orchestrator loads the scope, applies scope filters, and builds a wire-only `AssemblePacket` with order-preserving truncation. Assembly itself — ranking, retrieval, token budgets — is product-side.

## Handle rejects

Every orchestrator returns `SpokeResult`:

```ts
import { SpokeRejectCode } from "@42ch/spoke-operations";

if (!result.ok) {
  switch (result.code) {
    case SpokeRejectCode.REVISION_CONFLICT:
    case SpokeRejectCode.STORED_REVISION_STALE:
      // reload and retry with the fresh revision
      break;
    case SpokeRejectCode.CAPABILITY_PORT_MISSING:
      // the adapter does not implement the optional port this op needs
      break;
    default:
      // validation / state rejects — surface to the caller
  }
}
```

The wire responses follow the same one-failure dialect as the request/response envelopes: a response is either the success payload or `{ "error": ErrorEnvelope }` — never both. The library's rejects map to `ErrorEnvelope` shapes your transport carries.

## The purity boundary

The orchestrators run the protocol gates and drive your ports — they do not touch storage, LLM calls, ranking, retrieval, or transport directly. All of that is supplied by your product through the injected adapter. Finding and promote lifecycles (status transitions, acceptance gates) are pure, pre-persist rules in the library; persistence happens through your ports.

## Next steps

- [Ops wire reference](/reference/ops) — request/response envelope field tables and `Scope`.
- [Implement an adapter](/how-to/implement-adapter) — the port contract behind every orchestrator.
- [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) — orchestrator usage in the committed fixture graph.
