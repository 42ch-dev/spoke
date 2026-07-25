# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Release notes for GitHub Releases are extracted from the matching version section here.
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

- **v0-iter002:** Lock iteration-start — operations spec + three-column framing

- **readme:** Add twin consumer READMEs for integrators

- Fix CN wire terminology and align STRATEGY roadmap

- Align version labels and v0-iter002 delivered status

- Keep READMEs human-facing; move agent boundaries to AGENTS.md

- **v0-iter003:** Lock L0–L8 layers, TimelineScale, Rule/Event intent

- **spec:** Fix L7 operations helpers in layers matrix

- **spec:** Cross-link normative layers in umbrella and roadmap

- **concepts:** Sync Rule/Event wire notes for v0-iter003

- **spec:** Clarify target wire vs committed schemas for QC

- **spec:** Ship Rule vs Finding boundary for v0-iter003

- **schemas:** Update checklist and counts to 19 files

- **specs:** Finalize ops-harden error envelope and Scope contracts

- **iteration:** Lock v0-iter004 — ops actions + toy-world fixtures

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

- **iteration:** Close v0-iter002 — compound consumer-readme twin

- Rename spoke-schema packages to spoke-schemas

- **iteration:** Close v0-iter003 — compound TimelineScale and ops oneOf

- **iteration:** Close v0-iter004 — compound ops actions and fixture harness

- **codegen:** Assert schema file count in verify-codegen

- **iteration:** Close fixture-boundary slice — compound codegen verify note

- **codegen:** Regenerate TS/Rust types for KnowledgeEntry rename

- **iteration:** Close terminology slice — compound vocabulary note

- **iteration:** Close entry-type rename slice — compound + roadmap

- **codegen:** Regenerate after entry_type vocab description sync

- **iteration:** Close Computable slice — compound and roadmap

- Assert lockstep package versions

- Add tag-triggered GitHub Release workflow


### merge

- **plan:** V0.1 spoke-core-bootstrap into iteration/v0.1

- **plan:** V0.1 spoke-ci into iteration/v0.1

- **plan:** V0-iter002 spoke-operations into iteration/v0-iter002

- **plan:** V0-iter002 dev-docs into iteration/v0-iter002

- **plan:** Ops-lifecycle — OCC, Keyblock status, uniqueness helpers

- **plan:** Ops-wire-actions — Scope, upsert/relate, error map

- **plan:** Fixtures-conformance — Mira at Harbor toy-world + AJV CI

- **plan:** Fixture-boundary — toy-world harness out of spoke-operations

- **plan:** Protocol-status-sync — docs delivered + schema-count guard

- **plan:** Wire terminology — KnowledgeEntry + TimelineEvent schemas

- **plan:** Consumers docs — KnowledgeEntry ops and fixtures

