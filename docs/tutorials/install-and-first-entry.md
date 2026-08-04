---
title: Install and create your first KnowledgeEntry
---

# Install and create your first KnowledgeEntry

This tutorial walks you through a complete first round-trip: install the wire-type and operations packages, build a minimal in-memory adapter, upsert a KnowledgeEntry through `orchestrateUpsert`, and read the persisted entry back. Everything runs against the **published packages** — npm and crates.io — no repository checkout required.

The companion tutorial, [Open your first connect session](/tutorials/first-connect-session), adds identity derivation, allowlist, signed hello, and session correlation over a WebSocket.

## Before you start

- **TypeScript path** — Node.js ≥ 20.19 with `pnpm`.
- **Rust path** — a stable Rust toolchain with `cargo`.

All SPOKE packages share one lockstep SemVer (`X.Y.Z`). Pin the same version across every package you install.

## 1. Install the packages

TypeScript (npm):

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
```

Rust (crates.io):

```bash
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

What you get:

- **`@42ch/spoke-schemas` / `spoke-schemas`** — generated wire types (`KnowledgeEntry`, `UpsertRequest`, `UpsertResponse`, …) compiled from the JSON Schema source of truth in [`schemas/`](https://github.com/42ch-dev/spoke/tree/main/schemas).
- **`@42ch/spoke-operations` / `spoke-operations`** — pure lifecycle helpers, capability-sliced adapter ports, and the `orchestrate*` entrypoints that run the protocol gates before persistence.

## 2. Create a KnowledgeEntry

A KnowledgeEntry is the atomic knowledge-base unit: stable `entry_id`, open `entry_type` / `status` strings, a closed `body`, and the required `extensions` bag.

TypeScript:

```ts
import type { KnowledgeEntry } from "@42ch/spoke-schemas";

const mira: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "kb_mira",
  entry_type: "character",
  canonical_name: "Mira Vale",
  status: "provisional",
  body: { summary: "Reluctant cartographer arriving at Harbor Town." },
  extensions: { my_product: { world_id: "wld_harbor" } },
};
```

Rust — the generated types provide typed builders, so construction is compiler-checked:

```rust
use spoke_schemas::data::KnowledgeEntry;

let mira = KnowledgeEntry::builder()
    .schema_version(1)
    .entry_id("kb_mira")
    .entry_type("character")
    .canonical_name("Mira Vale")
    .status("provisional")
    .body(
        spoke_schemas::data::KnowledgeEntryBody::builder()
            .summary("Reluctant cartographer arriving at Harbor Town.".to_string())
            .build()?,
    )
    .extensions(std::collections::HashMap::new())
    .build()?;
```

## 3. Implement a minimal adapter

`orchestrateUpsert` runs the validation and status gates, then persists through your ports. The adapter is the bridge between the protocol and your storage — for this tutorial, an in-memory `Map` implements the baseline port families.

TypeScript:

```ts
import { SpokeRejectCode, spokeOk, spokeReject, type BaselinePorts, type SpokeResult } from "@42ch/spoke-operations";
import type { Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Rule, Scope, TimelineEvent } from "@42ch/spoke-schemas";

class InMemoryAdapter implements BaselinePorts {
  private entries = new Map<string, KnowledgeEntry>();

  getKnowledgeEntry(entryId: string): SpokeResult<KnowledgeEntry> {
    const entry = this.entries.get(entryId);
    return entry === undefined
      ? spokeReject(SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND, `no entry ${entryId}`)
      : spokeOk(entry);
  }

  putKnowledgeEntry(entry: KnowledgeEntry, expectedBaseRevision: number | null): SpokeResult<KnowledgeEntry> {
    const stored = this.entries.get(entry.entry_id);
    if ((stored?.revision ?? null) !== expectedBaseRevision) {
      return spokeReject(SpokeRejectCode.REVISION_CONFLICT, "revision mismatch");
    }
    this.entries.set(entry.entry_id, entry);
    return spokeOk(entry);
  }

  // Relation / Scope / Finding / Rule / Host-manifest ports — return empty
  // defaults for this tutorial; production adapters wire real storage.
  getRelation(): SpokeResult<Relation> { return spokeReject(SpokeRejectCode.RELATION_NOT_FOUND, "no relations"); }
  putRelation(): SpokeResult<Relation> { return spokeReject(SpokeRejectCode.INVALID_INPUT, "no relations"); }
  listKnowledgeEntries(_scope: Scope): SpokeResult<KnowledgeEntry[]> { return spokeOk([...this.entries.values()]); }
  listTimelineEvents(_scope: Scope): SpokeResult<TimelineEvent[]> { return spokeOk([]); }
  putFindings(findings: Finding[]): SpokeResult<Finding[]> { return spokeOk(findings); }
  listRules(): SpokeResult<Rule[]> { return spokeOk([]); }
  getHostCapabilityManifest(): SpokeResult<HostCapabilityManifest> {
    return spokeOk({ schema_version: 1, host_id: "host_tutorial", roles: ["data-store"], capabilities: ["spoke-baseline"], namespaces: ["tutorial"], extensions: {} });
  }
  listPeerHostCapabilityManifests(): SpokeResult<HostCapabilityManifest[]> { return spokeOk([]); }
}
```

Rust — the fixture crate implements the identical port traits on a reference `ToyWorldAdapter`; see [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) for the full Rust port implementation. `spoke-operations` re-exports the wire types (`spoke_operations::spoke_schemas`), so one dependency gives you both.

## 4. Run the upsert round-trip

TypeScript:

```ts
import { orchestrateUpsert } from "@42ch/spoke-operations";
import type { UpsertRequest } from "@42ch/spoke-schemas";

const adapter = new InMemoryAdapter();
const request: UpsertRequest = { knowledge_entries: [mira] };

const result = orchestrateUpsert(adapter, request);

if (result.ok) {
  const persisted = result.value.knowledge_entries[0];
  console.log(persisted.entry_id, persisted.status); // kb_mira provisional
} else {
  console.error(result.code, result.message);
}
```

Expected rejects return `SpokeResult` — a discriminated `{ ok: true }` / `{ ok: false, code, message }` union — instead of throwing, so you can branch on `SpokeRejectCode` (`CANDIDATE_NOT_PROVISIONAL`, `REVISION_CONFLICT`, …).

Rust:

```rust
use spoke_operations::{orchestrate_upsert, BaselinePorts};
use spoke_operations::spoke_schemas::{UpsertRequest, UpsertResponse};

fn upsert(adapter: &impl BaselinePorts, request: UpsertRequest) {
    match orchestrate_upsert(adapter, request) {
        Ok(response) => println!("persisted: {:?}", response.knowledge_entries),
        Err(reject) => println!("rejected: {} — {}", reject.code, reject.message),
    }
}
```

## 5. What the orchestrator did for you

`orchestrateUpsert` runs, per entry: schema validation, status-transition gating (when the entry already exists), active-uniqueness checks against the batch, and an optimistic-concurrency `putKnowledgeEntry` with the expected base revision. Your adapter stays pure I/O — all protocol rules live in the library.

## Next steps

- [Implement an adapter](/how-to/implement-adapter) — which port families your claimed capabilities require.
- [Orchestrate operations](/how-to/orchestrate-ops) — promote, relate, check, and assemble through the same port surface.
- [Open your first connect session](/tutorials/first-connect-session) — talk to another SPOKE host.
- [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) — the committed "Mira at Harbor" graph and a full `FullAdapter` implementation in TypeScript and Rust.
