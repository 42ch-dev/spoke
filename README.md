# SPOKE

**Standardized Programmable Ontology Keyblock Engine** — a protocol repository for narrative **Keyblock** data and operations. Story-AI products (e.g. Nexus, Creader) can share consistency-check and context-assembly I/O without sharing a runtime.

**v0.1 ships:** data-layer schemas (Keyblock, Relation, SourceAnchor, Finding, AssemblePacket); ops-layer schemas (`upsert`, extract→promote, `relate`, `check`, `assemble`); generated TypeScript (`@42ch/spoke-schema`) and Rust (`spoke-schema`); product payloads under `extensions.<namespace>` only.

Adapter implementations and conformance fixtures are **out of scope** — `adapters/nexus/` and `adapters/creader/` are empty placeholders.

Normative specs: [`.mstar/specs/`](.mstar/specs/) (start at [`spoke-protocol.md`](.mstar/specs/spoke-protocol.md)). Schema SSOT: [`schemas/`](schemas/).
