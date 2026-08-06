---
layout: home
hero:
  name: SPOKE
  text: Standardized Programmable Ontology Knowledge Engine
  tagline: Integrator documentation — a wire dialect for narrative knowledge, with one path for every job.
  actions:
    - { theme: brand, text: Build an Adapter, link: /how-to/implement-adapter }
    - { theme: alt, text: Open a Connect session, link: /tutorials/first-connect-session }
features:
  - { title: Tutorials, details: "Two end-to-end paths: install and upsert your first KnowledgeEntry, then open your first connect session.", link: /tutorials/install-and-first-entry }
  - { title: How-to guides, details: "The integrator journey: implement the adapter ports your capabilities require, then connect hosts with the TypeScript client, a RemoteAdapter over a consumer Transport, multi-peer routing, or the native bindings.", link: /how-to/implement-adapter }
  - { title: Reference, details: "Verify wire facts on-site: protocol, data-model, ops, and connect field tables sourced from the schemas.", link: /reference/protocol }
  - { title: Explanation, details: "The concepts behind the wire: nine layers, capability flags, dual-concern pairs, and the four Domain Profiles.", link: /explanation/concepts }
---

## Start here

If you run a product that stores narrative knowledge, [implement an adapter](/how-to/implement-adapter) — pick the port families for the capabilities you claim, and call the matching orchestrators. If you run a separate SPOKE host and want to talk to another host, [open a connect session](/tutorials/first-connect-session) or jump straight to the [TypeScript client](/how-to/connect-ts-client) / [native bindings](/how-to/connect-native-bindings). Complete beginners follow the two tutorials in order: [Install and create your first KnowledgeEntry](/tutorials/install-and-first-entry), then [Open your first connect session](/tutorials/first-connect-session).

## Why SPOKE

SPOKE is a protocol of JSON Schema wire contracts for narrative knowledge: independent products exchange consistency-check and context-assembly I/O through shared data and ops shapes, so each product stops inventing local formats for the same concepts. The repository is the SSOT — hand-authored schemas, generated TypeScript and Rust wire types, and pure operations libraries.

- **One wire dialect** — eight data objects and five baseline ops (plus optional `project` / `compute`)
- **Domain Profiles over open strings** — ontology vocabulary publishes over open strings and optional `modules.*` bags
- **Capability flags** — claim `spoke-baseline`, or declare `l2-computable`, `l5-fork`, `narrative-modules`, `spoke-connect` explicitly
- **Language parity** — one lockstep SemVer across npm and crates.io
- **Opt-in connect** — signed cross-process interaction envelopes with per-session ordering and extensible auth

## Further reading

- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — protocol vocabulary, defined in wire terms
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) — repository overview, install, and quick start
- [Package quick-start](/packages/quick-start) — npm and crates.io install pins
- [Version & release](/release/versioning) — lockstep SemVer for integrators
