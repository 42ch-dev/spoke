# SPOKE Roadmap

> Living **project** roadmap (tracked result). Strategy and architecture live in [`STRATEGY.md`](../STRATEGY.md) and [`.mstar/specs/`](specs/). Per-slice execution detail stays in local `delivery-compass.md` (process; gitignored).
>
> **Scope:** this repository only — protocol wire, pure ops libraries, fixtures, connect SDK prep, integrator docs. Consumer product engines (e.g. activation runtimes) are not scheduled here.

**Updated:** 2026-08-04  
**North star:** Cross-product KnowledgeEntry dialect for check + assemble I/O across independent product runtimes.  
**Protocol schema inventory:** **30** (24 baseline + 6 opt-in connect envelopes).

---

## Now (in progress)

Capability tracks in flight on the integration branch; land on `main` when their plans close.

| Capability track | What it gives integrators |
|------------------|---------------------------|
| Swift SPM iOS coverage | The `SpokeConnect` SPM product carries the `spoke_connectFFI.xcframework` — slices `macos-arm64`, `ios-arm64`, and `ios-arm64_x86_64-simulator` (arm64 + x86_64 lipo-combined) — covering macOS arm64 hosts, iOS devices, and iOS simulators on both Apple Silicon and Intel hosts, so iOS integrators link on device and both simulator hosts without running bindgen. macOS stays arm64-only |
| Pure-TS Noise transport (opt-in) | `@42ch/spoke-connect/noise` provides a pure-TypeScript Noise XX stack (X25519 + ChaCha20-Poly1305 + HKDF-SHA256) wire-compatible with the Rust libp2p reference, for integrators that need direct mesh interop; the default `.` / `./node` exports keep their thin dependency surface |
| CI identity-byte gate | The zero-dep connect identity proof runs in CI, catching drift between the TS identity derivation and the golden `peer_id` / JCS / Ed25519-signature vectors automatically |

**Durable decision (2026-08-02) — Knowledge Pack vs AssemblePacket; demote `modules.pack`:** A **Narrative Knowledge Pack** is a durable lore-library interchange pattern (full KnowledgeEntry + Relation atoms + product transport envelope). An **AssemblePacket** is an ephemeral `assemble` output (slim AI context projection). They share a vague “list of entries” shape but are **not** the same object and must not be merged. Pack catalog metadata (`title` / `version` / `creator`) is **product-envelope** — **not** `modules.*` on KnowledgeEntry or AssemblePacket. Keep `modules.activation` on KnowledgeEntry and `modules.placement` / `modules.activation_trace` on AssemblePacket. Handbook, triad ADR, CONCEPTS, docs, schemas descriptions, and companion fixture updated accordingly.

---

## Up next (planned)

| Slice | What ships | Trigger / notes |
|-------|------------|-----------------|
| **DHT discovery** | Libp2p Kademlia DHT peer discovery layered on the Noise mesh transport, for hosts that need peer routing beyond explicit relay/dial lists | A product host requires libp2p-mesh peer discovery; builds on the pure-TS Noise foundation and the Rust reference mesh |
| **iOS xcframework CI automation** | A CI job that assembles the multi-slice `spoke_connectFFI.xcframework` when the FFI surface changes, removing the maintainer manual-refresh step | Follows the three-slice matrix (`macos-arm64`, `ios-arm64`, `ios-arm64_x86_64-simulator`); the xcframework stays repo-committed between FFI-surface changes per [connect-binding-channels.md](specs/connect-binding-channels.md) |
| **Cross-language capability-token golden-vector TS-minted counterpart** | A TS-minted capability-token golden vector that the Rust verify path (and other bindings) round-trip, closing the current Rust-minted-only direction | Token auth is session-core parity; this adds the reverse-direction shared vector |
| **Integrator docs content** | Keep VitePress `docs/` aligned with the protocol SSOT; document the `./noise` subpath and the iOS xcframework slices in the connect how-to pages | EN+CN twin parity enforced by `tooling/docs/twin-parity.mjs`; CN terminology glossary: [docs-i18n-glossary.md](knowledge/conventions/docs-i18n-glossary.md) |
| **libp2p transitive vulnerability revisit** | Re-check yamux / hickory-proto pins in `Cargo.lock` when upstream libp2p ecosystem ships fixes | Last reviewed 2026-08-02 — no fix available on current libp2p 0.56 line |

---

## Done (delivered)

Newest first. Dates are delivery dates on `main`.

| When | Slice | What landed |
|------|-------|-------------|
| 2026-08-04 | Integrator docs Diátaxis restructure | `docs/` reorganized into four quadrants (Tutorials / How-to guides / Reference / Explanation) with EN↔CN 1:1 twin parity; conceptual content slimmed to key statements; Adapter implementation + Connect usage promoted to top-level how-to branches; consolidated Reference pages carry field tables inline (no internal-spec links); maintainer release-cut procedure cross-linked from sidebar to `CONTRIBUTING.md` |
| 2026-08-04 | connect TS↔Rust session-core full parity | Closed the four TS↔Rust asymmetries so the parity contract holds without footnotes: `peer_id` decode length cap (128 chars) on Rust; canonical base64url signature round-trip check on Rust verify; Rust issuance-side fail-fast on empty `capabilities` / `exp` inside clock-skew window / `iat` beyond skew (via required 3rd positional `now: u64` arg on `issue_capability_token`); intentional helper boundary documented (TS thin `Session` / `negotiatedCapabilities` / `generateNonce` stay client-side; Rust transport layer owns equivalents). Cross-language golden vector + 96 Rust + 17 integration + 120 TS tests green |
| 2026-08-04 | connect Path B hardening | Kotlin binding groupId `io.github.42ch-dev` → `dev.42ch` (reverse-DNS of `42ch.dev`); cross-language golden hello vector consolidated into single JSON SSOT (`crates/spoke-connect/tests/fixtures/golden-hello.json`) with registered copies + sync gate (`tooling/connect/golden-vector-sync.mjs`); docs CI twin-parity (`tooling/docs/twin-parity.mjs`) + dead-link gate (`tooling/docs/deadlink-check.mjs`) wired into `docs.yml` |
| 2026-08-03 | Swift + Kotlin connect bindings | Root `Package.swift` library product `SpokeConnect` + macOS xcframework + golden-parity smoke; Kotlin binding via JNA natives + golden-parity smoke |
| 2026-08-03 | Python connect binding | Packable `pyproject.toml` + platform wheels (`manylinux_2_35_x86_64`, `macosx_11_0_arm64`, `win_amd64`) + golden-parity smoke |
| 2026-08-03 | C# connect binding | Packable net8.0 project + multi-RID `ffi` matrix + `PackageReference` DX in docs/Smoke + golden-parity smoke (peer_id + hello signature + verify + tamper, protocol=1, plus allowlist / dispatch / capability / correlation / nonce / sequence lifecycle coverage — 11 checks) via a **vendored `uniffi-bindgen-cs` fork retargeted to uniffi 0.32** ([connect-csharp-binding.md](specs/connect-csharp-binding.md)) — drop the fork when upstream tags a uniffi 0.32+ release |
| 2026-08-03 | Integrator docs EN + CN i18n | VitePress locale support (`docs/.vitepress/config.mts` `locales`: EN root + `zh/` CN tree) + locale switch + CN twins of all integrator pages; SSOT stays English (`.mstar/specs/`); CN terminology glossary promoted to [knowledge](knowledge/conventions/docs-i18n-glossary.md); `vitepress build` green bilingually |
| 2026-08-03 | spoke-connect TS↔Rust session-core parity (verify side) | Port Rust `capability_token` module + `tokenAuthorizesOp` grant-membership + reverse peer_id to `@42ch/spoke-connect`; **verify-side** proof-shape u64 parity + canonical-sig + base58 cap; **issuance-side** shape + current-time validation (fail-fast — rejects claims the verifier would deterministically reject, e.g. already-expired `exp` or non-u64 timestamps); Rust-produced golden vector + cross-language verify test; TS↔Rust parity rule recorded in root `AGENTS.md` + [spoke-connect-ts-route.md](specs/spoke-connect-ts-route.md); 120 TS tests green |
| 2026-08-02 | Demote `modules.pack` (decision + SSOT) | **Pack ≠ AssemblePacket**; pack catalog → product transport envelope; remove `modules.pack` from ModuleMap examples (triad ADR, Knowledge Pack handbook, CONCEPTS, docs, schema descriptions, companion fixture); keep `modules.activation` on KE and assemble recipes on AssemblePacket |
| 2026-08-02 | spoke-connect multi-language deepen + integrator docs site | C# binding feasibility gate (decision record); `@42ch/spoke-connect` package surface locked (exports map, subpaths); **integrator docs site** (VitePress `docs/`, SSOT-linked pages); docs-site SSOT-link pattern captured as knowledge |
| 2026-08-01 | spoke-connect multi-language SDK start | `@42ch/spoke-connect` first slice (pure-TS-minimal + golden parity + WS interop); uniffi Swift sync-core skeleton; capability-token auth normative; mDNS same-LAN discovery (non-default `mdns` feature) |
| 2026-08-01 | spoke-connect multi-language pre-design | Three-layer embedding contract; pure `crates/spoke-connect` session core + golden vectors; TS route + identity-byte parity proof |
| 2026-07-31 | spoke-connect foundation | `schemas/connect/` six envelopes + capability flag; JCS hello auth; Rust reference spike; codegen inventory 24 → 30 |
| 2026-07-30 | `modules` wire + assemble recipes | `ModuleMap` optional on KE + AssemblePacket (`narrative-modules`); merge/preserve helpers; recipes handbook; Domain Profiles for lore-activation + Knowledge Pack (Seed/Pool) |
| 2026-07-30 | Naming triad + Scope.extensions | Core / `modules.*` / `extensions.<product>` ADR + CONCEPTS; optional `Scope.extensions`; companion pack fixture |
| 2026-07-29 | Relation OCC parity | `Relation.revision` + OCC-deep `orchestrateRelate` / `orchestrate_relate` |
| 2026-07-28 | Beat-assist protocol slice | Narrative-structure Domain Profile + Harbor beat chain + timeline sequence helpers |
| 2026-07-27 | Host capability collaboration | `HostCapabilityManifest` + baseline `HostManifestPort`; toy-world dual manifests |
| 2026-07-27 | ToyWorldAdapter + Adapter aliases | TS/Rust reference adapters under `fixtures/toy-world/`; composed-port aliases in ops packages |
| 2026-07-26 | Adapter ports + orchestration | Capability-sliced ports + `orchestrate*` / `orchestrate_*` |
| 2026-07-26 | L2 typed closed body + traits | Closed `body` + trait helpers |
| 2026-07-24 | Optional Fork (`l5-fork`) / Computable (`l2-computable`) | Fork ids on TimelineEvent; state/computable + project/compute ops |
| 2026-07-23 | KnowledgeEntry terminology + ops library + fixtures + v0.1 bootstrap | Wire naming lock; operations first slice; fixture harness ownership; schemas SSOT + codegen |

### Baseline inventory (as of last Done)

| Area | Status |
|------|--------|
| Data wire | KnowledgeEntry (closed L2 `body`; optional `modules` under `narrative-modules`; optional `body.state`/`computable` under `l2-computable`), Relation (optional `revision` OCC), SourceAnchor, Finding, AssemblePacket (optional `modules`), HostCapabilityManifest, Rule, TimelineEvent (+ optional fork fields), `extensions` |
| Ops wire | upsert / promote / relate / check / assemble (+ Scope, error-envelope); optional project / compute (`l2-computable`). `relate` OCC-deep-integrated with upsert |
| Ops library | Pure TS + Rust helpers + six baseline port families (incl. `HostManifestPort`) + `*Adapter` aliases + injection orchestration; `mergeModuleMaps` / `preserveModuleMaps` |
| Connect wire (opt-in) | `schemas/connect/` six envelopes; hello JCS + auth paths; `@42ch/spoke-connect` (TypeScript language-native client) + `spoke-connect` Rust reference; native bindings available for C#, Kotlin, Swift, Go, Python per [connect-binding-channels.md](specs/connect-binding-channels.md) |
| Fixtures | `fixtures/toy-world/` samples + harness + reference adapters; pack companion under `proposed/` |
| Codegen / CI | Schema inventory **30**; `verify-codegen`; Pages docs workflow |
| Specs | Umbrella, layers, data-model, ops, operations, extension-modules, Domain Profiles, assemble recipes, connect family under `.mstar/specs/` |

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
- Product activation engines, prompt-slot UIs, assembly inspectors (consumer repos)

---

## Pointers

| Doc / path | Use for |
|------------|---------|
| [`STRATEGY.md`](../STRATEGY.md) | Why / principles / three-column architecture |
| [`CONCEPTS.md`](../CONCEPTS.md) | KnowledgeEntry / TimelineEvent spelling + dual-concern |
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Normative umbrella |
| [`.mstar/specs/spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) | L0–L8 + capability levels |
| [`.mstar/specs/spoke-extension-modules.md`](specs/spoke-extension-modules.md) | Core / modules / extensions triad |
| [`.mstar/specs/domain-profile-lore-activation.md`](specs/domain-profile-lore-activation.md) | Lore-activation (`modules.activation`) |
| [`.mstar/specs/domain-profile-narrative-knowledge-pack.md`](specs/domain-profile-narrative-knowledge-pack.md) | Knowledge Pack + Seed/Pool |
| [`.mstar/specs/assemble-module-recipes.md`](specs/assemble-module-recipes.md) | AssemblePacket placement / activation_trace |
| [`.mstar/specs/spoke-connect.md`](specs/spoke-connect.md) | Opt-in connect wire family |
| [`schemas/`](../schemas/) | Wire SSOT |
| [`packages/spoke-operations/`](../packages/spoke-operations/) | Pure behavior library (TypeScript) |
| [`crates/spoke-operations/`](../crates/spoke-operations/) | Pure behavior library (Rust) |
| [`fixtures/toy-world/`](../fixtures/toy-world/) | Protocol samples, harness, and reference Adapter examples |
| [`docs/`](../docs/) | Integrator VitePress site (SSOT links into specs) |
