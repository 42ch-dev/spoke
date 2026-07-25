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

