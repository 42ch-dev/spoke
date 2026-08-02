---
layout: home

hero:
  name: SPOKE
  text: Standardized Programmable Ontology Knowledge Engine
  tagline: Protocol documentation for integrators — wire shapes, operations, Domain Profiles, and connect bindings.
  actions:
    - theme: brand
      text: Quick start
      link: /packages/quick-start
    - theme: alt
      text: Read the protocol
      link: /guide/protocol

features:
  - title: Protocol & data model
    details: KnowledgeEntry, relations, findings, ops wire — integrator summaries linked to the normative specs.
    link: /guide/data-model
  - title: Domain Profiles
    details: Narrative structure, lore activation, knowledge packs, assemble recipes.
    link: /profiles/narrative-structure
  - title: Connect
    details: The opt-in connect family — TypeScript route and native binding paths.
    link: /connect/overview
  - title: Packages
    details: npm and crates.io install pins with lockstep versioning.
    link: /packages/quick-start
---

## Why SPOKE

SPOKE is a protocol of JSON Schema wire contracts for narrative knowledge. Independent products exchange consistency-check and context-assembly I/O through shared data and ops shapes, so each product stops inventing local formats for the same concepts.

The repository is the protocol SSOT: hand-authored schemas in `schemas/`, normative specs in `.mstar/specs/`, generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) wire types, and hand-written operations libraries (`@42ch/spoke-operations`, `spoke-operations`) with pure lifecycle helpers, capability-sliced adapter ports, and injection orchestration.

- **One wire dialect** — eight data objects and five baseline ops (plus optional `project` / `compute`) define a common surface for narrative knowledge and checker / context-assembly I/O.
- **Domain Profiles, not closed enums** — ontology vocabulary (beat mapping, lore activation, knowledge packs, assemble placement) publishes over open strings and optional `modules.*` bags, so core schemas stay stable.
- **Capability flags** — claim `spoke-baseline`, or declare optional `l2-computable`, `l5-fork`, `narrative-modules`, and `spoke-connect` explicitly.
- **Language parity** — generated schema packages plus operations libraries at one lockstep SemVer across npm and crates.io.
- **Opt-in connect** — signed cross-process interaction envelopes with per-session ordering and extensible auth.

## Next steps

- [Package quick-start](/packages/quick-start) — install and pin the TypeScript and Rust packages
- [Protocol guide](/guide/protocol) — the three columns of the protocol
- [Layers & capabilities](/guide/layers) — baseline vs optional capability flags
- [Domain Profiles](/profiles/narrative-structure) — publish ontology vocabulary over open wire
- [Connect](/connect/overview) — cross-process interaction envelopes

## Normative references

The authoritative protocol documentation lives in the repository specs — these pages summarize and link, with the specs remaining the single source of truth.

- [spoke-protocol.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol.md) — umbrella: three columns, schema inventory, extensions contract
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — protocol vocabulary, defined in wire terms
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — nine layers (L0–L8) and capability levels
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — data objects, Rule, TimelineEvent
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — ops wire request/response envelopes
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) — operations library behavior
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — connect envelope family
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) — repository overview, install, and quick start
