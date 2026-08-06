---
title: Connect architecture
---

# Connect architecture

**Connect** is the opt-in interaction envelope family for cross-process SPOKE hosts (the `spoke-connect` capability flag): signed manifest exchange, session context, remote op invocation, and extensible authentication. It is additive — baseline compliance and baseline schemas stay unchanged, and hosts that do not declare `spoke-connect` are unaffected.

The family reads as one integrator journey: install → a language-native client session → a RemoteAdapter over a consumer `Transport` → multi-peer routing → native bindings → a loopback smoke. This page explains the concepts behind that journey; the [tutorial](/tutorials/first-connect-session) walks it, the [how-to guides](/how-to/connect-remote-adapter) are the recipes, and the [wire reference](/reference/connect) is the dictionary.

## The three embedding surfaces

| Surface | What ships | When to choose it |
|---------|------------|-------------------|
| **Language-native client** | The wire contract and session-core rules implemented in the host language — the TypeScript `@42ch/spoke-connect` client, paired with the platform WebSocket | No Rust runtime in the host; browser or Node consumers |
| **Native bindings** | The shared session core exported into host languages via FFI (C# NuGet, Kotlin Maven, Swift SPM, Go modules, Python PyPI) | Host languages with an FFI story that want the core implemented once, with transport in the host |
| **Rust reference** | The published `spoke-connect` crate: the session-core reference, the binding source, and a rust-libp2p transport stack | Rust consumers, and the reference for byte-level parity everywhere |

All three surfaces share the same session-core rules — `peer_id` derivation, hello crypto, allowlist, nonce, sequence, correlation, dispatch gate — locked by golden vectors.

## Session lifecycle

A connect session passes through four states: `Disconnected` → `Handshaking` → `Established` → `Closed`.

The handshake is a signed hello exchange. Each side signs a canonicalized object — `{protocol_version, peer_id, nonce, host}` (initiator) or the same plus `peer_nonce` (responder) — with its Ed25519 peer identity. The responder's `peer_nonce` echoes the initiator's nonce, binding the dial: a captured responder hello cannot be replayed into a fresh dial. Admission is fail-closed on an allowlist, and each accepted `(peer_id, nonce)` pair is single-use.

Once both hellos are accepted, the session snapshot (`ConnectSession`) records the session id, the bound peer ids, the negotiated capabilities, and the starting sequence. From `Established` on, each peer maintains its own per-direction `sequence` counter starting at 0, and every invoke carries a caller-generated `request_id`; a response must echo `session_id`, `sequence`, and `request_id` or the correlation check fails. Sequence exhaustion closes the session rather than wrapping — a fresh session starts clean counters instead of reusing sequence space.

## Envelope authentication

Envelope authenticity is a **protocol-level property above the transport**: every post-hello trust-affecting envelope — the session snapshot, invoke requests, and invoke responses — carries an Ed25519 signature over the RFC 8785 JCS-canonicalized signed object, verified inside the session core. The construction is the same as the hello, extended to three algorithm ids (`spoke-connect-session-jcs-v1`, `spoke-connect-invoke-request-jcs-v1`, `spoke-connect-invoke-response-jcs-v1`). Protocol version **2** makes these signatures required on the post-hello wire; the hello exchange itself stays at version 1.

This is why the adapter works over any ordered, reliable carrier — TCP, WebSocket, yamux, or Noise — without trusting the transport for authenticity: the envelope itself is authenticated, and a transport that supplies an authenticated peer identity does not relax the rules. The signed object covers the trust-affecting fields; `extensions` stays outside the signature, so it never influences authorization. Mixed-version peers fail closed: a v2 peer refuses to establish with a v1 peer rather than accept unauthenticated envelopes, and there is no compatibility shim.

## Capability routing

A **RemoteAdapter** implements the async `BaselinePorts` adapter contract by proxying each port call as a reserved `port.*` op over an established session — the remote host's port surface appears local. A **multi-peer router** composes N registered adapters behind the same `BaselinePorts` surface, so `orchestrateUpsert(router, req)` reaches a capable peer without naming one.

The router's selection is a pure function of the registered peers and the request: hard gates on the peer's declared capabilities (the op's required capability), namespaces, and `authority.scope_key`; a soft preference for the op's preferred role; and a deterministic lowest-`peer_id` tie-break. When no registered peer passes the hard gates, the call rejects with `no_capable_peer` — the consumer registers a satisfying peer and re-invokes with a fresh `request_id`. Retry is consumer-owned: a call may have been applied before a transport failure, so the consumer decides whether re-running the operation is safe.

Capabilities have two sources: the session's `negotiated_capabilities` (the agreed subset of both hosts' lists) and capability tokens (short-lived, capability-scoped grants from a trusted issuer). A token grant authorizes membership for the ops it covers, but it does not replace the negotiated set — both must allow an op when the token gate is active.

## Where transport lives

Transport is a consumer-implemented seam. The connect packages define a message-oriented `Transport` — one connect envelope per `send` / `recv` call, blocking `recv` until an envelope arrives or the connection closes, idempotent `close` — and ship an in-memory loopback pair for tests. WebSocket and other carriers are consumer-side implementations of the same three methods; byte-stream carriers apply length-prefix (or equivalent) delimiting. The loopback pair is test-only — the smoke that dials through it is a verification flow, not a production carrier.

## Related

- [Open your first connect session](/tutorials/first-connect-session) — the learning path, step by step.
- [RemoteAdapter over a Transport](/how-to/connect-remote-adapter) — dial a remote peer over a consumer `Transport`.
- [Route across multiple peers](/how-to/multi-peer-routing) — one router over N registered adapters.
- [Connect from native bindings](/how-to/connect-native-bindings) — the shared session core over FFI.
- [Connect wire reference](/reference/connect) — envelope field tables and the v2 envelope-authentication rules.
