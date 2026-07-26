# Adapters

Capability-sliced **adapter port contracts** and **injection orchestration** live in the operations packages:

- TypeScript: [`@42ch/spoke-operations`](../packages/spoke-operations/) (`KnowledgeEntryPort`, `BaselinePorts`, `orchestrateUpsert`, …)
- Rust: [`spoke-operations`](../crates/spoke-operations/) (`KnowledgeEntryPort`, `BaselinePorts`, `orchestrate_upsert`, …)

Normative matrix and sequences: [`.mstar/specs/spoke-operations.md`](../.mstar/specs/spoke-operations.md) § Adapter Interfaces / Injection Orchestration.

This directory holds **future product mapping packages** (`adapters/<product>/`) that:

1. Implement the operations port traits/interfaces for a concrete product store
2. Map product DTOs to and from SPOKE wire types (`@42ch/spoke-schemas` / `spoke-schemas`)
3. Preserve `extensions.<namespace>` on round-trip
4. Call operations orchestration entrypoints for shared lifecycle sequences
5. Own transport, persistence, retries, and **transaction boundaries** (including multi-entry upsert)

Active-uniqueness helpers in operations take **caller-supplied peer sets**. Orchestration passes batch-local peers; adapters that need store-wide uniqueness load a wider peer snapshot before calling those helpers.

Schemas in [`schemas/`](../schemas/) remain the protocol SSOT. See [`AGENTS.md`](../AGENTS.md) and [`.mstar/specs/spoke-protocol.md`](../.mstar/specs/spoke-protocol.md).
