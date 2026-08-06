---
title: Package quick-start
---

# Package quick-start

SPOKE ships consumer packages on **lockstep SemVer** — generated wire types, hand-written operations libraries, and Connect session clients (language-native TypeScript client, Rust reference, and native bindings). Pin all surfaces you use to the same `X.Y.Z`.

## TypeScript (npm)

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
```

- **`@42ch/spoke-schemas`** — generated wire types (import `KnowledgeEntry`, `TimelineEvent`, `PromoteRequest`, `AssemblePacket`, `HostCapabilityManifest`, … from the package root).
- **`@42ch/spoke-operations`** — pure helpers, capability-sliced adapter ports, and `orchestrate*` entrypoints.

## Rust (crates.io)

```bash
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

```toml
[dependencies]
spoke-schemas = "X.Y.Z"
spoke-operations = "X.Y.Z"
```

- **`spoke-schemas`** — generated Rust wire types from the same JSON Schema SSOT.
- **`spoke-operations`** — port traits + `orchestrate_*` (re-exports `spoke_schemas`).

## Connect

Connect session clients ship alongside the wire/operations packages at the same lockstep SemVer:

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

- **`@42ch/spoke-connect`** — TypeScript language-native client.
- **`@42ch/spoke-connect/remote`** — the RemoteAdapter subpath: dial a remote peer over a consumer `Transport` and route across multiple peers (see [RemoteAdapter over a Transport](/how-to/connect-remote-adapter)).

```bash
cargo add spoke-connect@X.Y.Z
```

- **`spoke-connect`** — Rust reference crate (libp2p transport + uniffi binding surface).

Native bindings for host languages (C# NuGet `42ch.Spoke.Connect`, Kotlin Maven `dev.42ch:spoke-connect`, Swift SPM `SpokeConnect`, Go module `github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go`, Python PyPI `spoke-connect`) embed the shared session core via FFI — see [Native bindings](/how-to/connect-native-bindings).

## Integrator path

1. Import wire types from the schemas package.
2. Implement the port families for the capabilities you claim on one adapter type (`BaselineAdapter` … `FullAdapter`).
3. Call the matching orchestrator (`orchestrateUpsert`, `orchestratePromote`, …) — pure gates run, persistence goes through your ports.
4. Walk the committed "Mira at Harbor" graph and the reference `ToyWorldAdapter` in `fixtures/toy-world/` (TypeScript adapter + Rust fixture crate).

## Version policy

All packages bump together (lockstep SemVer); annotated tags `vX.Y.Z` match. See [Version & release](/release/versioning) for the pin guide.

## More resources

- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) — install, quick start, operations overview
- [@42ch/spoke-schemas on npm](https://www.npmjs.com/package/@42ch/spoke-schemas) and its [package README](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-schemas/README.md)
- [@42ch/spoke-operations on npm](https://www.npmjs.com/package/@42ch/spoke-operations) and its [package README](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-operations/README.md)
- [spoke-schemas on crates.io](https://crates.io/crates/spoke-schemas) and its [crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-schemas/README.md)
- [spoke-operations on crates.io](https://crates.io/crates/spoke-operations) and its [crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-operations/README.md)
