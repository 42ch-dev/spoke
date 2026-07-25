# @42ch/spoke-schemas

Generated TypeScript wire types for [SPOKE](https://github.com/42ch-dev/spoke) — the Standardized Programmable Ontology Knowledge Engine.

Types are produced from the repository JSON Schema SSOT (`schemas/`). Pair with [`@42ch/spoke-operations`](https://www.npmjs.com/package/@42ch/spoke-operations) for pure lifecycle helpers.

## Install

```bash
pnpm add @42ch/spoke-schemas
# Prefer the same SemVer as @42ch/spoke-operations (e.g. @X.Y.Z)
```

Lockstep with `@42ch/spoke-operations` at the same SemVer when using both.

## Usage

```ts
import type {
  KnowledgeEntry,
  TimelineEvent,
  PromoteRequest,
  AssemblePacket,
  Finding,
  Relation,
} from "@42ch/spoke-schemas";

const entry: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "kb_01",
  entry_type: "character",
  canonical_name: "Aria",
  status: "provisional",
  body: { summary: "A reluctant scout." },
  extensions: {},
};
```

Protocol docs, concepts, and the lockstep SemVer release policy live in the [SPOKE repository](https://github.com/42ch-dev/spoke).
