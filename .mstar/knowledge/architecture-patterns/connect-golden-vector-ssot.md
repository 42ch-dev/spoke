---
module: spoke-connect
date: 2026-08-04
problem_type: testing-pattern
category: architecture-patterns
severity: medium
tags: [connect, golden-vector, cross-language, parity, ssot, test-fixture, uniffi]
applies_when: consolidating hardcoded test constants shared across multiple language bindings
---

# Cross-language golden vector SSOT pattern

## Context

When a protocol crate has bindings in multiple languages (Rust core + TypeScript + C# + Go + Python + Swift + Kotlin), the golden test vector — the pinned input bytes and independently captured output bytes used to assert cross-language parity — is typically duplicated as hardcoded constants in each language's test module. This duplication creates drift risk: a change in one language's golden constant silently breaks parity.

## Guidance

Use a **single shared JSON artifact** as the golden vector SSOT, with registered byte-identical copies for languages that cannot easily read the monorepo-relative path at test time.

### SSOT design

| Element | Rule |
|---------|------|
| **Write authority** | One path, owned by the language that is the historical capture authority (for `spoke-connect` hello: Rust crate, because the libp2p capture was first done there) |
| **Artifact path** | `crates/spoke-connect/tests/fixtures/golden-hello.json` — crate-local `tests/fixtures/` |
| **Field schema** | BOTH inputs (`seed_hex`, `nonce`, `manifest`) AND pinned outputs (`pubkey_hex`, `peer_id`, `jcs_hex`, `signature_b64u`). Outputs are **transcribed from existing committed constants**, never regenerated via code-under-test |
| **Manifest completeness** | `authority` omitted (never `null`); `namespaces: []` preserved verbatim |

### Per-language load strategy

| Surface | Load approach |
|---------|---------------|
| Rust test modules | `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden-hello.json"))` + `serde_json` in `#[cfg(test)]` only |
| TypeScript | Registered copy at `packages/spoke-connect-ts/tests/fixtures/`; thin loader in a non-exported module (not in npm `exports`/`files`) |
| Go / Python smokes | Read SSOT via monorepo-relative path from the test file |
| C# / Kotlin / Swift smokes | Registered local `Smoke/fixtures/` copy (Content/resource / file read next to smoke) — cwd and sandbox often make deep monorepo relative paths fragile |

### Sync gate

A zero-dep script (`tooling/connect/golden-vector-sync.mjs`) holds a **manifest of all registered copy paths** and verifies byte-equality with the SSOT. Default mode: verify, exit non-zero on drift. Optional `--write` refreshes copies from SSOT.

**Critical:** the sync gate must cover ALL copies (TS + every binding), not only Rust↔TS. A TS-only sync gate leaves binding copies free to drift.

## Why this matters

- **Drift prevention:** with N languages hardcoding the same constant, any change requires N coordinated edits. With a SSOT + sync gate, one edit + `--write` propagates.
- **Parity integrity:** pinned outputs must be independently captured (transcribed from historical constants or a one-time generation run), never written back by a generator that runs the code under test — otherwise the test asserts code-vs-itself.
- **Production boundary:** the artifact is test-only. Rust `#[cfg(test)]` gating and TS non-exported modules ensure production library code never depends on the test fixture.

## When to apply

- Any cross-language protocol crate with golden-parity smokes
- When the same test vector is hardcoded in ≥2 languages
- When adding a new binding language to an existing multi-language crate

## Examples

- `spoke-connect` hello vector: `crates/spoke-connect/tests/fixtures/golden-hello.json` + 4 registered copies (TS, C#, Kotlin, Swift) + Go/Python reading SSOT directly
- Capability-token golden vectors use a **bidirectional** model: the TS-minted vector (`capability-token-ts-golden.json`) has its SSOT under `crates/spoke-connect/tests/fixtures/` with a gate-managed registered TypeScript copy; the Rust-minted vector (`capability-token-golden.json`) stays TS-local and unregistered, with its own drift test (`capability-token-golden.test.ts`)
- Sync gate: `tooling/connect/golden-vector-sync.mjs` (verify mode; CI-wired as a `golden-vector-sync` job in `ci.yml`)
