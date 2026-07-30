# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release notes for GitHub Releases are extracted from the matching version section here.
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

