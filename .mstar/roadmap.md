# SPOKE Roadmap

> Living **project** roadmap (tracked result). Strategy and architecture live in [`STRATEGY.md`](../STRATEGY.md) and [`.mstar/specs/`](specs/). Per-slice execution detail stays in local `delivery-compass.md` (process; gitignored).
>
> **Scope:** this repository only — protocol wire, pure ops libraries, fixtures, connect SDK prep, integrator docs. Consumer product engines (e.g. activation runtimes) are not scheduled here.

**Updated:** 2026-08-03  
**North star:** Cross-product KnowledgeEntry dialect for check + assemble I/O across independent product runtimes.  
**Lockstep SemVer on main:** `0.7.1` · protocol schema inventory **30** (24 baseline + 6 opt-in connect envelopes).

---

## Now (in progress)

No active delivery slice on `main`. See **Up next** for planned work; durable contract facts live under **Done → Baseline inventory** and `.mstar/specs/`.

**Durable decision (2026-08-02) — Knowledge Pack vs AssemblePacket; demote `modules.pack`:** A **Narrative Knowledge Pack** is a durable lore-library interchange pattern (full KnowledgeEntry + Relation atoms + product transport envelope). An **AssemblePacket** is an ephemeral `assemble` output (slim AI context projection). They share a vague “list of entries” shape but are **not** the same object and must not be merged. Pack catalog metadata (`title` / `version` / `creator`) is **product-envelope** — **not** `modules.*` on KnowledgeEntry or AssemblePacket. Keep `modules.activation` on KnowledgeEntry and `modules.placement` / `modules.activation_trace` on AssemblePacket. Handbook, triad ADR, CONCEPTS, docs, schemas descriptions, and companion fixture updated accordingly.

---

## Up next (planned)

| Slice | What ships | Notes |
|-------|------------|-------|
| **spoke-connect uniffi bindings** | Binding matrix (**C#, Go, Python, Swift, Kotlin**) on four publish channels: **GitHub Packages** (C# NuGet landed, Kotlin Maven in progress), **SPM git** + **Go modules git** (Swift skeleton landed; Go in progress), **PyPI** OIDC (**Python landed** — `spoke-connect` platform wheels + `publish-pypi`). Packaging contract: [connect-binding-channels.md](specs/connect-binding-channels.md), [connect-publish-strategy.md](specs/connect-publish-strategy.md) | Community bindgen tools verified against the uniffi 0.32 pin before each slice; vendored-fork path available ([connect-uniffi-bindgen-fork.md](knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md)) if a tool lags |
| **Docs CI hardening** | Docs twin-parity check (EN↔CN page-count drift) + dead-link gate for the VitePress site | **Trigger:** next `docs/**` slice; pairs with the cross-platform/CI slice above. EN+CN locale twins already landed |
| **Integrator docs content** | Keep VitePress `docs/` + GitHub Pages aligned with `.mstar/specs/` SSOT (site scaffold already on main; EN root + `zh/` CN twins live) | Content/SSOT sync as profiles and connect docs evolve; CN terminology glossary: [docs-i18n-glossary.md](knowledge/conventions/docs-i18n-glossary.md) |
| **libp2p transitive vulnerability revisit** | Re-check yamux / hickory-proto pins in `Cargo.lock` when upstream libp2p ecosystem ships fixes | Last reviewed 2026-08-02 — no fix available on current libp2p 0.56 line |

---

## Done (delivered)

Newest first. Dates are delivery dates on `main`.

| When | Slice | What landed |
|------|-------|-------------|
| 2026-08-03 | Python `spoke-connect` PyPI | Packable `pyproject.toml` + platform wheels (`manylinux_2_17_x86_64`, `macosx_11_0_arm64`, `win_amd64`) + golden-parity smoke + `publish-pypi` Trusted Publishing on `release.yml`; lockstep row in `spoke-version-release.md` |
| 2026-08-03 | C# `42ch.Spoke.Connect` GitHub Packages NuGet | Packable net8.0 project + multi-RID `ffi` release matrix + `publish-nuget` on `release.yml`; PackageReference DX in docs/Smoke; four-channel binding publish strategy locked (GitHub Packages NuGet/Maven, SPM git, Go modules git, PyPI) |
| 2026-08-03 | Integrator docs EN + CN i18n | VitePress locale support (`docs/.vitepress/config.mts` `locales`: EN root + `zh/` CN tree) + locale switch + CN twins of all 17 integrator pages (index, 7 guide, 4 profiles, 3 connect, packages, release); SSOT stays English (`.mstar/specs/`); CN terminology glossary promoted to [knowledge](knowledge/conventions/docs-i18n-glossary.md); `vitepress build` green bilingually, Pages deploy covers `/zh/` |
| 2026-08-03 | spoke-connect TS↔Rust session-core parity | Port Rust `capability_token` module + `tokenAuthorizesOp` grant-membership + reverse peer_id to `@42ch/spoke-connect`; **verify-side** proof-shape u64 parity + canonical-sig + base58 cap; **issuance-side** shape + current-time validation (fail-fast — rejects claims the verifier would deterministically reject, e.g. already-expired `exp` or non-u64 timestamps); Rust-produced golden vector + cross-language verify test; TS↔Rust parity rule recorded in root `AGENTS.md` + [spoke-connect-ts-route.md](specs/spoke-connect-ts-route.md); 120 TS tests green |
| 2026-08-03 | C# binding landed | Generated `spoke_connect.cs` binding + net8.0 golden-parity smoke (peer_id + hello signature + verify + tamper, protocol=1, plus allowlist / dispatch / capability / correlation / nonce / sequence lifecycle coverage — 11 checks) via a **vendored `uniffi-bindgen-cs` fork retargeted to uniffi 0.32** ([connect-csharp-binding.md](specs/connect-csharp-binding.md)) — drop the fork when upstream tags a uniffi 0.32+ release |
| 2026-08-02 | Demote `modules.pack` (decision + SSOT) | **Pack ≠ AssemblePacket**; pack catalog → product transport envelope; remove `modules.pack` from ModuleMap examples (triad ADR, Knowledge Pack handbook, CONCEPTS, docs, schema descriptions, companion fixture); keep `modules.activation` on KE and assemble recipes on AssemblePacket |
| 2026-08-02 | spoke-connect multi-language deepen & delivery prep | C# binding feasibility gate (decision record); publish strategy locked (Stage 1 = npm `@42ch/spoke-connect` + Pages docs); TS published-shape prep; **integrator docs site** (VitePress `docs/`, SSOT-linked pages, GitHub Pages deploy); knowledge `connect-publish-staging` + `integrator-docs-site-ssot-links` |
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
| Connect wire (opt-in) | `schemas/connect/` six envelopes; hello JCS + auth paths; published `@42ch/spoke-connect` + crates.io `spoke-connect`; Path B bindings on four channels (C# NuGet + Python PyPI landed; Kotlin Maven, SPM, Go modules per [connect-binding-channels.md](specs/connect-binding-channels.md)) |
| Fixtures | `fixtures/toy-world/` samples + harness + reference adapters; pack companion under `proposed/` |
| Codegen / CI | Schema inventory **30**; `verify-codegen`; release lockstep; Pages docs workflow |
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
| [`.mstar/specs/spoke-version-release.md`](specs/spoke-version-release.md) | Lockstep SemVer, tags, CI-gated GitHub Release, registry publish |
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
