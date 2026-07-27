# @42ch/spoke-operations

Hand-written TypeScript lifecycle helpers for [SPOKE](https://github.com/42ch-dev/spoke): extension merge/preserve, Finding status transitions, promote acceptance gates, Scope/upsert/relate validators, `body.attributes` read helpers, `AssemblePacket` builders, **capability-sliced adapter ports**, and **injection orchestration**.

Depends on [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) for wire types. Behavioral parity with the Rust crate [`spoke-operations`](https://crates.io/crates/spoke-operations).

## Install

```bash
pnpm add @42ch/spoke-schemas @42ch/spoke-operations
# Pin both to the same lockstep SemVer (e.g. @X.Y.Z)
```

Pin both packages to the same lockstep SemVer.

## Usage — pure helpers

```ts
import type { PromoteRequest } from "@42ch/spoke-schemas";
import {
  validatePromoteRequest,
  applyPromoteAcceptance,
  buildAssemblePacket,
  transitionFindingStatus,
  mergeExtensionMaps,
  SpokeRejectCode,
} from "@42ch/spoke-operations";

const request: PromoteRequest = { candidate /* KnowledgeEntry */ };
const gate = validatePromoteRequest(request);

if (gate.ok) {
  const accepted = applyPromoteAcceptance(request);
  // Persist via your product adapter
} else {
  console.error(gate.code, gate.message); // e.g. SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL
}
```

## Usage — adapter ports and orchestration

Implement capability-sliced ports (`KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, `HostManifestPort`, plus optional `ComputablePort` / `ForkTimelineQueryPort`) on one adapter type. Ergonomic composed-port aliases (`BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, `FullAdapter`) name the same intersections as `BaselinePorts`, `ComputablePorts`, `ForkPorts`, and `FullPorts`. Then call the matching orchestrator:

```ts
import type { CheckRequest, UpsertRequest } from "@42ch/spoke-schemas";
import {
  orchestrateUpsert,
  orchestrateCheck,
  spokeOk,
  type BaselinePorts,
  type CheckRunInput,
} from "@42ch/spoke-operations";

declare const ports: BaselinePorts;
declare const upsertRequest: UpsertRequest;
declare const checkRequest: CheckRequest;

const upserted = orchestrateUpsert(ports, upsertRequest);

const checked = orchestrateCheck(ports, checkRequest, (_input: CheckRunInput) => {
  // Product-owned checker; library loads scope data and persists findings via ports
  return spokeOk([]);
});
```

Optional capabilities use the composed port types (`ComputablePorts`, `ForkPorts`) with `orchestrateProject` / `orchestrateCompute` and `orchestrateForkCheck` / `orchestrateForkAssemble`.

**Integrator notes**

- Adapters own **transaction boundaries** for multi-entry upsert and other multi-write sequences.
- Active-uniqueness helpers take **caller-supplied peer sets**. Orchestration supplies batch-local peers; pass a store-wide snapshot when product uniqueness must span the whole store.
- Absent optional ports at a dynamic boundary surface `SpokeRejectCode.CAPABILITY_PORT_MISSING`. `HostManifestPort` is baseline-required — not gated behind that code.

Reference **FullAdapter** implementation: [`fixtures/toy-world/`](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) (`ToyWorldAdapter` in TypeScript `src/adapter/`).

Helpers and orchestrators are pure relative to host I/O — all reads and writes go through injected ports. Normative behavior: [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md).
