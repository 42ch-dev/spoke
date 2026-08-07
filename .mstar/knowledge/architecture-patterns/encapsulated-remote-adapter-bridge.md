---
module: spoke-connect
date: 2026-08-05
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["bridging an orchestration/port library to a session-protocol library", "exposing a remote capability as a drop-in local adapter", "designing a pluggable transport seam with in-library interface and consumer-side implementation", "hardening an encapsulation boundary against emitted type declarations", "gating an optional cross-library dependency behind a subpath export or cargo feature", "preparing a single-peer proxy for later multi-peer routing or FFI exposure"]
related_components: ["spoke-operations", "spoke-connect", "spoke-schemas"]
tags: [remote-adapter, transport-seam, encapsulation, proxy-adapter, port-invoke-mapping, feature-gating, concurrency-demux, ts-rust-parity]
---

# Encapsulated remote-adapter bridge across two library surfaces

## Context

Two libraries often exist side by side without a connection: a **pure orchestration library** that drives entity lifecycles through injected port interfaces (read/write/query methods), and a **session-protocol library** that provides authenticated, ordered request/response channels between peers. Integrators want a connected peer to feel like a **local adapter**: the same `orchestrate*(adapter, request)` calls, the same `SpokeResult` outcomes, and none of the protocol mechanics visible at the callsite.

This pattern is the reusable shape of that bridge, as built in spoke-connect: a `RemoteAdapter` that implements the async `BaselinePorts` interface of spoke-operations by proxying each port call over a connect session, with **all** verification (hello sign/verify, allowlist, nonce, sequence, request correlation, dispatch gate, capability-token) encapsulated inside the adapter. The single-peer slice stays multi-peer-ready: session state is kept **per peer**, not in a process-global singleton, so a later registry can compose N adapters without changing the consumer-facing ports contract.

SPOKE-specific decision record (placement, catalogue, error matrix, concurrency rules): `.mstar/specs/spoke-remote-adapter.md`.

## Guidance

### 1. Proxy the port interface; map each method to a protocol invoke

The adapter implements the consumer-facing port interface (here the six `BaselinePorts` families). Each method maps to one protocol invoke **over existing wire shapes** — an open `op` string plus an opaque JSON `payload` — so **no new schemas** are needed:

- `op` under a reserved product prefix (`port.knowledge.get`, `port.relation.put`, `port.scope.list_knowledge_entries`, …) — do **not** reuse the core ops vocabulary (`upsert`, `check`, …), because orchestration runs **locally** on the caller and each port I/O is one remote round-trip.
- request `payload` carries method arguments (snake_case field names); success `payload` carries the success value; failures ride the existing error envelope.
- required capability per op family (`spoke-baseline` for the baseline catalogue), authorized on the host through the existing product `op_capability_requirements` map.

A port method that reads session state rather than the peer (e.g. `getHostCapabilityManifest` returning the remote hello `host`, cached at establish) needs **no round-trip at all** — prefer cache-only defaults.

### 2. Message-oriented Transport seam: interface in-library, implementation consumer-side

Define the transport as a **message-oriented** interface — one protocol envelope's bytes per `send` / `recv` call — rather than a raw byte stream. The application unit is the envelope; re-implementing framing inside the adapter duplicates a concern the protocol layer already owns.

```ts
export type EnvelopeBytes = Uint8Array; // bytes of exactly one UTF-8 JSON envelope

export interface Transport {
  send(envelope: EnvelopeBytes): Promise<void>; // one envelope per call
  recv(): Promise<EnvelopeBytes>;               // rejects on transport close
  close?(): void | Promise<void>;               // optional, idempotent
}
```

- The interface ships **in the library**; concrete transports (WebSocket, …) are **consumer-side, language-native**.
- An in-memory **loopback pair** (`loopbackTransportPair()` / `loopback_transport_pair()`) ships in-repo for tests; closing either end fails the peer's pending `recv` like a real connection drop.
- Byte-stream carriers (TCP, pipes) document length-prefix (or equivalent) delimiting as **transport-adapter-owned**, not the adapter's job.
- Ordering contract: an ordered, reliable bidirectional message stream (WebSocket one-message-per-envelope conforms).

### 3. Encapsulation is a shipped-surface property, not an IDE convention

The adapter owns the full session lifecycle internally: hello sign/verify, allowlist, nonce single-use, sequence allocate/advance, `request_id` generation, correlation map, dispatch awareness, capability-token attach/validate, the single receive loop, and invoke timeout timers. Consumers **must not** need any verification helper to perform an operation.

The public surface is exactly: the async port interface + a dial/connect constructor + read-only session info (`sessionId`, `remotePeerId`, `remoteManifest`, `state`) + `close`.

**TypeScript gotcha — `#private` vs `private`:** TypeScript `private` still **emits the member into `dist/*.d.ts`** (as a `private` declaration) and `public @internal` ships too unless `stripInternal` is on — a consumer can cast `as any` and call the member at runtime. The 5 lifecycle methods (`beginHandshake`, `sendEnvelope`, `recvEnvelope`, `establish`, `closeSession`) were shipped this way and created a real bypass: `establish()` could forge `Established` state **without hello/allowlist verification**. ECMAScript `#private` members compile to a single `#private;` marker in the `.d.ts` and are **runtime-unreachable even via `as any`** — use them for every internal member a consumer must never touch.

```ts
class RemoteAdapter implements BaselinePorts {
  #core: SessionCore;            // #private — invisible in emitted .d.ts
  async getKnowledgeEntry(id: string) { … }   // public — the only surface
}
```

**Rust:** module-private (`pub(crate)` / private fields) is sufficient; verify with a compile-time surface probe or shipped `.d.ts` grep that no verification member name appears.

### 4. Reuse the protocol core; do not widen its parity surface

The bridge **imports and reuses** the existing session core (allowlist, hello crypto, nonce, sequence, correlation, dispatch, capability-token). It must not expand the shared cross-language parity table — the parity surface stays exactly the session core, transports remain intentionally asymmetric and outside parity. Keep the bridge module **opt-in** so the default library surface and dependency graph are unchanged:

- TS: a **subpath export** (`./remote`) with the root bundle excluding the bridge symbols; assert the split with a dist-shape smoke test.
- Rust: an **optional cargo feature** (`remote-adapter = ["dep:spoke-operations"]`) with `default = []`; verify with `cargo tree -p … -e normal` (0 occurrences default, 1 with feature).
- **CI must compile and run the feature.** A feature that is never exercised in CI silently rots (this was a real finding: the Rust remote module merged with zero CI signal until a dedicated feature-step was added).

### 5. Concurrent invokes: atomic sequence + per-request correlation, one receive loop

Multiple port calls may be in flight on one established session:

- outbound `sequence` allocated **atomically** at send time (per-peer counter, starts 0, no wrap — exhaustion closes the session);
- pending map keyed by `request_id` (never reused while in flight); responses must echo `session_id`, `sequence`, `request_id`;
- completions may arrive **out of order** — demux delivers to the matching waiter, no FIFO on caller awaits;
- the adapter owns a **single receive loop** over `Transport.recv`; port callers never call `recv` directly;
- transport close mid-flight fails **all** pending waiters; a per-waiter invoke timeout fails only that waiter (session stays usable); protocol v1 defines no retry — callers re-invoke with a fresh `request_id`.

### 6. Error mapping: settle everything the caller can observe

Lifecycle/application outcomes stay on the port result type; only session-establishment failures are exceptional:

| Failure class | Surface |
|---------------|---------|
| Remote application rejects (NOT_FOUND, REVISION_CONFLICT, …) | port result reject, codes preserved via the existing envelope mapping |
| Dispatch deny (`op_unsupported` / `capability_missing`) | port result reject `CAPABILITY_PORT_MISSING` with `details.wire_code`; unknown wire codes → `INVALID_INPUT` + `wire_code` |
| Transport I/O, session closed, invoke timeout, correlation mismatch, sequence exhaustion | port result reject `INTERNAL_ERROR` with `details.kind` = `transport` / `session_closed` / `timeout` / `correlation_mismatch` / `sequence_exhausted` |
| Dial / hello / allowlist / nonce failure | **constructor throws** / `Err` — no half-open adapter instance |

Port methods always settle to a result (no bare rejects for mapped classes); typed transport errors remain on the lower-level session API. Drop-in uniformity across local and remote adapters is what the orchestrator sees.

### 7. Prove the drop-in property with loopback parity

A consumer-style test drives the adapter **exclusively through the orchestrators** (`orchestrateUpsert(remoteAdapter, req)`, `orchestrateCheck(...)`) against a loopback host serving a local async adapter, and asserts:

- identical `SpokeResult` shape as the same call against the local adapter (success + reject paths);
- the remote write actually landed in the host-side store;
- verification actually ran (`hellosVerified === 1`, session state `Established`, `remotePeerId`/`remoteManifest` are the peer's);
- the consumer test never calls hello/verify/nonce/sequence directly — encapsulation asserted at compile time (assign the adapter to the port type) and at runtime (verification helper names are `undefined` / absent).

## Why This Matters

Without a bridge, every product re-implements remote capability behind its own facade and drifts from the local programming model. Without **shipped-surface encapsulation**, the "verification hidden" promise is cosmetic: a leaked d.ts member lets consumers forge session state and bypass the protocol's security checks. Without the message-oriented seam, the library re-implements framing for every transport and multiplies adapter code. Without opt-in gating, an optional cross-library dependency widens the default published surface and breaks consumers that never asked for it.

## When to Apply

- Bridging a pure orchestration/port library to a session-protocol library so a peer acts as a drop-in local adapter.
- Exposing any remote capability behind a local interface while keeping protocol mechanics hidden.
- Designing a transport-pluggable surface: interface in-library, concrete implementations consumer-side, loopback in-repo.
- Any `#private`-grade encapsulation boundary in TypeScript where the emitted `.d.ts` is the real security surface.
- Preparing a single-peer proxy for follow-on multi-peer routing or native-binding FFI exposure (keep session state per peer, never a process-global singleton).

## Examples

### Before: consumer couples to protocol callsites

```ts
// consumer must manage verification themselves — leaks protocol state
const nonce = generateNonce();
const hello = signHello(identity, nonce);
await sendEnvelope(transport, hello);
await verifyHello(peer, await recvEnvelope(transport));
const seq = allocateOutboundSequence();
await sendEnvelope(transport, makeInvoke({ seq, op: "upsert", … }));
```

### After: drop-in adapter, verification internal

```ts
import { connectRemoteAdapter } from "@42ch/spoke-connect/remote";
import { orchestrateUpsert } from "@42ch/spoke-operations";

const adapter = await connectRemoteAdapter({ transport, localIdentity, allowlist });
const result = await orchestrateUpsert(adapter, request); // same as a local adapter
```

## See also

- `.mstar/specs/spoke-remote-adapter.md` — SPOKE decision record (D1–D9): placement above session-core, Transport seam, port-method catalogue, concurrency, error matrix, TS↔Rust parity statement
- `architecture-patterns/spoke-connect-wire-and-auth.md` — the session core and wire family the bridge reuses
- `architecture-patterns/adapter-injection-orchestration.md` — the local port-injection model the bridge implements remotely
- `architecture-patterns/connect-session-core-ffi-boundary.md` — the pure session-core extraction that makes session state reusable above transport
- `architecture-patterns/connect-ts-client-sdk.md` — language-native client surface and session-core parity discipline
