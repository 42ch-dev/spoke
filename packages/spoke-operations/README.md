# @42ch/spoke-operations

Hand-written TypeScript lifecycle helpers for [SPOKE](https://github.com/42ch-dev/spoke): extension merge/preserve, Finding status transitions, promote acceptance gates, Scope/upsert/relate validators, and `AssemblePacket` builders.

Depends on [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) for wire types. Behavioral parity with the Rust crate [`spoke-operations`](https://crates.io/crates/spoke-operations).

## Install

```bash
pnpm add @42ch/spoke-schemas @42ch/spoke-operations
# Pin both to the same lockstep SemVer (e.g. @X.Y.Z)
```

Pin both packages to the same lockstep SemVer.

## Usage

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

Helpers are pure functions over wire types. Normative behavior: [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md).
