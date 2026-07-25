# @42ch/spoke-operations

Hand-written TypeScript lifecycle helpers for [SPOKE](https://github.com/42ch-dev/spoke): extension merge/preserve, Finding status transitions, promote acceptance gates, Scope/upsert/relate validators, and `AssemblePacket` builders.

Depends on [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) for wire types. Behavioral parity with the Rust crate `spoke-operations`.

## Install

```bash
pnpm add @42ch/spoke-operations
```

## Usage

```ts
import {
  applyPromoteAcceptance,
  mergeExtensionMaps,
} from "@42ch/spoke-operations";
```

Helpers are pure functions over wire types. Protocol docs and the lockstep SemVer release policy live in the [SPOKE repository](https://github.com/42ch-dev/spoke).
