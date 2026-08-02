---
layout: home
hero:
  name: SPOKE
  text: Standardized Programmable Ontology Knowledge Engine
  tagline: Integrator documentation — wire shapes, operations, Domain Profiles, and connect bindings.
  actions:
    - { theme: brand, text: Quick start, link: /packages/quick-start }
    - { theme: alt, text: Read the protocol, link: /guide/protocol }
features:
  - { title: Protocol & data model, details: "KnowledgeEntry, relations, findings, ops wire — integrator summaries linked to the normative specs.", link: /guide/data-model }
  - { title: Domain Profiles, details: "Narrative structure, lore activation, knowledge packs, assemble recipes.", link: /profiles/narrative-structure }
  - { title: Connect, details: "The opt-in connect family — TypeScript route and native binding paths.", link: /connect/overview }
  - { title: Packages, details: "npm and crates.io install pins with lockstep versioning.", link: /packages/quick-start }
---

## Why SPOKE

SPOKE is a protocol of JSON Schema wire contracts for narrative knowledge: independent products exchange consistency-check and context-assembly I/O through shared data and ops shapes, so each product stops inventing local formats for the same concepts. The repository is the SSOT — hand-authored schemas, normative specs in `.mstar/specs/`, generated TypeScript and Rust wire types, and pure operations libraries.

- **One wire dialect** — eight data objects and five baseline ops (plus optional `project` / `compute`)
- **Domain Profiles over open strings** — ontology vocabulary publishes over open strings and optional `modules.*` bags
- **Capability flags** — claim `spoke-baseline`, or declare `l2-computable`, `l5-fork`, `narrative-modules`, `spoke-connect` explicitly
- **Language parity** — one lockstep SemVer across npm and crates.io
- **Opt-in connect** — signed cross-process interaction envelopes with per-session ordering and extensible auth

## Normative references

The specs in the repository are the single source of truth; these pages summarize and link to them.

- [spoke-protocol.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol.md) — umbrella: three columns, schema inventory, extensions contract
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — protocol vocabulary, defined in wire terms
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — nine layers (L0–L8) and capability levels
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — data objects, Rule, TimelineEvent
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — ops wire request/response envelopes
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) — operations library behavior
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — connect envelope family
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) — repository overview, install, and quick start
