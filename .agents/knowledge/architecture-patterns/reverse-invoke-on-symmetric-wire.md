---
module: spoke-connect remote layer
date: 2026-08-16
problem_type: architecture_pattern
category: architecture-patterns
severity: high
tags: [reverse-invoke, wire-compat, request-classification, send-tail-ordering, connect, tools]
related_components: [packages/spoke-connect-ts/src/remote, crates/spoke-connect/src/remote, schemas/connect]
applies_when: ["extending a symmetric request/response wire with server-to-client (reverse) invokes", "adding capability-gated op families over an open op vocabulary", "auditing a receive loop before enabling reverse invokes"]
---

# Reverse invokes over a symmetric request/response wire

## Context

SPOKE connect pairs a dialer (`RemoteAdapter`) with a responder over one message-oriented session. The wire is symmetric by construction: per-direction outbound sequence counters, identical `ConnectInvokeRequest`/`ConnectInvokeResponse` shapes in both directions, and protocol_version 2 envelope-auth signatures that are signer-generic (each side signs with its hello-identity seed and verifies with the peer's hello public key). Serving tools from the dialer back to the orchestrating host (the `tools.<ns>.<tool_id>` family) exercises that symmetry in reverse — responder issues invokes, dialer serves them — with zero envelope-schema change.

Two invariants make this safe, and both were learned the hard way during review.

## Guidance

### 1. Classify inbound envelopes request-shape-first

A reverse `ConnectInvokeRequest` carries `payload` plus the three correlation fields, so it satisfies naive response discriminators. Before enabling reverse invokes, harden **both** discriminators so an `op`-bearing document is never treated as a response, and classify in this order:

1. Document carries `op` ⇒ it is a request → reverse-serving pipeline.
2. Otherwise treat as a response → pending-waiter demux by `request_id`.

Without this, reverse requests land in the response demux and are silently dropped as unknown responses — the feature fails silently (not a security hole: a pending-id collision is still rejected by correlation and envelope-auth checks).

### 2. Serve in the canonical order, auth before advance

The reverse-serving pipeline runs: stray-session check → sequence **peek** (non-mutating) → envelope-auth verify → **advance** the inbound counter → dispatch gate → handler or deny → signed response with correlation echo. Steps peek→verify→advance MUST be serialized per session; the inbound counter advances only after auth verify passes. Deny answers reuse the existing error branch (`op_unsupported` for gate-fail and unhandled ops, `auth_failed`, `invalid_sequence`) — no new wire codes.

### 3. Allocate the outbound sequence under the send lock

On multi-threaded runtimes, allocating the outbound sequence **before** acquiring the send-tail lock lets two concurrent invokes reach the wire out of allocation order; the peer's inbound gate then rejects the early request (`invalid_sequence`, no advance) and every later invoke fails while both sides report Established — a silently poisoned session. Acquire the send-tail lock **first**, then allocate, sign, register/complete the waiter, and send. Waiter registration may happen before the tail (call-time clock) as long as its correlation entry is completed under the tail before the send; deferred-send poison-close semantics (waiter settles while queued ⇒ close the session) survive unchanged.

### 4. Make op families self-describing instead of registry-backed

For the `tools.*` family the required capability **is** the op string (`tools.<ns>.<tool_id>`), negotiated when both hellos list it — no umbrella flag, no central registry, and the namespace must be a member of the declaring manifest's `namespaces[]` (ownership, anti-spoof). The discovery source is the authenticated manifest (`tools[]` descriptors carrying the input/output ABI as JSON Schema subschemas); the handler registry is a pure local concern and never mutates the manifest. A manifest-declared tool with no registered handler is denied fail-closed at invoke time.

## Why This Matters

These four points are the difference between a reverse-invoke feature that silently no-ops (discriminator collision), one that quietly deadlocks a session (out-of-order wire writes), and one that is auditable (deny matrix on existing codes, auth-before-advance). They are wire-level invariants: they apply to any future family that makes a dialer serve ops back to its host.

## When to Apply

- Extending connect (or any symmetric request/response protocol here) with peer-initiated invokes.
- Reviewing a receive-loop change: check classification order, peek/verify/advance serialization, and lock-before-allocate before anything else.
- Designing a new op family: prefer self-describing capability ids over registries or umbrella flags; keep discovery on the authenticated manifest.

## Examples

- Shipped surfaces: `registerToolHandler` / `invokeTool` on `RemoteAdapter` and `ConnectResponder`; `MultiPeerRouter.invokeTool` delegates through the selected adapter (it never crafts envelopes). Spec: `spoke-remote-adapter.md` D13/D14; session rules: `spoke-connect.md` §Op dispatch gate.
- The ordering regression is pinned by a multi-threaded loopback test: a `YieldingSendTransport` (yield per send) plus a wire-witness recorder asserting 32 concurrent invokes hit the wire in exactly allocation order 0..31. On the pre-fix lock order this test fails 5/5 with the exact `invalid_sequence` reject; with lock-before-allocate it passes 5/5 deterministically. That test shape (forcing scheduler interleaving + wire-order witness) is the reusable template for any lock-ordering invariant on this wire.
