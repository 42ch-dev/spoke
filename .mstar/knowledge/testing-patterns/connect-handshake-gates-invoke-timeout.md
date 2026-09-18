---
module: spoke-connect
date: 2026-09-18
problem_type: testing_pattern
category: testing-patterns
severity: medium
plan_id: connect-handshake-timeout-budget
tags:
  - connect
  - handshake
  - test-timeout
  - flake
  - loopback
  - ts-rust-parity
---

# Connect handshake budgets: a test's invoke timeout also caps the dial

## Context

`connect_remote_adapter` takes a single `invoke_timeout_ms` and uses it for two different jobs. It bounds ordinary invokes, and it also bounds every handshake step: the client hello send, the `server hello` wait, and the session-snapshot wait (`crates/spoke-connect/src/remote/remote_adapter.rs`). The TypeScript client (`packages/spoke-connect-ts`) shares that design — the same option gates its dial.

The loopback test harness separates the two costs. `DialOptions.host_delay` is applied inside `handle_invoke` only (`crates/spoke-connect/tests/common/loopback_oracle_impl.rs`), so a delay set to force an invoke timeout does not slow the handshake.

## Guidance

Tests that need a small `invoke_timeout_ms` to force a timeout must keep that budget well above handshake latency — handshake work (Ed25519 signing/verification, transport round-trips, task scheduling) is unbounded on a loaded CI runner, while an invoke timeout only needs the budget to sit below the delay.

- Budget: 100 ms invoke timeout.
- Host delay: 200 ms — strictly greater than the budget, so the timeout assertion is deterministic rather than a race.
- Apply the same pair in both language twins. The TS and Rust tests cover the same behavior and are edited together.

A budget below handshake latency fails the dial itself (`Timeout("connect: server hello timed out after <N>ms")`) and surfaces as a dial error, not as the timeout the test intended to assert.

## Why This Matters

A dial-time failure reads as a product defect in the CI log even though only the test budget is wrong, and the failing test name points at the timeout assertion rather than at the handshake. A budget that sits at handshake latency also converts a deterministic assertion into a load-dependent flake: it passes locally and intermittently fails on CI.

## When to Apply

Any test that constructs a remote adapter with an explicit `invoke_timeout_ms` — including tests that exist to exercise invoke timeouts, request correlation, or session-closed paths. Also when adding a new timeout-shaped test to either language: pick the budget against handshake latency first, then set the delay above it.

Not applicable to the deliberate dial-timeout tests (`ffi_error_parity_dial_timeout_on_never_responding_server`), where a server that never answers makes a short budget the point of the test.

## Examples

- `crates/spoke-connect/src/ffi.rs` — `ffi_error_parity_invoke_timeout`: 100 ms budget, 200 ms host delay, asserts the `timeout` reject kind plus FFI/async parity.
- `crates/spoke-connect/tests/remote_loopback.rs` — `maps_invoke_timeout_to_internal_error_kind_timeout_without_closing_session`: same pair, then clears the delay and asserts the session stays usable.
- `packages/spoke-connect-ts/tests/remote/remote-adapter.test.ts` — the TS twin of that case, same pair and the same naming of the underlying hazard.
