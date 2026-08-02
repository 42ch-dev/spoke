# SPOKE Roadmap

> Living **project** roadmap (tracked result). Strategy and architecture live in [`STRATEGY.md`](../STRATEGY.md) and [`.mstar/specs/`](specs/). Per-slice execution detail stays in local `delivery-compass.md` (process; gitignored).

**Updated:** 2026-08-02  
**North star:** Cross-product KnowledgeEntry dialect for check + assemble I/O across independent product runtimes.

---

## Now (in progress)

**Integrator notes (durable):** Adapters own transaction boundaries for multi-entry upsert. Active-uniqueness helpers evaluate **caller-supplied peer sets** — orchestration supplies batch-local peers; store-wide uniqueness is available when the adapter loads a wider peer snapshot into that helper. Composed-port aliases (`BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, `FullAdapter`) export from `@42ch/spoke-operations` and `spoke-operations`; reference examples and conformance harness live under `fixtures/toy-world/`. Baseline composition includes required `HostManifestPort` for host self-description and product-supplied peer manifests.

**Persisted-entity OCC parity (durable invariant):** every entity with a write `*Port` and a create-or-update op carries structural `revision` and is persisted through `put*(entity, expectedBaseRevision)` (null/None on create, stored revision on update). `KnowledgeEntry` (upsert/promote) and `Relation` (relate) are the carriers; `Finding`, `Rule`, `HostCapabilityManifest`, `TimelineEvent`, `SourceAnchor`, and `AssemblePacket` are exempt by design (see `spoke-data-model.md` §Persisted-entity OCC parity). `orchestrateRelate` / `orchestrate_relate` deep-integrate load → validate(create/update) → OCC → put, symmetric with `orchestrateUpsert`. Relation error codes: `RELATION_NOT_FOUND`, `RELATION_ALREADY_EXISTS` (plus reused `STORED_REVISION_STALE` / `REVISION_CONFLICT`).

**Extension bags — core / modules / extensions triad (durable):** SPOKE carries three placement homes. **Core fields** serve all baseline hosts. **`extensions.<product>`** is a product/adapter-owned namespace bag on durable data objects and on `Scope` (product-scoped query metadata, e.g. branch / search filters); adapters round-trip unknown namespaces verbatim. **`modules.*`** is the cross-product **functional**-dialect bag — **shipped** as optional `ModuleMap` (object or array namespace values) on `KnowledgeEntry` + `AssemblePacket`, behind the **`narrative-modules`** capability flag (opt-in, same pattern as `l2-computable` / `l5-fork`); inner dialect shapes (activation / pack / placement / activation_trace) stay handbook-defined. Functional dialects never live under `extensions.*` (see [`spoke-extension-modules.md`](specs/spoke-extension-modules.md)).

**Assemble recipes + `modules` wire shipped:** the narrative-dialect foundation for multi-host lore interchange is complete (Phase A exit met).

**spoke-connect interaction family (durable):** SPOKE carries an opt-in fourth wire family `schemas/connect/` (hello / session / invoke request-response / auth challenge-response) behind the **`spoke-connect`** capability flag, enabling multi-adapter client interaction over a shared network. Auth is `noise-peerid` (libp2p Noise peer identity + PeerId allowlist + JCS-signed hello `spoke-connect-hello-jcs-v1` over `{protocol_version, peer_id, nonce, host}`); session ordering is per-direction monotonic `sequence` from 0 + `request_id` correlation; `peer_id` (network trust) is distinct from `host_id` (manifest label). Baseline excludes transport bindings / network discovery - connect is opt-in. A workspace-private Rust reference spike `crates/spoke-connect` (`publish = false`) proves two-node manifest exchange + op invoke over rust-libp2p (noise + yamux + request-response + identify). See [`spoke-connect.md`](specs/spoke-connect.md); knowledge `spoke-connect-wire-and-auth` + `spoke-connect-libp2p-spike`.

**Multi-language embedding contract (durable):** connect is a three-layer embedding model — **wire contract** (envelopes, JCS, identity mapping), **session core** (pure rules: peer_id derive, hello sign/verify over raw keys, nonce, allowlist, sequence, correlation, dispatch gate), **transport adapter** (per-language, per-client: rust-libp2p / js-libp2p / WebSocket / uniffi bindings). Protocol_version 1 identity is Ed25519: `peer_id` = base58btc of the **identity** multihash `0x00` of the protobuf `PublicKey` (not sha2-256 of the raw pubkey) — implementable from [`spoke-connect.md`](specs/spoke-connect.md) alone in any language. Op dispatch gate is MUST-level (op capability ⊆ `negotiated_capabilities`); new ops families register capability + `op` vocabulary in sync with schema descriptions. `crates/spoke-connect` now carries the pure `src/core/` session core (golden vectors locked against libp2p) with a documented sync/async FFI boundary for future uniffi bindings. TS connectivity route locked: **pure-TS-minimal** (WebSocket + WebCrypto/`@noble` Ed25519 + JCS + ported session core) with js-libp2p as mesh fallback — see [`spoke-connect-ts-route.md`](specs/spoke-connect-ts-route.md); identity-byte parity proven in JS by `tooling/connect-identity-proof/`.

---

## Up next (planned)

| Slice | What ships | Notes |
|-------|------------|-------|
| **connect publish execution** | Stage 1: first registry publish of `@42ch/spoke-connect-ts` (npm) + docs site live per [`.mstar/specs/connect-publish-strategy.md`](specs/connect-publish-strategy.md); dist/ build + packed-tarball import smokes; extend `release.yml` publish-npm list + Trusted Publishing + co-update `spoke-version-release.md`; amend the root `AGENTS.md` connect boundary line as part of the Stage 1 change; keep `spoke-connect` crate and bindings unpublished unless Stage 2 decision revises | **Trigger:** decision record accepted on main + published-shape checklist complete + golden/suite green on main + maintainer release cut. **Owner:** release maintainer / next delivery slice |
| **spoke-connect TS SDK deepening** | Continue the pure-TS-minimal path (first slice delivered): published-shape prep in-repo; js-libp2p mesh fallback on demand; browser WebTransport deferred | **Direction confirmed (2026-08-02): keep pure-TS + golden-vector parity discipline** — one Rust core contract, TS re-implementation locked byte-for-byte by golden vectors; **WebTransport = deferred** (WebSocket default); **js-libp2p = fallback-only** (no default dep); **published-shape** prepared per connect-publish-strategy; WASM of the Rust core stays a contingency if pure-TS crypto/JCS diverges from Rust |
| **spoke-connect uniffi bindings (multi-language)** | Binding matrix by product priority: **C#, Go, Python, Swift, Kotlin — C# first**. Swift sync-core skeleton landed; C# next when bindgen toolchain clears | **C# status:** deferred — `uniffi-bindgen-cs` v0.11.0 targets uniffi 0.31 vs repo pin uniffi 0.32 (see [`connect-csharp-bindgen-deferred.md`](specs/connect-csharp-bindgen-deferred.md)); revisit trigger: a bindgen-cs tag (or main commit) targeting uniffi 0.32+. Go/Python/Kotlin: verify community bindgens against uniffi 0.32 before binding work. Async-FFI options in crate README; DHT/NAT discovery remains future |
| **Integrator docs site** | VitePress `docs/` + GitHub Pages publishing Domain Profile handbooks and CONCEPTS-aligned guides | Consumer-facing home for profile handbooks; Stage 1 companion to connect publish (deploy on main; not blocked on npm). Repo-local SSOT remains `.mstar/specs/` until promoted |
| **libp2p transitive vulnerability revisit** | Re-check yamux (0.12.1, CVE-2026-32314) and hickory-proto (0.25.2, GHSA-3v94-mw7p-v465 / GHSA-q2qq-hmj6-3wpp) in `Cargo.lock` and bump when upstream fixes land | Dismissed 2026-08-02 (no upstream fix: libp2p 0.56.0 is the latest release; libp2p-yamux 0.47.0 still pins yamux ^0.12.1; libp2p-mdns 0.48.0 requires hickory-proto ^0.25.2 whose fix is 0.26.1); trigger: libp2p ecosystem bump |

---

## Done (delivered)

Newest first. Dates are delivery dates on `main`.

| When | Slice | What landed |
|------|-------|-------------|
| 2026-08-01 | spoke-connect multi-language SDK start | **TS client first slice** (`@42ch/spoke-connect-ts`, workspace-private, pure-TS-minimal: WebSocket framing + WebCrypto/`@noble` Ed25519 + RFC 8785 JCS + session-core port + connectClient; golden-vector parity + two-node WS interop; CI + lockstep wired); **uniffi Swift sync-core skeleton landed** (`ffi` feature, 8 fns + 3 objects + error enums, cdylib local-only, macOS smoke 25 golden checks, ubuntu CI `--features ffi`); **capability-token auth normative** (claims/proof `{v,claims,sig}`, `trusted_issuers`, challenge/response + per-invoke auth, `auth_failed`/`op_unsupported`, pure `core/capability_token.rs`); **mDNS same-LAN discovery** (non-default `mdns` feature, slot-fair auto-dial, discovery-never-grants-trust, deterministic tests, CI step); knowledge `connect-ts-client-sdk` + `connect-capability-token-auth` + FFI-boundary/spike updates |
| 2026-08-01 | spoke-connect multi-language pre-design | Normative embedding contract in [spoke-connect.md](specs/spoke-connect.md): three-layer embedding model (wire / session core / transport adapter), stack-agnostic identity binding (Ed25519 + identity multihash `0x00` of protobuf `PublicKey` + base58btc), transport framing contract, session-core state machine, op dispatch gate MUST + ops-family registration; `crates/spoke-connect` pure `src/core/` session core extracted (golden vectors locked against libp2p, zero libp2p/tokio deps) + README binding-facade decisions (sync/async FFI boundary, Swift-first matrix); TS connectivity route locked **pure-TS-minimal** ([spoke-connect-ts-route.md](specs/spoke-connect-ts-route.md)) with identity-byte parity proven in JS (`tooling/connect-identity-proof/`, 6/6 PASS); knowledge `connect-session-core-ffi-boundary` + `connect-identity-parity-proof` + wire-and-auth updated |
| 2026-07-31 | spoke-connect foundation | `spoke-connect` opt-in wire family (`schemas/connect/`: hello / session / invoke request-response / auth challenge-response) + capability flag; JCS `spoke-connect-hello-jcs-v1` auth + `noise-peerid` allowlist + dual identity (`peer_id` vs `host_id`) + session ordering; codegen (inventory 24 -> 30); normative spec [spoke-connect.md](specs/spoke-connect.md); two-host fixtures; Rust `crates/spoke-connect` reference spike (`publish = false`, rust-libp2p noise+yamux+request-response+identify) with two-node manifest exchange + op invoke integration test; release lockstep extended to private workspace crates; knowledge `spoke-connect-wire-and-auth` + `spoke-connect-libp2p-spike` |
| 2026-07-30 | Phase A close: `modules` shipped + assemble recipes | `ModuleMap` (object\|array namespace values) shipped as **optional, capability-flagged (`narrative-modules`)** on `KnowledgeEntry` + `AssemblePacket`; codegen (inventory 24 at the time); `mergeModuleMaps`/`preserveModuleMaps` pure ops helpers (generalized namespace-merge core; extension helpers unchanged); ADR/CONCEPTS/handbooks flipped proposed → shipped; AssemblePacket `placement[]`/`activation_trace[]` recipes handbook ([assemble-module-recipes.md](specs/assemble-module-recipes.md)); **Phase A exit met — Phase B (Nexus) unlocked**; knowledge `capability-flagged-optional-bag` |
| 2026-07-30 | Naming triad + Scope.extensions + W1 handbooks | Core / **proposed** `modules.*` / `extensions.<product>` triad ADR ([spoke-extension-modules.md](specs/spoke-extension-modules.md)) + CONCEPTS; optional `Scope.extensions` (`ExtensionMap`) wire + codegen TS/Rust + matcher-ignore preserve tests + fixture (unblocks consumer product query metadata); lore-activation Domain Profile ([domain-profile-lore-activation.md](specs/domain-profile-lore-activation.md), proposed `modules.activation`); Narrative Knowledge Pack handbook ([domain-profile-narrative-knowledge-pack.md](specs/domain-profile-narrative-knowledge-pack.md)) + Seed/Pool pattern + toy-world companion fixture; knowledge `proposed-wire-shape-companion-fixture` |
| 2026-07-29 | Relation OCC parity | `Relation.revision` wire (optional, integer ≥ 0); `RelationPort` OCC parity (`getRelation` + `putRelation(relation, expectedBaseRevision)`, TS+Rust); `orchestrateRelate` deep-integrated load→validate(create/update)→OCC→put; relate gate create/update rules; `RELATION_NOT_FOUND`/`RELATION_ALREADY_EXISTS`; toy-world OCC-aware adapter; dual-language tests; normative persisted-entity OCC-parity guardrail; knowledge `relation-occ-parity` |
| 2026-07-28 | Beat-assist protocol slice | Domain Profile handbook [`domain-profile-narrative-structure.md`](specs/domain-profile-narrative-structure.md); Harbor ordered-moment beat chain + KE-scoped `precedes`; pure timeline sequence helpers (TS + Rust); knowledge `beat-assist-moment-sequence` |
| 2026-07-27 | Host capability collaboration | `HostCapabilityManifest` schema + codegen (inventory **24** at the time); five-role / ns-exclusivity / authority normative specs; baseline-required `HostManifestPort` (TS+Rust); toy-world dual manifests + fixture-backed adapters |
| 2026-07-27 | ToyWorldAdapter reference examples | Runnable TS `ToyWorldAdapter` (`fixtures/toy-world/src/adapter/`) + Rust `spoke-fixture-toy-world` (`fixtures/toy-world/rust/`); CI `cargo test -p spoke-fixture-toy-world` |
| 2026-07-27 | Adapter aliases | `*Adapter` composed-port aliases in TS/Rust ops; integrator docs point to operations + `fixtures/toy-world/` |
| 2026-07-26 | Adapter interfaces + injection orchestration | Capability-sliced ports + `orchestrate*` / `orchestrate_*` in TS and Rust ops packages; `CAPABILITY_PORT_MISSING`; mock-backed orchestration tests; normative matrix in [spoke-operations.md](specs/spoke-operations.md) |
| 2026-07-26 | L2 typed closed body + trait helpers | Closed `body` with optional `summary` / `tags` / `BodyAttribute[]`; codegen; Mira traits fixture; pure TS/Rust trait read helpers; SemVer-agnostic `bump-version` tests; knowledge `l2-closed-body-and-traits` + `release-bump-tests-version-agnostic` |
| 2026-07-24 | Optional Fork (`l5-fork`) | `ForkId`; TimelineEvent `fork_id` / `parent_fork_id`; `Scope.fork_id` + matcher; fixtures; Moment `computable_logs` schema example; schema-count **23** |
| 2026-07-24 | Optional Computable (`l2-computable`) | `body.state` / `body.computable`, Moment `computable_logs`, Session lifecycle normative; optional `project`/`compute` ops; pure validators; fixtures; schema-count **23** |
| 2026-07-23 | KnowledgeEntry / TimelineEvent terminology | Wire locks `KnowledgeEntry` / `TimelineEvent`; ops API + `*KNOWLEDGE_ENTRY*` error codes; fixtures dual-concern pair; product expand **Standardized Programmable Ontology Knowledge Engine** (SPOKE acronym kept) |
| 2026-07-23 | Fixture harness ownership + CI harden | AJV/Vitest under `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`); removed from `@42ch/spoke-operations`; `AGENTS.md` boundary; CI `test:fixtures`; `verify-codegen` schema-count assert |
| 2026-07-23 | Operations library deepen + fixtures | OCC compare, KnowledgeEntry status, uniqueness, Scope/upsert/relate gates, error-envelope map; `fixtures/toy-world/` protocol JSON graph |
| 2026-07-23 | Protocol layers + Rule/TimelineEvent | Normative L0–L8 + capability levels; `Rule` + `TimelineEvent` schemas; Scope / error-envelope / Rule-aware `check` |
| 2026-07-23 | Operations library first slice | `@42ch/spoke-operations`: promote, Finding transitions, extensions preserve, AssemblePacket builders; consumer README EN/CN |
| 2026-07-23 | v0.1 bootstrap | `schemas/` SSOT, codegen `@42ch/spoke-schemas` + Rust `spoke-schemas`, CI verify gate |

### Baseline inventory (as of last Done)

| Area | Status |
|------|--------|
| Data wire | KnowledgeEntry (closed L2 `body`: optional `summary`, `tags`, `attributes`; optional `body.state`/`body.computable`, `computable_logs` under `l2-computable`), **Relation (optional `revision` for OCC parity with KnowledgeEntry)**, SourceAnchor, Finding, AssemblePacket, **HostCapabilityManifest**, Rule, TimelineEvent + `extensions`; optional `fork_id`/`parent_fork_id` on TimelineEvent (`l5-fork`); `OpaqueJson` for opaque log field values |
| Ops wire | upsert / promote / relate / check / assemble (+ Scope, error-envelope); optional project / compute (`l2-computable`). **`relate` is OCC-deep-integrated (load→validate→OCC→put) symmetric with upsert** |
| Ops library | Pure TS + Rust helpers over wire types (incl. `Scope.fork_id` TimelineEvent match; KnowledgeEntry / TimelineEvent naming) plus adapter port contracts (six baseline families incl. `HostManifestPort`), `*Adapter` composed-port aliases, and injection orchestration (`spoke-baseline`, optional `l2-computable`, optional `l5-fork`) |
| Connect wire (opt-in `spoke-connect`) | `schemas/connect/` six envelopes (hello / session / invoke request-response / auth challenge-response); `spoke-connect-hello-jcs-v1` signed hello; `noise-peerid` allowlist auth; session-ordered invoke correlation. Baseline excludes transport bindings / network discovery; connect is opt-in. Rust reference spike `crates/spoke-connect` (`publish = false`) |
| Fixtures | `fixtures/toy-world/` samples + conformance + reference Adapter examples (dual-concern ontology `"event"` + TimelineEvent; Fork-aware TimelineEvent sample under `l5-fork`; two-host connect exchange samples) |
| Codegen / CI | Protocol schema inventory **30** (24 + 6 opt-in connect envelopes); `verify-codegen`; `test:release` for lockstep assert/bump; Rust typify strategy A documented |
| Specs / vocabulary | Umbrella, layers, data-model, ops wire, operations library under `.mstar/specs/`; CONCEPTS + knowledge vocabulary pattern |

---

## Out of scope (durable)

Do not schedule these into SPOKE itself unless strategy is explicitly reversed:

- Shared daemon / MCP server / single multi-product runtime
- I/O, LLM, ranking, retrieval, or product detectors inside `@42ch/spoke-operations`
- Protocol-fixture AJV/fs harness inside `@42ch/spoke-operations` (belongs under `fixtures/`)
- In-repo `adapters/*` product binding packages — product DTO↔SPOKE adapters live in consumer repositories; this repo keeps port contracts + `fixtures/toy-world/` reference examples only
- Default full manuscript text on the wire
- Closed forever enums that freeze product ontology growth
- Creator Memory / unpromoted chat as KnowledgeEntry graph canon
- Publishing fixture or codegen packages to registries

---

## Pointers

| Doc / path | Use for |
|------------|---------|
| [`STRATEGY.md`](../STRATEGY.md) | Why / principles / three-column architecture |
| [`CONCEPTS.md`](../CONCEPTS.md) | KnowledgeEntry / TimelineEvent spelling + dual-concern |
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Normative umbrella |
| [`.mstar/specs/spoke-version-release.md`](specs/spoke-version-release.md) | Lockstep SemVer, tags, CI-gated GitHub Release, registry publish |
| [`.mstar/specs/spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) | L0–L8 + capability levels |
| [`.mstar/specs/domain-profile-narrative-structure.md`](specs/domain-profile-narrative-structure.md) | Narrative-structure Domain Profile — Beat mapping |
| [`knowledge/architecture-patterns/l5-fork-timeline-event-wire.md`](knowledge/architecture-patterns/l5-fork-timeline-event-wire.md) | Compound note on optional Fork wire |
| [`knowledge/architecture-patterns/spoke-codegen-pipeline.md`](knowledge/architecture-patterns/spoke-codegen-pipeline.md) | Codegen inventory, OpaqueJson, Rust typify strategy A |
| [`schemas/`](../schemas/) | Wire SSOT |
| [`packages/spoke-operations/`](../packages/spoke-operations/) | Pure behavior library (TypeScript) |
| [`crates/spoke-operations/`](../crates/spoke-operations/) | Pure behavior library (Rust) |
| [`fixtures/toy-world/`](../fixtures/toy-world/) | Protocol samples, harness, and reference Adapter examples |
