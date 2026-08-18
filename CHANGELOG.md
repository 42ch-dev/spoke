# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release notes for GitHub Releases are extracted from the matching version section here.
## [0.11.0] - 2026-08-18


### Added

- **connect:** Tool FFI surfaces — reverse invoke + responder/tool-serving across five native bindings (#83)

- **connect:** ToolPort full-surface reconciliation + release/CI hardening round (#85)

- **connect:** Optional-port remote capability full-stack (library + FFI + five bindings + demo + docs) (#86)


### Internal

- Update gitignore

- Update

## [0.10.0] - 2026-08-14


### Added

- L5-mind capability (mind-axis) — MindState wire + modules dialects + validators


### Documentation

- Document l5-mind capability in READMEs (#80)

- **knowledge:** Precipitate mind-axis ops extraction gate

- **knowledge:** Align mind-axis ownership frontmatter with schema


### Fixed

- **connect:** Protocol_version_mismatch alignment + pnpm allowBuilds fix (#79)


### Internal

- Post-merge close record refinements (truthful outcome + reusable gotcha)

## [0.9.2] - 2026-08-07


### Added

- **examples:** Scaffold connect demo server with mock engine and adapter

- **examples:** Add connect responder host with spec-derived envelope auth

- **examples:** Add websocket transport, demo clis, and e2e gate

- **examples:** Merge connect integration demo (mock inference host + RemoteAdapter client)

- **connect:** Pass session-authenticated peer to invoke handler (additive InvokeHandlerV2)


### Changed

- **connect:** **BREAKING:** Remove mdns feature (explicit peering only)


### Documentation

- **roadmap:** Add connect integration demo and remote adapter tutorial slices

- **tutorials:** Add remote adapter integration tutorial

- **tutorials:** Add cn twin and sidebar entries for remote adapter tutorial

- **examples:** Fix connect-demo readme build set and link shipped tutorial

- **tutorials:** Fix cn wording, first-use glosses, and error-kind ellipsis

- **roadmap:** Mark connect demo + remote adapter tutorial delivered; add responder compound knowledge


### Fixed

- **examples:** Settle ws bind failure, add deny-path tests, and serve-loop logs

- **examples:** Close demo host connection on unparseable inbound frame

- **examples:** Latch closed on remote ws close so recv rejects promptly

- **connect:** Add missing invoke_handler_v2 field to README usage sample

## [0.9.1] - 2026-08-06


### Changed

- **connect:** Add TS-minted capability-token golden vector + drift guard

- **connect:** Verify TS-minted capability-token golden vector in Rust

- **connect:** Consolidate panic helper + cfg(test) injection split


### Documentation

- **connect:** Sync tracked specs, roadmap, and knowledge with current connect facts

- **connect:** State bidirectional capability-token golden-vector parity

- **connect:** Roadmap capability-token row to Done + knowledge SSOT model sync

- **connect:** Close binding Transport contract + feature-pair docs; polish residual doc items

- **connect:** Rework connect family for the RemoteAdapter era (EN)

- **connect:** Fix Rust adapter snippet + align version-mismatch row with shipped surface

- **connect:** CN twins for the connect family rework + glossary sync

- **connect:** Journey-ordered sidebar + anchor fixes + integrator-docs roadmap row

- **connect:** Fix QC findings — SpokeResult consumption, Dial triggers, smoke import, zh freeze, anchors

- Refresh connect publish facts and index GitNexus


### Fixed

- **connect:** Tighten golden-vector gate bounds + byte-identical drift guard

- **connect:** Contain FFI block_on panics with catch_unwind

- **connect:** Propagate smoke-host start panics as FfiError; thread-key panic injection

- **connect:** Binding artifact fixes for residual sweep task 2

- **ci:** Lockstep spoke-operations path dep for v0.9.0 release

- **connect:** Restore test fixture dev-dep; de-fixture ffi-smoke-host adapter for publish-safe smoke builds


### Internal

- **connect:** Cover capability-token family in golden-vector sync gate

- **connect:** Drop dead ffi helper and unused imports

- **docs:** Enforce fragment resolution + structural twin parity in docs gates

- Ignore local GitNexus skill install


### merge

- **main:** Sync v0.9.0 release state for task-5 CI repair

- **main:** Sync v0.9.0 release CI repairs

## [0.9.0] - 2026-08-06


### Added

- **connect:** Reject non-canonical base64url signatures

- **connect:** Add issuance-time validation to capability_token

- **connect:** Ship iOS slices in committed Swift xcframework + simulator smoke

- **connect:** Add Noise crypto primitives (X25519/HKDF/ChaCha20-Poly1305)

- **connect:** Add Noise XX handshake state machine

- **connect:** Add libp2p Noise framing + NoiseHandshakePayload

- **connect:** Expose opt-in ./noise subpath; add bundle-isolation gate

- **connect:** Add RemoteAdapter + Transport interface (TS)

- **connect:** Add RemoteAdapter + Transport (Rust parity)

- **connect:** **BREAKING:** Bind responder hello to initiator nonce (dial binding)

- **connect:** Protocol_version 2 envelope authentication + multi-peer routing design

- **connect:** Protocol v2 envelope-auth signature fields in wire schemas

- **connect:** TS envelope-auth authenticate/verify + sequence peek

- **connect:** Rust envelope-auth authenticate/verify + sequence peek

- **connect:** Envelope-auth golden vectors + sync gate

- **connect:** TS RemoteAdapter envelope-auth enforcement + loopback

- **connect:** Rust RemoteAdapter inbound envelope-auth verify + loopback host enforcement

- **connect:** TS multi-peer capability router + selection + BaselinePorts

- **connect:** Rust multi-peer capability router + selection + BaselinePorts parity

- **connect:** TS hostId empty-string default parity + surface handoff

- **connect:** Add cdylib-owned tokio runtime for FFI surface

- **connect:** Add foreign-callback Transport interface for FFI

- **connect:** Add sync RemoteAdapterFFI object over cdylib runtime

- **connect:** Complete FfiError mapping to full D7 surface

- **connect:** Regenerate Swift binding + add RemoteAdapterFFI loopback smoke

- **connect:** Regenerate Kotlin binding + add RemoteAdapterFFI loopback smoke

- **connect:** Regenerate Python binding + add RemoteAdapterFFI loopback smoke

- **connect:** Regenerate C# binding + RemoteAdapterFFI loopback smoke

- **connect:** Regenerate Go binding + RemoteAdapterFFI loopback smoke

- **connect:** Add sync MultiPeerRouterFFI over cdylib runtime

- **connect:** MultiPeerRouterFFI aggregation + lifecycle surface

- **connect:** Regenerate bindings for MultiPeerRouterFFI + multi-peer routing smoke


### Changed

- **connect:** Make non-canonical-sig mutation robust across sigs

- **connect:** Add rust-libp2p Noise XX golden-transcript interop

- **operations:** Migrate TS adapter ports + orchestrate to async

- **operations:** Migrate Rust adapter ports + orchestrate to async

- **operations:** Fmt async-trait test stubs

- **connect:** Add 5-field responder golden vector + TS↔Rust interop

- Drop dial-binding ffi.rs doc comments to restore uniffi metadata

- **connect:** Cast OpaqueJson auth inputs in envelope-auth tests

- **connect:** TS dual-peer loopback routing proof

- **connect:** QC wave-1 fixes for multi-peer router

- **connect:** Complete §6/§7 surface verification for multi-peer router


### Documentation

- **connect:** Replace Path A/B jargon with embedding vocabulary

- **connect:** Map Path A/B to consumer embedding vocabulary in SSOT

- **specs:** Refresh publish-state facts and refine TS↔Rust parity surface

- **connect:** Correct base64 0.22 strict-decode comment in verify_capability_token

- **connect:** Add canonical base64url signature rule to capability-token spec

- **tutorials:** Add install-and-first-entry and first-connect-session (EN+CN)

- **how-to:** Add implement-adapter and orchestrate-ops (EN+CN)

- **how-to:** Add connect-ts-client and connect-native-bindings (EN+CN)

- **how-to:** Add walk-toy-world reference adapter walkthrough (EN+CN)

- **reference:** Consolidate protocol, data-model, ops, connect field tables (EN+CN)

- **explanation:** Consolidate concepts and domain-profiles key statements (EN+CN)

- **nav:** Restructure sidebar and nav into Diátaxis quadrants (EN+CN)

- **index:** Rewrite hero + feature cards for Diátaxis framing; drop normative-refs section; fix README cross-links

- Remove old guide/, profiles/, connect/ trees after consolidation

- Drop spec links from kept quick-start/versioning pages; use external canonicals

- **reference:** Rename embedding-model table header on connect pages

- **connect:** Redirect stale native-bindings links to new how-to page

- **roadmap:** Drop tag/release/publish framing; keep functional points only

- **connect:** Record transport-adapter parity boundary; refresh roadmap up-next

- **connect:** Document iOS coverage of the Swift xcframework matrix

- **connect:** Record opt-in ./noise subpath in route spec + integrator how-to

- **connect:** Promote frozen Noise XX wire contract to tracked spec

- **connect:** Record connect identity proof CI gate

- **connect:** Note git-lfs tracking of the Swift xcframework static libs

- **connect:** Seed RemoteAdapter ADR + async-operations roadmap slice

- **operations:** Rewrite port-contract spec to async current-facts

- **operations:** Rewrite spec signature tables to async envelope

- Update consumer docs to async orchestrate/ports call shapes

- **connect:** Finalize RemoteAdapter ADR to current-facts

- **connect:** Finalize envelope-auth v2 normative section + checklist

- **connect:** Note envelope-auth FFI deferral in ffi.rs facade doc

- **connect:** Specs state envelope-auth enforcement as shipped fact (Task 4)

- **connect:** Multi-peer router integrator how-to EN + zh twin

- **connect:** Spec + roadmap state multi-peer surface as shipped fact

- **connect:** Roadmap shipped rows + affirmative multi-peer how-to (QC fix wave 1)

- **connect:** Record RemoteAdapterFFI binding facade surface

- **connect:** Sync specs, roadmap, integrator docs for native RemoteAdapter FFI


### Fixed

- **connect:** Cap peer_id decode length to prevent O(n²) DoS

- **connect:** Align swift build target-assert with nightly + ignore .swiftpm

- **connect:** Single-shot handshake finish; gate built dist shape

- **ci:** Fail-open connect-identity gate on path-filter error

- **deps:** Reconcile esbuild specifier in lockfile (>=0.28.1) to unbreak frozen-lockfile CI

- **deps:** Bump fast-uri/brace-expansion/postcss to clear Dependabot advisories

- **connect:** Make RemoteAdapter lifecycle methods private (encapsulation)

- **connect:** Validate RemoteAdapter success payload + error-path tests

- **connect:** Drop dead negotiated_capabilities field from Rust adapter

- **connect:** Harden RemoteAdapter state encapsulation + close-rejection handling

- **connect:** Close handler accepts thenable Transport.close() rejections

- **connect:** Reject responder hello without peer_nonce (fail-closed dial binding)

- **connect:** Catch synchronous Transport.close() throws in closeSession

- **connect:** Replace hard-coded test nonces with runtime-generated values

- **connect:** Envelope-auth v2 hardening — auth-before-advance, 86-char signatures, field-table sync

- **connect:** Invoke-request auth binding note + signature Type 86-char

- **connect:** Accept OpaqueJson auth on invoke-request verify + CoreError for bad verify key

- **connect:** Reject odd-length hex in golden sync gate

- **connect:** Sign remote-adapter outbound invoke requests

- **connect:** Verify invoke envelope auth before advancing inbound

- **connect:** Guard deferred sends against timeout + register connect-client pending before sign

- **connect:** Close session on skipped timed-out send (prevent sequence poisoning)

- **connect:** Graceful same-sequence duplicate handling + response-branch discriminator parity (P3)

- **connect:** QC wave 1 — outbound send serialization, crypto-verify kind, host gate + decode grace

- **connect:** Remove iteration-id reference from multi-peer-router doc comment

- **toy-world:** Add v2 signature to connect fixture envelopes

- **connect:** RemoteAdapterFFI close enters runtime; manifest error mapping; dedup loopback oracle

- **connect:** Correct pub(crate) and test-comment accuracy in FFI/oracle (QC S-003/S-004)

- **connect:** Gate smoke-host behind ffi-smoke-host; rebuild xcframework with remote-adapter

- **connect:** Build-connect-ffi includes remote-adapter; Kotlin README bindgen row

- **connect:** Ci.yml tests ffi,remote-adapter; binding READMEs feature-pair docs

- **connect:** Go darwin_amd64 + release dylib; fork patch SetTemplate.go; C# CI + binding docs ffi,remote-adapter

- **connect:** Roadmap pipe; framing docs; Go generator spec sync

- **connect:** How-to anchors; roadmap stale fact; Transport wording; D12 acceptance

- **connect:** Gate test_support oracle on remote-adapter (CI no-feature build)


### Internal

- **connect:** Gate connect identity proof in CI

- **iteration:** Close connect binding matrix + TS Noise + identity gate — compound round, roadmap update

- **lfs:** Track Swift xcframework static libs via git-lfs (forward-only)

- **lfs:** Convert committed Swift xcframework static libs to LFS pointers

- **connect:** Test remote-adapter feature in CI

- **iteration:** Close async-operations + RemoteAdapter bridge — compound round, roadmap update

- Build spoke-operations before spoke-connect dist smoke (remote-adapter dep)

- **iteration:** Compound — sync block-on-async FFI bridge knowledge doc

- Replace greptile submodule with npx skills add (check-pr, cli-review, greploop)

- **connect:** Add binding-regen drift check for Swift/Python generated sources


### build

- **connect:** Generalize Swift xcframework build to iOS target matrix

## [0.8.2] - 2026-08-03


### Added

- **connect:** Rename Kotlin Maven groupId to dev.42ch


### Changed

- **connect:** Consolidate golden hello vector into shared JSON SSOT


### Fixed

- **connect:** Include golden-hello fixture in C# PackageReference smoke

- **connect:** Use Link to place C# golden fixture under fixtures/ subdir

- **docs:** Derive site base from VitePress config in dead-link gate


### Internal

- **docs:** Add EN↔CN twin-parity check and dead-link gate

- **connect:** Wire golden-vector-sync gate into ci.yml

## [0.8.1] - 2026-08-03


### Fixed

- **release:** Correct Maven and PyPI action pins (#56)

## [0.8.0] - 2026-08-03


### Added

- **connect:** Publish 42ch.Spoke.Connect NuGet to GitHub Packages

- **connect:** Add vendored uniffi-bindgen-go 0.32 fork recipe

- **connect:** Add Go bindings and golden-parity smoke

- **connect:** Complete Go module layout and README

- **connect:** Merge Go spoke-connect module export

- **connect:** Add Python binding module and golden smoke

- **connect:** Add spoke-connect PyPI packaging layout

- **release:** Add publish-pypi Trusted Publishing job

- **connect:** Merge Python spoke-connect PyPI packaging

- **connect:** Add Swift SPM package with committed bindings and xcframework

- **connect:** Add Kotlin binding with post-generate patch and golden smoke

- **connect:** Add Kotlin Maven publish to GitHub Packages

- **connect:** Merge Swift SPM and Kotlin Maven packaging


### Documentation

- **readme:** Reframe for third-party integrators

- **connect:** Lock four-channel binding publish matrix

- **connect:** Align binding install prose with channel readiness

- **connect:** Update Go binding consumer docs

- **connect:** Document spoke-connect PyPI install path

- **specs:** Register Python PyPI in lockstep and publish SSOT

- **connect:** Mark Swift SPM and Kotlin Maven as landed

- **specs:** Add Kotlin Maven lockstep row and landed bindings

- **knowledge:** Compound Path B Kotlin Throwable message pattern

- **connect:** Add per-language package usage guides

- **connect:** Document Go linux/windows native staging

- **connect:** Tie Go natives to in-tree tag layout

- **connect:** Document Go linux/windows native replace staging

- **connect:** Document Go Windows DLL and NuGet native source


### Fixed

- **ci:** Pin csharp-connect actions and smoke via PackageReference

- **connect:** Align Go import path and dylib @rpath install names

- **connect:** Use manylinux wheel tag and pre-publish verify

- **connect:** Avoid duplicate JNA natives in Kotlin jar

- **ci:** Pin Gradle 9.6.1 for publish-maven

- **connect:** Align Python wheel tag with ubuntu-22.04 glibc floor


### Internal

- **connect:** Drop Python pycache from binding tree

## [0.7.1] - 2026-08-03


### Changed

- **connect:** Rename npm package @42ch/spoke-connect-ts -> @42ch/spoke-connect


### Documentation

- **connect:** Update READMEs for published @42ch/spoke-connect + spoke-connect

- **connect:** Update crate README + lib.rs — published, not workspace-private (Greptile P2)


### Fixed

- **docs:** Override vite to ^6.4.3 — clear Dependabot dev-server advisories

- **connect-ts:** Tsconfig paths for self-import — resolve src, not dist (CI typecheck)

- **connect-ts:** Vitest alias for self-import + exports->dist (CI typecheck+test pass)

- **connect-ts:** Reorder vitest alias — /node before bare name (prefix match)


### Internal

- **release:** Publish @42ch/spoke-connect + spoke-connect via release.yml (Stage 1+2)

## [0.7.0] - 2026-08-03


### Added

- **schemas:** Spoke-connect interaction wire family and Rust libp2p spike (#37)

- **connect:** Add pure session core with golden vectors

- **connect-ts:** Add workspace-private TS connect package foundation

- **connect-ts:** Add one-JSON-per-message WebSocket framing

- **connect-ts:** Port pure session-core guards from Rust core

- **connect-ts:** Add minimal WebSocket connect client and two-node interop test

- **connect-ts:** Wire package into root CI gates and lockstep versioning

- **spoke-connect:** Uniffi Swift-first sync-core binding surface

- **spoke-connect:** Export is_allowlisted through uniffi facade

- **connect:** Capability-token issue/verify core rules

- **connect:** Capability-token challenge/response and invoke auth gate

- **connect:** Mdns same-LAN discovery behind non-default feature

- **connect-ts:** Declare exports map and publish metadata (published-shape prep)

- **connect-ts:** Port capability-token auth for Rust parity

- **connect-ts:** TokenAuthorizesOp + proof-shape guard + TS/Rust parity rule

- **connect:** Land C# binding + golden smoke via vendored uniffi-bindgen-cs fork

- **docs:** Add VitePress EN root + zh CN locale infrastructure


### Changed

- **connect:** Cut transport over to the pure session core

- **connect-ts:** Source PROTOCOL_VERSION from core version module

- **connect-ts:** Use schema-conformant manifests for wire tests

- **spoke-connect:** Add Swift smoke asserting golden parity for the uniffi facade

- **spoke-connect:** Fix rustfmt drift in golden canonical bytes test

- **spoke-connect:** Rustfmt the malformed-hello test

- **connect:** Disable mdns autodial in two-node exchange tests

- **connect:** Deterministic mdns auth-interplay and helper coverage

- **connect:** Document mdns default-feature tripwire guard

- **connect-ts:** Smoke-test both package exports map subpaths


### Documentation

- **roadmap:** Remove non-repo and release-process entries; keep functional delivery history

- **roadmap:** Restore integrator docs site (GitHub Pages) as planned slice

- **knowledge:** Fix schema count in codegen-pipeline index

- **specs:** Connect embedding, identity binding, and ops dispatch contract

- **specs:** Spell multihash wire bytes for connect peer_id

- **schemas:** Document connect peer_id Ed25519 PeerId derivation

- **connect:** Record binding facade decision and target-language matrix

- **connect:** Tighten core purity docs, spec cross-refs, and golden-vector notes

- **connect:** Lock pure-TS-minimal TS route with identity proof

- **connect:** Soften informative TS-route wording for QC nits

- **knowledge:** Fix register in connect spike patterns

- **connect-ts:** Add first-slice package README

- **connect-ts:** State scope affirmatively in README

- **connect-ts:** State engine floor and affirm scope in README

- **specs:** List connect-ts package in lockstep surface table

- **spoke-connect:** Mark Swift sync-core binding skeleton as landed

- **spoke-connect:** Warn about default-build cdylib clobber in swift smoke

- **connect:** Normative capability-token auth method

- **connect:** Capability-token auth usage, spec cosmetics, CONCEPTS entry

- **connect:** Capability-token docs hygiene — usage block, session-grant lifetime, provider contract

- **agents:** Standardize local Rust toolchain on nightly

- **agents:** Honor -Zno-embed-metadata via nightly, forbid flag-clearing workarounds

- **connect:** MDNS discovery section — feature usage, autodial, bounds

- **connect:** MDNS dial scheduling and amplification boundary

- **connect:** Fix discovery spec link; affirmative mDNS wording

- **connect:** Fix spec relative links in crate README

- **connect:** Fix remaining spec relative links in crate README

- **connect:** Affirmative register pass on crate README

- **connect:** Align toolchain guidance with nightly convention

- **roadmap:** Confirm pure-TS path for TS SDK; reorder uniffi binding targets (C#, Go, Python, Swift, Kotlin)

- **roadmap:** Track libp2p transitive vulnerability revisit on ecosystem bump

- Fix stale publish and binding-matrix statements

- **connect:** Defer C# binding, record T1 version-gap verdict

- **roadmap:** Mark C# uniffi bindings deferred pending bindgen-cs uniffi 0.32

- **connect:** List TokenInvalid in Swift CoreError mapping

- **connect:** Fix broken links and clarify C# deferred state

- **connect:** Add publish strategy decision record and roadmap Stage 1 row

- **connect-ts:** Update usage to subpath imports and add publish guidance

- **connect:** Finalize evaluation evidence and Stage 1 steps in publish strategy

- **connect:** Mandate dist build and packed-tarball smokes for Stage 1

- **connect-ts:** Point publish guidance at strategy and add transition note

- **site:** Fill home and guide pages with SSOT-linked summaries

- **site:** Fill profile, connect, and package pages with SSOT-linked summaries

- **site:** Trim homepage to hero, CTAs, and SSOT links

- **readme:** Add pointer to integrator docs site

- **versioning:** Publish on every tag without -rc. prerelease segment

- **site:** Pin es2020 build target and record vite advisory disposition

- **readme:** Add local docs command and published Pages URL

- **site:** State current facts affirmatively across register

- **contributing:** Document integrator docs site workflow

- **roadmap:** Slim Now and schedule docs EN+CN i18n

- **modules:** Demote pack catalog from ModuleMap

- **knowledge:** Align companion-fixture index after pack sample removal

- **specs:** Corpus hygiene — fix anchor rot, state docs-site URL as current fact, tighten ts-route framing

- **i18n:** CN translations — guide and profiles pages

- **i18n:** CN translations — connect, packages, release pages

- **roadmap:** Reflect final TS parity + C# smoke coverage after Phase 5 hardening


### Fixed

- **connect:** Drop dead CoreError::NotAllowlisted variant and map arm

- **connect:** Gate inbound invokes on negotiated capabilities

- **connect:** Suppress CodeQL hard-coded-nonce alerts on golden fixtures

- **connect:** Dispatch product ops through configurable capability map

- **connect:** Restructure golden nonce fixtures for Rust CodeQL

- **connect:** Build golden nonce at runtime for CodeQL

- **connect-ts:** Assert Ed25519 WebCrypto probe contract directly

- **connect-ts:** Fail fast on handshake failure and keep invoke promise-only

- **connect-ts:** Close socket on dial failure and fail fast on malformed frames

- **connect-ts:** Mark OutboundSequence.setNext as test-only internal

- **spoke-connect:** Recover poisoned ffi locks and cover malformed hello verify

- **connect:** Clear pending challenge slot on auth outbound failure

- **connect:** Mdns review fixes — bounded candidates, passive example autodial

- **connect:** Auto-dial slot fairness — explicit preemption, slot-free retry

- **connect:** Run mdns behaviour construction tests in a tokio runtime

- **connect:** Bind pending connects only to the dial that originated them

- **docs:** Align guide, release, profile, and connect pages with SSOT wording

- **connect-ts:** QC tri fixes — proof-shape u64 parity, canonical sig, sign/return consistency, base58 cap, golden vector

- **connect:** C# binding review fixes — README apply path, roadmap, rename decision record

- **connect:** C# binding QC fixes — smoke coverage, locked regen, gitignore clone, roadmap

- **connect-ts:** Validate capability-token claims at issuance (Greptile P2)

- **connect-ts:** Issuance current-time check — reject already-expired/future-iat claims (Greptile)


### Internal

- **harness:** Ignore status-write lock file

- **iteration:** Close spoke-connect multi-language pre-design — compound round, roadmap update

- **connect-ts:** Align engines floor and correct WebCrypto comments

- **connect-ts:** Typecheck the package in CI

- Build and test spoke-connect ffi feature on ubuntu

- **connect:** Test spoke-connect with mdns feature in rust job

- **iteration:** Close spoke-connect multi-language SDK start — compound round, roadmap update

- Advanced CodeQL setup — rust + js/ts + actions, exclude swift smoke

- Rename CodeQL workflow to avoid default-setup name collision

- Restructure CodeQL workflow to canonical starter form

- Pin CodeQL workflow actions to immutable SHAs

- Remove custom CodeQL workflow, rely on GitHub default setup

- **docs:** Add GitHub Pages build and deploy workflow

- **docs:** Rebuild docs when lockfile changes

- **docs:** Make Pages deploy latest-wins with workflow-level concurrency

- **docs:** Ref-scope concurrency and restore main-only deploy group

- **iteration:** Close spoke-connect multi-language deepening — compound round, roadmap update

- **fixtures:** Drop redundant Knowledge Pack companion sample

- **connect:** Vendor C# bindgen fork patch retargeted to uniffi 0.32


### build

- **docs:** Scaffold VitePress site under docs/

## [0.6.1] - 2026-07-30


### Added

- **operations:** Add InternalError 500-class reject code


### Changed

- **operations:** Cover INTERNAL_ERROR in envelope round-trips

## [0.6.0] - 2026-07-30


### Added

- **schemas:** Add optional extensions to Scope (product query metadata)

- **schemas:** Add optional capability-flagged modules (ModuleMap) to KnowledgeEntry + AssemblePacket

- **operations:** Add module map merge/preserve helpers + narrative-modules capability


### Changed

- **operations:** Assert Scope.extensions preserved and ignored by matchers

- **toy-world:** Scope extensions conformance sample

- **toy-world:** Narrative Knowledge Pack companion fixture (proposed modules)

- **schemas:** Smoke-test canonical Scope::builder() + extensions round-trip


### Documentation

- **roadmap:** Plan naming triad, Scope.extensions, lore/pack handbooks

- **specs:** Add core/modules/extensions naming triad ADR

- **concepts:** Add Modules (proposed) concept + triad ADR cross-link

- **specs:** Document optional Scope.extensions (product query metadata)

- **specs:** Add lore-activation Domain Profile handbook (proposed modules.activation)

- **concepts:** Cross-link lore-activation Domain Profile handbook

- **specs:** Lore-activation handbook discoverability + robust pack forward-ref

- **specs:** Add Narrative Knowledge Pack handbook + Seed/Pool pattern

- **specs:** Fix Knowledge Pack illustrative example accuracy (schema_version, entry_type, modules placement)

- **knowledge:** Document Rust Builder as non-breaking construction path

- **knowledge:** **BREAKING:** Classify Scope.extensions as 0.x Rust-source breaking change

- **roadmap:** Schedule Phase A close — assemble recipes + capability-flagged modules

- **specs:** Add AssemblePacket module recipes (placement + activation_trace)

- **concepts:** Cross-link AssemblePacket module recipes handbook

- **specs:** Mark modules shipped (capability-flagged) across ADR/CONCEPTS/handbooks/ops

- **specs:** Align lore-activation labels to shipped capability-flagged modules


### Fixed

- **docs:** Correct Scope builder migration to try_into() + lock full chain in smoke test


### Internal

- **harness:** Ignore .mstar/references/ (process research)

## [0.5.0] - 2026-07-29


### Added

- **schemas:** Add optional revision to Relation wire

- **operations:** RelationPort OCC parity + relation error codes

- **operations:** OrchestrateRelate deep OCC integration + relate gate

- **toy-world:** Upgrade RelationPort to OCC-aware get/put with dual-language tests


### Changed

- **operations:** Assert RELATION_ALREADY_EXISTS on relate create path


### Documentation

- **specs:** Normative persisted-entity OCC parity guardrail for Relation

- **knowledge:** Relation OCC parity + generalize persisted-entity OCC note

- **knowledge:** Relation-occ-parity current facts only

- **roadmap:** Record Relation OCC parity as delivered capability

- **operations:** Document RelationPort revision assignment


### Fixed

- **operations:** Relate gate explicit mode + implicit-path errors

## [0.4.1] - 2026-07-28


### Added

- **fixtures:** Add Harbor ordered moment beat chain

- **operations:** Add timeline sequence pure helpers

- **operations:** Add Rust timeline sequence helpers


### Documentation

- **agents:** Require Conventional Commits for changelog generation

- **readme:** Document adapter ports and orchestrate* integrator path

- **specs:** Add narrative-structure Domain Profile for Beat assist


### Fixed

- **specs:** Correct Harbor fixture and ops helper claims in narrative profile

- **operations:** Align precedes linked-set and UTF-8 tie-break

- **operations:** Replace node:buffer UTF-8 compare with TextEncoder

- **operations:** Reject duplicate input ids and self-loop precedes


### Internal

- **iteration:** Close beat-assist slice — compound and roadmap

## [0.4.0] - 2026-07-27


### Added

- **toy-world:** Add HostCapabilityManifest JSON fixtures


### Changed

- **toy-world:** Cover peer manifest normalization edge cases


### Documentation

- **toy-world:** State host manifest facts affirmatively


### Fixed

- **operations:** UTF-8 byte order peer sort in orchestrate tests

- **toy-world:** Return defensive HostCapabilityManifest copies


### Internal

- **docs:** Close host-capability collaboration compound and roadmap.

## [0.3.0] - 2026-07-27


### Added

- **operations:** Add adapter port interfaces and CAPABILITY_PORT_MISSING

- **operations:** Add baseline injection orchestration with in-memory mocks

- **operations:** Add optional computable and fork orchestration

- **operations:** Add Rust adapter ports and injection orchestration


### Changed

- **operations:** Add adapter TS/Rust parity export checklist


### Documentation

- **specs:** Lock adapter interface and injection orchestration contracts

- **specs:** Add capability matrix to adapter interfaces section

- **specs:** Add injection orchestration cross-links and acceptance

- **specs:** Address QC findings for check orchestration contract

- Sync adapter interface protocol across strategy and ops READMEs

- **protocol:** Lock Adapter aliases and toy-world example path

- **roadmap:** State Adapter aliases delivery affirmatively

- **roadmap:** Drop in-repo adapters/* from scope

- Clear Up next queue and drop registry version badges


### Fixed

- **operations:** Merge check rules by rule_id; gate promote on stored state

- **operations:** Harden promote OCC with stored-based revision and adapter CAS contract

- **operations:** Require expectedBaseRevision on putKnowledgeEntry for adapter OCC

- **release:** Include spoke-fixture-toy-world in Cargo.lock bump rewrite

- **toy-world:** Echo empty computable and reject project error fixtures

- Parse README release badge URLs instead of substring match


### Internal

- **iteration:** Close adapter-interfaces slice — compound round, roadmap update

- **iteration:** Close adapter aliases and toy-world examples

## [0.2.0] - 2026-07-26


### Added

- **schema:** Close KnowledgeEntry body with BodyAttribute traits

- **spoke-operations:** Add body attribute read helpers

- **spoke-operations:** Add Rust body attribute read helpers


### Changed

- **fixtures:** Add BodyAttribute traits to Mira KB sample


### Documentation

- Lock closed L2 body and BodyAttribute trait contract

- **roadmap:** Note release bump-test harden in Now

- Align tracked results with closed L2 body wire

- **spoke-operations:** Document body attribute read helpers


### Fixed

- **release:** Make bump-version tests independent of live SemVer

- **release:** Derive refuse/drift SemVer from fixture version

## [0.1.1] - 2026-07-25


### Added

- **schema:** Add OpaqueJson ref for ComputableLogChange fields


### Changed

- **release:** Add lockstep assert and bump unit tests


### Documentation

- **protocol:** Lock CI/codegen harden contracts and consumer docs

- **readme:** Drop hardcoded lockstep version literals

- **schemas:** Refresh OpaqueJson type-test header comment

- **codegen:** Document Rust typify duplication strategy A


### Fixed

- **release:** Resolve RELEASE_TAG from inputs.tag on workflow_call

- **release:** Unblock npm OIDC by dropping setup-node registry-url

- **release:** Top-level release.yml for Trusted Publishing OIDC

- **release:** Keep verify/publish running when tag job is skipped


### Internal

- **codegen:** Regenerate OpaqueJson as unknown JSON value

- **roadmap:** Close CI/codegen harden and refresh codegen knowledge


## [0.1.0] - 2026-07-25


### Added

- **schemas:** Add v0.1 JSON Schema SSOT (17 files)

- **codegen:** Add spoke-schema TS/Rust codegen pipeline

- **operations:** Add @42ch/spoke-operations package with four helper families

- **schemas:** Add Rule and Event wire shapes with codegen

- **schemas:** Wire error-envelope on all ops responses + shared Scope

- **spoke-operations:** Add OCC, Keyblock status, uniqueness helpers

- **spoke-operations:** Add Scope, upsert/relate gates, error map; fix uniqueness R1

- **fixtures:** Add Mira at Harbor toy-world graph with AJV CI validation

- **fixtures:** Relocate toy-world AJV harness out of spoke-operations

- **schema:** Add l2-computable body fields and Moment log carrier

- **fixtures:** Add l2-computable toy-world samples

- **schemas:** Add project/compute optional op wire schemas

- **operations:** Add l2-computable validators and op fixtures

- **schemas:** Add optional l5-fork TimelineEvent and Scope fields

- **fixtures:** Add Fork-aware toy-world TimelineEvent sample

- **release:** Add git-cliff CHANGELOG and notes extractor


### Changed

- **schemas:** Rename Keyblock→KnowledgeEntry and Event→TimelineEvent

- **operations:** Rename Keyblock API to KnowledgeEntry vocabulary

- **schemas:** Rename block_type fields to entry_type

- Rename block_type to entry_type across codegen and ops

- **wire:** Rename knowledge_entry_id fields to entry_id


### Documentation

- **specs:** Require GitHub Actions CI gate for v0.1

- **roadmap:** Add durable dual-surface + nine-layer roadmap

- **specs:** Finalize v0.1 normative specs and greenfield stubs

- **adapters:** Document deferred nexus/creader mapping placeholders

- Document CI required checks for v0.1

- **readme:** Add twin consumer READMEs for integrators

- Keep READMEs human-facing; move agent boundaries to AGENTS.md

- **spec:** Fix L7 operations helpers in layers matrix

- **spec:** Clarify target wire vs committed schemas for QC

- **schemas:** Update checklist and counts to 19 files

- Ban iteration ids from git-shared records

- Extend iteration-id ban to commits and PR prose

- **protocol:** Clarify fixture harness ownership boundary

- Mark operations deepen and fixtures delivered

- **knowledge:** Fix fixture harness file names in pattern

- **roadmap:** Reshape into Now / Up next / Done project plan

- **protocol:** Lock KnowledgeEntry and TimelineEvent terminology

- **fixtures:** Align toy-world and surface docs to KnowledgeEntry vocabulary

- **roadmap:** Mark terminology delivered on main

- **knowledge:** Note entry_type rename in flight for Block* fields

- **spec:** Sync canvas coverage to entry_type vocab and ops inventory

- Keep long-term knowledge and specs fact-only

- Protocol-only CONCEPTS; facts-only rule in AGENTS

- **spec:** Lock optional Computable capability contracts

- **protocol:** Lock optional l5-fork TimelineEvent wire semantics

- Add README status badges and doc nav links

- **protocol:** Lock unified CI and tag version release policy

- **protocol:** Align release policy Related and roadmap wording

- **protocol:** Align release CI SSOT with four verify jobs

- Document SPOKE version release flow


### Fixed

- **schemas:** Enforce assemble-response packet XOR error

- **operations:** Harden malformed-input guards per QC review

- **spoke-operations:** Validate block_type and deep-copy extension merges

- **codegen:** Export titled oneOf schemas by title only in Rust

- **security:** Pin CI Actions and bump esbuild past CVE range

- **spoke-operations:** Address QC warnings on upsert, relate, error map

- **spoke-operations:** Unblock ci:typescript for fixture test harness

- **ci:** Run toy-world fixture tests before operations build

- **operations:** Guard null/undefined changes in validateComputableLogEntry

- **operations:** Align logged_at validation with date-time schema intent

- **operations:** Honor Scope.fork_id in timelineEventMatchesScope

- **ci:** Run lockstep assert via node in release workflow

- **release:** Defer annotated tag until after version bump commit

- **release:** Anchor README badge regex for CodeQL

- **ci:** Verify release tag matches package version

- **release:** Avoid hostname regex for README badge parse


### Internal

- **v0.1:** Lock iteration-start — SPOKE protocol bootstrap harness

- **harness:** Move specs under .mstar and ignore process paths

- Add CLAUDE.md pointer to AGENTS.md

- Add verify-codegen GitHub Actions workflow

- Add typescript and rust package health jobs

- **iteration:** Close v0.1 — compound codegen knowledge + roadmap

- Run spoke-operations typecheck and tests in typescript job

- Rename spoke-schema packages to spoke-schemas

- **codegen:** Assert schema file count in verify-codegen

- **iteration:** Close fixture-boundary slice — compound codegen verify note

- **codegen:** Regenerate TS/Rust types for KnowledgeEntry rename

- **iteration:** Close terminology slice — compound vocabulary note

- **iteration:** Close entry-type rename slice — compound + roadmap

- **codegen:** Regenerate after entry_type vocab description sync

- **iteration:** Close Computable slice — compound and roadmap

- Assert lockstep package versions

- Add tag-triggered GitHub Release workflow

- Use CHANGELOG section for GitHub Release body

## [0.1.0-alpha.2] - 2026-07-25

### Fixed

- **npm:** Publish dist-only package tarballs with package READMEs (omit `src/`)

## [0.1.0-alpha.1] - 2026-07-25


### Added

- **rust-ops:** Add spoke-operations crate with first-slice helpers

- **spoke-operations:** Add OCC and KnowledgeEntry lifecycle helpers

- **spoke-operations:** Add scope, upsert, and relate helpers

- **spoke-operations:** Add error envelope and computable validators


### Documentation

- **release:** Mirror npm/crates partial-failure note in README_CN

- **protocol:** Lock Rust spoke-operations crate contract

- **spec:** Record Rust deepen and computable parity

- **spec:** Realign Hard In/Out Out column

- **spec:** Clarify Rust export parity vs TS index

- Add Rust spoke-operations consumer pin surfaces

- **knowledge:** Crystallize Rust spoke-operations parity patterns


### Fixed

- **release:** Rc tag assert and registry publish hygiene

- **release:** Align Cargo repository URL with GitHub remote

- **release:** Reject lightweight tags in release verify

- **rust-ops:** Wire body snippet path and revision validation

- **rust-ops:** Assemble body validation and reject-code doc count

- **release:** Pin spoke-schemas version for ops publish

- **spoke-operations:** Address PR review parity and OCC safety

- **deps:** Bump brace-expansion to >=5.0.8 for GHSA-mh99-v99m-4gvg


### Internal

- Publish npm packages and spoke-schemas crate on release tags

- Run spoke-operations tests in rust verify jobs

- Publish spoke-operations crate after spoke-schemas


### build

- Mark consumer packages publishable to npm and crates.io


### release

- Assert spoke-operations in lockstep surfaces

