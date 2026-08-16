# RemoteAdapter + message-oriented Transport

> **Owns:** Library-level decisions for the single-peer remote `BaselinePorts` adapter over connect, the multi-peer capability router over registered adapters, and the message-oriented Transport seam in spoke-connect.  
> **Status:** Implemented — normative surface in `@42ch/spoke-connect` (TS `./remote` subpath) and `spoke-connect` (Rust `remote-adapter` feature).  
> **Does not own:** Wire schema changes or in-repo WebSocket clients.

## Relationship

| Document | Role |
|----------|------|
| [`spoke-connect.md`](spoke-connect.md) | Normative connect wire, session-core, framing, dispatch gate |
| [`spoke-operations.md`](spoke-operations.md) | Port families, orchestration, `SpokeResult` |
| [`spoke-connect-ts-route.md`](spoke-connect-ts-route.md) | Language-native client route; session-core parity boundary |
| This document | RemoteAdapter placement, Transport seam, port-method invoke mapping, error and concurrency rules |

## Decisions

### D1 — RemoteAdapter sits above session-core

`RemoteAdapter` lives in spoke-connect as an opt-in surface that **reuses** session-core (allowlist, hello crypto, nonce, sequence, correlation, dispatch gate, capability-token) and **does not** expand the TS↔Rust session-core parity table. Transports remain outside that parity surface.

### D2 — Drop-in async BaselinePorts

`RemoteAdapter` implements the async `BaselinePorts` (six families) in both TypeScript and Rust, so consumers call `orchestrateUpsert(remoteAdapter, request)` (and other baseline orchestrators) without transport or verification callsites. Public surface = async `BaselinePorts` + dial/connect constructor + read-only session metadata + `close`.

### D3 — Message-oriented Transport seam

spoke-connect defines a **message-oriented** `Transport` (`send` / `recv` of one connect envelope’s bytes per call). This matches §Transport framing in [`spoke-connect.md`](spoke-connect.md) (one JSON document = one envelope). WebSocket and other product transports are **external**; spoke-connect ships a loopback/in-memory Transport for tests. Byte-stream carriers document length-prefix (or equivalent) delimiting as transport-adapter-owned.

Shipped shape (TS `Transport` interface; Rust `spoke_connect::remote::Transport` trait with `async fn send(&self, &[u8]) -> Result<(), TransportError>` / `async fn recv(&self) -> Result<Vec<u8>, TransportError>` / `async fn close(&self)` default no-op, `Send + Sync`):

```ts
export type EnvelopeBytes = Uint8Array; // bytes of exactly one UTF-8 JSON connect envelope

export interface Transport {
  send(envelope: EnvelopeBytes): Promise<void>; // one envelope per call
  recv(): Promise<EnvelopeBytes>;               // rejects on transport close (fail-fast)
  close?(): void | Promise<void>;               // optional, idempotent
}
```

`loopbackTransportPair()` / `loopback_transport_pair()` return the client+server ends of one in-memory connection; closing either end fails the peer's pending `recv` like a real connection drop.

### D4 — Port-method ops over open vocabulary (no schema change)

Each BaselinePorts method maps to a connect invoke with:

- `op` under the reserved product prefix `port.*` (open string; not the core ops list `upsert` / `check` / …);
- `payload` as existing `OpaqueJson` carrying method arguments (snake_case field names);
- success `payload` carrying the success value `T`;
- failure via existing `ConnectInvokeResponse` error branch + shared `ErrorEnvelope`.

Core ops vocabulary remains for full ops-family remote calls. RemoteAdapter BaselinePorts proxy uses **port-method** ops so local `orchestrate*` can run on the caller and perform remote I/O per port call. Required capability for baseline port ops: `spoke-baseline`, authorized through the existing product `op_capability_requirements` map on hosts.

Catalogue (baseline, shipped):

| Method | `op` | Request `payload` | Success `payload` |
|--------|------|-------------------|-------------------|
| `getKnowledgeEntry` | `port.knowledge.get` | `{ "entry_id": string }` | `KnowledgeEntry` |
| `putKnowledgeEntry` | `port.knowledge.put` | `{ "entry": KnowledgeEntry, "expected_base_revision": number \| null }` | `KnowledgeEntry` |
| `getRelation` | `port.relation.get` | `{ "relation_id": string }` | `Relation` |
| `putRelation` | `port.relation.put` | `{ "relation": Relation, "expected_base_revision": number \| null }` | `Relation` |
| `listKnowledgeEntries` | `port.scope.list_knowledge_entries` | `{ "scope": Scope }` | `KnowledgeEntry[]` |
| `listTimelineEvents` | `port.scope.list_timeline_events` | `{ "scope": Scope }` | `TimelineEvent[]` |
| `putFindings` | `port.finding.put` | `{ "findings": Finding[] }` | `Finding[]` |
| `listRules` | `port.rule.list` | `{ "rule_refs": string[] }` | `Rule[]` |
| `listPeerHostCapabilityManifests` | `port.host.list_peer_manifests` | `{}` | `HostCapabilityManifest[]` |
| `getHostCapabilityManifest` | *(none — session cache)* | — | remote hello `host`, cached at establish, **no round-trip** |

Optional families (reserved, not shipped in the baseline RemoteAdapter): `project`/`compute` → `port.computable.*` (`l2-computable`); `listForkTimelineEvents` → `port.fork.list_timeline_events` (`l5-fork`).

**No `schemas/` changes** — open `op` + opaque `payload` already carry this design.

### D5 — HostManifestPort on a single-peer RemoteAdapter

- `getHostCapabilityManifest` → remote peer’s `HostCapabilityManifest` (hello `host`, cached at session establish). Not the local client manifest.
- `listPeerHostCapabilityManifests` → proxy to the remote via `port.host.list_peer_manifests` (peers-only semantics enforced on the host; empty list valid).

Session state is per peer so the multi-peer router (D11) composes multiple RemoteAdapters without changing consumer orchestrator callsites.

### D6 — Concurrent invokes

Concurrent port calls on one established session are allowed. Outbound `sequence` is allocated atomically at send time (per-peer counter starting at 0, no wrap — exhaustion closes the session). Responses demultiplex on `request_id` (plus echo checks for `session_id` and `sequence`). Completions may arrive out of order. RemoteAdapter owns one receive loop over `Transport.recv`; BaselinePorts callers do not call `recv` directly. Each pending invoke carries an adapter-owned timeout timer; on elapse only that waiter fails (`details.kind = "timeout"`) and the session stays usable. Protocol v1 defines no retry — callers may re-invoke with a fresh `request_id`. In-flight `request_id`s are never reused; duplicate/unknown responses are dropped.

### D7 — Error mapping

| Class | Surface |
|-------|---------|
| Application / lifecycle rejects (including remote `SpokeRejectCode` on the error branch) | `SpokeResult` reject (preserve codes via existing envelope mapping) |
| Dispatch deny (`op_unsupported` / `capability_missing`) | `SpokeResult` reject `CAPABILITY_PORT_MISSING` with `details.wire_code` |
| Unknown wire codes | `SpokeResult` reject `INVALID_INPUT` with `details.wire_code` |
| Transport I/O | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "transport"` |
| Session closed / connection loss | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "session_closed"`; adapter → `Closed`, all pending failed |
| Invoke timeout | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "timeout"` (waiter only) |
| Panic containment at FFI boundary | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "panic"` for panics caught by `catch_unwind` around futures driven by exported FFI-object `block_on` sites (sync port methods, dial, and smoke-host start); waiter only, never unwinds across FFI — `message` carries the raw panic payload (developer-oriented, consistent with uniffi scaffolding). Foreign-callback panics inside `ForeignCallbackTransport` `spawn_blocking` do not use `kind = "panic"`; they fail the blocking join and surface as transport `Io`. |
| Correlation mismatch | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "correlation_mismatch"` |
| Sequence exhaustion | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "sequence_exhausted"`; session closed |
| Envelope-auth rejection (missing / invalid / session-unbound response or request) | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind` ∈ {`envelope_auth_missing`, `envelope_auth_invalid`, `envelope_auth_session_unbound`} (waiter only; session state untouched, session stays usable) |
| Dial / hello / allowlist / nonce failure | Constructor error — no adapter instance (dial kinds: `config` / `handshake` / `timeout` / `protocol_version_mismatch`) |

Port methods always settle to `SpokeResult`; typed transport errors stay on lower-level connect APIs.

### D8 — Async Rust ports use `async-trait` + Send futures

Operations port traits use the `async-trait` crate so existing dyn availability probes (`as_computable`, `as_fork_timeline`) stay object-safe. Port method futures are `Send`. The operations crate does not take a runtime dependency for the library itself. The Rust `Transport` trait and the `RemoteAdapter` port impls follow the same `async-trait` + Send convention.

### D9 — spoke-connect may depend on spoke-operations

spoke-connect depends on spoke-operations for port and result types used by RemoteAdapter — gated so default builds stay lean: the TS `./remote` subpath export (root bundle excludes RemoteAdapter symbols) and the Rust `remote-adapter` cargo feature (default `spoke-connect` builds do not require spoke-operations). Operations stays pure (no reverse dependency on connect).

### D10 — Per-envelope authentication enforcement (protocol_version 2)

RemoteAdapter enforces envelope-authentication (protocol_version 2 post-hello rules) on every post-hello envelope it emits or accepts, while the hello exchange remains at the core `PROTOCOL_VERSION` until a dedicated hello bump (normative field sets, canonicalization, and verify rules: [`spoke-connect.md`](spoke-connect.md) §[Envelope authentication (protocol_version 2)](spoke-connect.md#envelope-authentication-protocol_version-2)):

- **Establish:** the dial verifies the responder hello (`peer_nonce` dial-binding assert) and then the responder's `ConnectSession` snapshot with `spoke-connect-session-jcs-v1` against the responder's hello Ed25519 public key — wire form, before typed deserialization — including the step-6 peer-id binding (`initiator_peer_id` / `responder_peer_id` equal the authenticated hello peer ids). A snapshot that fails verification fails the dial; no adapter instance is created.
- **Invoke — outbound:** every `ConnectInvokeRequest` carries a `spoke-connect-invoke-request-jcs-v1` signature over `{session_id, sequence, request_id, op, payload}` (plus `auth` when attached), computed with the adapter's hello-identity seed. The pending waiter registers synchronously before signing, and sends serialize in allocation order.
- **Invoke — inbound:** every `ConnectInvokeResponse` runs the correlation echo check first and then `spoke-connect-invoke-response-jcs-v1` verification over the exact wire branch against the peer's hello Ed25519 public key. A forged, tampered, stripped-signature, or session-unbound response fails only that waiter and leaves session state untouched; the session stays usable.

The `authenticate_*` / `verify_*` helpers are module-internal (TS) / crate-private (Rust); consumers reach enforcement only through the adapter surface. Rejections surface as `SpokeResult` rejects with `INTERNAL_ERROR` and `details.kind` ∈ {`envelope_auth_missing`, `envelope_auth_invalid`, `envelope_auth_session_unbound`} (D7 rows).

### D11 — Multi-peer capability router (shipped)

A multi-peer capability router (`connectMultiPeerRouter` / `connect_multi_peer_router`) composes N registered per-peer `RemoteAdapter` instances behind the same async `BaselinePorts` surface, so `orchestrate*(router, req)` selects and reaches a capable peer without a per-op `peer_id`. The router sits above the per-peer adapters and below `orchestrate*`; consumers dial each peer's adapter (signed-hello handshake, allowlist, session establishment) and register the established adapter. Shipped in TypeScript (`@42ch/spoke-connect/remote`) and Rust (`remote-adapter` feature) with behavioral parity.

**Registry (`registerPeer` / `unregisterPeer` / `listPeers`):** `registerPeer(adapter)` accepts an established adapter, returns its `peer_id`, and caches the peer's `HostCapabilityManifest` (the `host` field from the authenticated hello) at registration; re-registering the same `peer_id` replaces the stored adapter. `unregisterPeer(peer_id)` removes the peer from selection and leaves the adapter's lifecycle with the consumer; unregistering an unknown `peer_id` leaves the registry unchanged. `listPeers()` returns the registered `peer_id`s in registration order. A registered peer whose session leaves `Established` (e.g. `Closed`, `Disconnected`, `Handshaking`) is excluded from selection on the next call; the registry keeps the peer until the consumer unregisters it.

**Selection (inputs + algorithm):** selection matches each registered peer's cached `HostCapabilityManifest` against the operation request:
- **Hard filters:** the peer's `capabilities[]` includes the operation's required capability (`spoke-baseline` for the `upsert` / `promote` / `relate` / `check` / `assemble` families and the `port.*` baseline ops; `l2-computable` for `project` / `compute` and the `port.computable.*` ops); when the request carries a namespace (derived from the payload `Scope`), the peer's `namespaces[]` includes it exactly — v1 namespace matching is exact, so a peer declaring `namespaces: ["*"]` declares the literal string `"*"`; when the peer manifest and the request both declare `authority.scope_key`, the two match exactly, and a single-sided declaration passes that gate.
- **Soft preference:** peers whose `roles[]` include the operation's preferred role (`checker` for `check`, `assembler` for `assemble`, `l2-computable` for `project` / `compute`) sort ahead of equally capable peers; capable peers without the role stay eligible.
- **Deterministic tie-break:** the surviving candidates resolve to the lowest `peer_id` in lexicographic UTF-8 byte order — a pure function of the candidate set, so the same peers and the same request always select the same peer.

**Aggregation:** the router implements `HostManifestPort` over the composed peer set with two views, both aggregated locally from the cached per-peer manifests. `getHostCapabilityManifest` returns a synthesized manifest carrying the router's own `host_id`, the set-union (deduplicated) of connected peers' `capabilities` / `roles` / `namespaces` plus the `tools[]` union (dedup by `capability_id`, lexicographic order — per D14), and `extensions.router.peers` listing the contributing `peer_id`s in lexicographic UTF-8 byte order; per-peer authority reads use the per-peer view. `listPeerHostCapabilityManifests` returns one entry per connected peer — each peer's own cached hello manifest — ordered by `peer_id` in lexicographic UTF-8 byte order; a router with zero registered peers returns `[]`. The composed view is an introspection surface and stays advisory: capability and tool routing decisions always re-check the cached per-peer manifests.

**Failure policy:** when no registered peer passes the hard filters, the router rejects with the locked terminal reject — `SpokeResult` `CAPABILITY_PORT_MISSING` with `details.wire_code = "no_capable_peer"` and `details.kind = "no_capable_peer"`. The reject is deterministic for a given peer set and terminal for that request; the consumer registers a satisfying peer and re-invokes with a fresh `request_id`. When the selected peer's session fails mid-operation, the router returns the underlying `SpokeResult` reject exactly as the peer's adapter produced it (`INTERNAL_ERROR` with `details.kind` preserved, e.g. `transport` or `session_closed`), keeping the failure attributable to the peer that served the call; the router re-selects against the current peer set on the consumer's next invocation.

**Public surface:** `connectMultiPeerRouter(options)` / `connect_multi_peer_router(options)` plus `registerPeer` / `register_peer`, `unregisterPeer` / `unregister_peer`, `listPeers` / `list_peers`, the async `BaselinePorts` six families, and the tool-invoke face `invokeTool` / `invoke_tool` (D14). Per-peer adapters stay encapsulated after registration — the router holds only the adapter references the consumer dialed. The constructor carries no dial options; consumers dial each peer's `RemoteAdapter` and register the established adapter, and an unconfigured router uses the `multi-peer-router` host id.

### D12 — Native-binding FFI surface (delivered)

The RemoteAdapter contract ships over FFI as a synchronous surface on the `spoke-connect` cdylib (`ffi` + `remote-adapter` features), so native binding hosts consume the same adapter lifecycle as the Rust and TypeScript clients. The surface is generated into all five native binding channels (C#, Go, Kotlin, Python, Swift).

- **Cdylib-owned runtime:** the cdylib owns a process-wide multi-thread tokio runtime, initialized once. Every exported adapter call is a synchronous block-on-async call over that runtime; foreign callbacks run through the runtime's `spawn_blocking` pool, so a blocking `recv` never monopolizes an async worker.
- **`RemoteAdapterFFI` object:** `connect_remote_adapter_ffi(transport, local_seed, local_manifest_json, remote_pubkey, allowlist, invoke_timeout_ms)` dials through the binding's transport (signed-hello handshake, allowlist, session establishment) and returns an established adapter handle. The object exposes session info — `state` / `session_id` / `remote_peer_id` / `remote_manifest` — and the async `BaselinePorts` methods as synchronous calls. Payloads and manifests cross the boundary as JSON strings, keys as raw 32-byte arrays, peer ids as strings.
- **Foreign-callback `Transport` interface:** the binding implements the message-oriented `Transport` (`send` accepts exactly one envelope's bytes; `recv` blocks until one envelope arrives or the connection closes; `close` is idempotent resource release). `loopback_transport_pair()` / `LoopbackTransport` / `LoopbackTransportPair` expose the in-memory loopback over FFI so bindings exercise the surface without a network carrier.
- **`FfiError` (D7 mapping):** `Dial { kind, message }` for constructor / dial failures before an adapter exists (`config` / `handshake` / `timeout` / `protocol_version_mismatch` kinds); `Rejected { code, message, kind, wire_code }` for invoke-path `SpokeResult` rejects — application codes preserved, `INTERNAL_ERROR` rows carry `kind`, dispatch deny and unknown wire codes carry `wire_code`. The callback transport's own failures surface as `TransportError` (`Closed` / `Io`).
- **`MultiPeerRouterFFI`:** `new_multi_peer_router_ffi()` returns the router as a synchronous object — registry (`register_peer` / `unregister_peer` / `list_peers`), the async `BaselinePorts` six families, and the two `HostManifestPort` aggregation views, all block-on-async over the same runtime. `register_peer` accepts an established `RemoteAdapterFFI` handle and returns its `peer_id`.

### D13 — Reverse tool invocation + RemoteAdapter tool serving (shipped)

The `RemoteAdapter` receive loop classifies inbound envelopes **request-first**: a document carrying `op` is a `ConnectInvokeRequest` and is classified before the response demux, so a reverse invoke can never satisfy the response discriminator. Both response discriminators are hardened accordingly — TS `isConnectInvokeResponse` rejects `op`-bearing documents and Rust `wire_response_correlation` returns `None` when the document carries `op` (the shared TS guard also covers the node connect-client demux; no response ever carried `op` per the wire field tables). A reverse invoke arriving at the dialer is served through the frozen pipeline order (mirrors spec §Envelope authentication — Verify rules; both implementations follow exactly):

1. **Stray check** — a `session_id` bound to a different live session is ignored (no response); a `session_id` bound to no established session stays on this path and is rejected `auth_failed` at step 3. The adapter is single-session, so only the unbound case is reachable here (rejected `auth_failed` with `details.kind = envelope_auth_session_unbound`).
2. **Sequence peek** (non-mutating) — mismatch → error branch `invalid_sequence`, no side effect, counter NOT advanced.
3. **Envelope-auth verify** against the peer's hello Ed25519 public key — failure → `auth_failed` with `details.kind` ∈ {`envelope_auth_missing`, `envelope_auth_invalid`, `envelope_auth_session_unbound`}; session state NOT advanced (auth-before-advance — the inbound counter advances only after verify passes).
4. **Advance** the inbound counter.
5. **Dispatch gate** — `Session.dispatchAllowed`-level check (never a raw requirements-map composition): the `tools.*` required capability is the op string itself, evaluated against `negotiated_capabilities` (the intersection of the two authenticated hello manifests, computed and stored at session establish).
6. **Handler or deny** — the registered handler runs off the receive loop (gate steps 2–4 are awaited inline so they serialize per session; dispatch may interleave); denial answers the error branch.
7. **Signed response** — `authenticateInvokeResponse` with correlation echo of `session_id` / `sequence` / `request_id`.

**Deny-code matrix (shipped — existing wire codes; no new envelope error codes):**

| Condition | Wire error code | Notes |
|---|---|---|
| Gate fail: `tools.*` op ∉ `negotiated_capabilities` | `op_unsupported` | No handler side effect |
| Gate pass but no registered handler for that `capability_id` | `op_unsupported` | Fail-closed serving |
| Unknown non-`tools` op (no core-table row, no product map row) | `op_unsupported` | Existing behavior |
| Signature missing/invalid/session-unbound | `auth_failed` | Existing envelope-auth branch |
| Sequence gap/duplicate | `invalid_sequence` | No counter advance |

The invoker-side mapping is the **existing D7 row**: an error branch carrying `op_unsupported` / `capability_missing` maps at the invoking library surface to `SpokeResult` reject `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved — the connect-layer blame, distinct from the operations-layer `CAPABILITY_PORT_MISSING` (port-family absence, `details.capability`).

**Tool-handler registry:** `registerToolHandler(capabilityId, handler)` — grammar-asserted via `parseToolCapabilityId` (a non-`tools.` id throws (TS) / panics (Rust) — programmer misuse); duplicate registration for the same `capability_id` overwrites (last-wins, documented); the registry MUST NOT mutate the local manifest — the manifest's `tools[]` (carried through hello) is the discovery source. A registry/manifest mismatch is surfaced at invoke time: a manifest-declared tool with no registered handler is denied fail-closed (`op_unsupported` → `CAPABILITY_PORT_MISSING`); `validateManifestTools` checks manifest-internal consistency only. A throwing/rejecting handler answers the error branch — a crash maps to a synthesized `INTERNAL_ERROR` (Rust: `catch_unwind` containment at the future-poll boundary, mirroring the existing invoke path; TS: `toErrorEnvelope`) — an application `SpokeReject` passes through verbatim.

**Tool-invoke face:** `invokeTool(capabilityId, arguments): Promise<SpokeResult<unknown>>` / `invoke_tool(&self, capability_id, arguments: Value) -> SpokeResult<Value>` is the public forward tool-invoke face (at the session layer "forward" vs "reverse" is the same envelope machinery — per-direction counters, symmetric envelopes). It reuses the existing `#invokeOp` / `invoke_op` wire-order serialization (allocate → sign → send tail) including the deferred-send poison-close, fails fast on a non-`tools.` id (`INVALID_INPUT` with `details.capability_id`, no wire traffic), maps denies through the D7 row, and gates the success payload to the frozen `{ result: <opaque JSON> }` shape — a success payload without a `result` key rejects `INTERNAL_ERROR` `details.kind = "transport"` (session stays usable).

Handler type (TS surface names the parameter `args` — `arguments` is rejected as a binding identifier by TypeScript strict mode; the positional contract is unchanged):

```ts
export type ToolHandler = (
  args: Record<string, unknown>,
) => Promise<SpokeResult<unknown>>;
```

Rust: `pub type ToolHandler = Arc<dyn Fn(Value) -> BoxFuture<'static, SpokeResult<Value>> + Send + Sync>`.

### D14 — `connectResponder` + tool-aware multi-peer routing (shipped)

**Responder:** `connectResponder(options)` / `connect_responder(options)` productizes the demo-server responder recipe (`examples/connect-demo/server/src/host/connect-host.ts`) into the library with the same gating as `RemoteAdapter` (TS `./remote` subpath; Rust `remote-adapter` feature; default builds stay lean). Options shape (frozen): `{ transport, identity, manifest, allowlist, peerKeys, ports?, invokeTimeoutMs? }` — **no `toolHandlers` constructor option** (handlers register post-construction via `registerToolHandler`, one registration path mirroring `RemoteAdapter`); `peerKeys` is the responder-owned trust config (preconfigured hello public keys for allowlisted peers).

**Intentional TS↔Rust surface divergence (documented in code):** TS `connectResponder(options): Promise<ConnectResponder>` returns the responder instance; Rust `connect_responder(options) -> Arc<ConnectResponder>` returns the instance directly with **no `Result`** — the TS construction-time seed-length throw is type-enforced in Rust (`RemoteIdentity.seed: [u8; 32]`), so the contract's error slot is uninhabitable; handshake failures surface via `state → Closed` + transport close, exactly like TS. `ConnectResponderState` is the same vocabulary as the adapter (`Disconnected` / `Handshaking` / `Established` / `Closed`; Rust aliases `RemoteAdapterState`).

**Handshake recipe (ported verbatim; carried-over demo behaviors documented):** allowlist-first (fail-closed before any signature work) → `verifyHelloEd25519` (peer_id binding against the preconfigured peer key) → nonce single-use record → dial-bound responder hello (5-field signed object carrying the initiator's nonce as `peer_nonce`) → signed `ConnectSession` snapshot (`authenticateSession`). The empty-intersection fallback is preserved as-is: the wire snapshot requires ≥1 negotiated capability, so a degenerate dial emits `["spoke-baseline"]`; the dialer computes its own intersection from the hellos, so there is no authorization impact. Unparseable inbound closes the connection (demo semantics).

**Serving:** the serve loop uses the same gate serialization as the adapter (peek → verify → advance, awaited inline; dispatch fires without blocking the loop). `port.*` ops are served against the injected async `BaselinePorts` via the D4 catalogue with the product `op_capability_requirements` map (every baseline `port.*` op requires `spoke-baseline`); absent `ports` still answers `port.*` with the dispatch-deny branch (`op_unsupported`); unknown `port.*` methods / unknown ops → `op_unsupported`. `tools.*` is served through registered handlers per the D13 pipeline. The port success payload is the raw D4 value `T` (the dialer's `invoke_mapped` validates it); the tool success payload is the `{ result }` shape.

**Reverse face:** `invokeTool` / `invoke_tool` on the responder — outbound counter, `authenticateInvokeRequest` signing, send-tail wire-order serialization, response demux by `request_id` with correlation echo + `verifyInvokeResponseAuth`, per-waiter timeout (**waiter-only** — a timed-out request that did hit the wire leaves the session usable), and the deferred-send poison-close mirror (a waiter that settles while its send is still queued ⇒ the allocated outbound sequence never hit the wire ⇒ close the session). Same success-payload gate and D7 deny mapping as the adapter.

**Public surface:** `invokeTool`, `registerToolHandler`, session info (`sessionId`, `remotePeerId`, `remoteManifest`, `state`), `close`. No envelope-auth/sequence internals exported (`#`-private in TS; no pub internals in Rust).

**Router tool awareness:** `MultiPeerRouter.invokeTool(capabilityId, arguments)` / `invoke_tool(…)` (name frozen — **no `invokeToolOnPeer`**; peer selection is internal):

- **Hard filter:** the registered peer's cached (hello) manifest `capabilities[]` contains the exact tool capability string — the router's private selection table gains the `tools.` prefix rule (required capability IS the op string; same `Option<&str>` widening as core dispatch). No namespace/authority/role filters for tools: the capability string is already ns-scoped and tool payloads carry no `Scope`.
- **Deterministic tie-break:** lowest `peer_id` (UTF-8 byte order).
- **None → terminal reject:** the existing `no_capable_peer` reject — `CAPABILITY_PORT_MISSING` with `details.wire_code` / `details.kind = "no_capable_peer"` and `details.op = capability_id`. The selected peer's underlying reject is returned as-is (no alternate-retry).
- **Delegation boundary:** the router never crafts envelopes — it delegates through the selected adapter's `invokeTool` / `invoke_tool`; `RoutedRemoteAdapter` gains the tool face, satisfied by `RemoteAdapter`.
- **Composed manifest:** `getHostCapabilityManifest` unions `tools[]` alongside the existing unions — dedup by `capability_id`, sorted by `capability_id` lexicographically (UTF-8 byte order) for stability; `listPeerHostCapabilityManifests` unchanged (per-peer views already carry their own `tools`).
- **Topology note:** router-registered adapters are locally dialed (local = initiator), so router tool invokes travel initiator→responder and are served by the **peer's responder-side** tool serving — the dual-peer proofs drive tool-serving responder doubles (connectResponder-based or minimal test responders with the D13 pipeline), not dialer-side `registerToolHandler` alone.

### D15 — Reverse-invoke FFI face (invokeTool over FFI)

`RemoteAdapterFFI` and `MultiPeerRouterFFI` expose the shipped tool-invoke library faces — `RemoteAdapter::invoke_tool` and `MultiPeerRouter::invoke_tool` (D13/D14) — as synchronous methods for native binding hosts, joining the existing objects on the D12 cdylib surface (`ffi` + `remote-adapter` features; the surface is generated into the same five native binding channels — C#, Go, Kotlin, Python, Swift). Every call is a block-on-async call over the cdylib-owned runtime (D12): the foreign thread blocks while the library future resolves. Tool arguments cross the boundary as a JSON string (`arguments_json`, parsed inside Rust); success returns the tool's `result` payload value serialized as a JSON string.

```rust
impl RemoteAdapterFFI {
    /// `capability_id` must match the `tools.<ns>.<tool_id>` grammar — otherwise
    /// fail-fast `FfiError::Rejected { code: "INVALID_INPUT", .. }` with zero wire
    /// traffic. Success returns the tool's `result` payload as a JSON string;
    /// deny / timeout / session failures map through the D7 rows (table below).
    fn invoke_tool(&self, capability_id: String, arguments_json: String) -> Result<String, FfiError>;
}
impl MultiPeerRouterFFI {
    /// Same invoke semantics. No capable peer → `Rejected {
    /// code: "CAPABILITY_PORT_MISSING", kind: Some("no_capable_peer"),
    /// wire_code: Some("no_capable_peer") }` with the capability id embedded
    /// in the message.
    fn invoke_tool(&self, capability_id: String, arguments_json: String) -> Result<String, FfiError>;
}
```

**Grammar fail-fast:** `capability_id` is foreign input. A non-`tools.<ns>.<tool_id>` id fails fast with `FfiError::Rejected { code: "INVALID_INPUT", kind: None, wire_code: None }` and zero wire traffic on both faces — the library grammar reject (`parse_tool_capability_id`, the `spoke-operations` public API) passes through, the offending id rides in `message`, and the router rejects before any peer selection. `arguments_json` parses before the library call through the FFI face's JSON convention (`parse_json_field`): malformed JSON rejects `INVALID_INPUT` with the parse error in `message` (`kind` / `wire_code` `None`), also with zero wire traffic.

**Router semantics:** `MultiPeerRouterFFI.invoke_tool` adds no routing logic of its own. Exact-capability hard filter over the cached hello manifest, lowest-`peer_id` (UTF-8 byte order) tie-break, and the terminal `no_capable_peer` reject are the library router's (D14); the selected peer's underlying reject returns as-is. With no registered peer advertising the exact tool capability, the terminal reject crosses as `Rejected { code: "CAPABILITY_PORT_MISSING", kind: Some("no_capable_peer"), wire_code: Some("no_capable_peer") }` with the capability id embedded in `message`.

**Concurrency:** concurrent `invoke_tool` calls from multiple foreign threads are allowed — each call blocks its own calling thread and the shared runtime multiplexes the library futures (D6).

**Error rows (D7 extension by reference):** every library `SpokeResult` reject crosses through the existing D7 mapping — `code` and `message` preserved, `details.kind` → `kind`, `details.wire_code` → `wire_code`; no other detail field is surfaced separately (the library reject's `details.capability_id` and `details.op` ride in `message`). The `FfiError` shape is unchanged and no wire semantics are added.

| Invoke-path failure | `FfiError` row | Library source |
|---------------------|----------------|-----------------|
| Malformed `arguments_json` (FFI-boundary parse) | `Rejected { code: "INVALID_INPUT", kind: None, wire_code: None }`, parse error in `message` | FFI JSON convention (D12 face) |
| `capability_id` fails the `tools.<ns>.<tool_id>` grammar | `Rejected { code: "INVALID_INPUT", kind: None, wire_code: None }`, offending id in `message`; zero wire traffic | D13 grammar fail-fast |
| Dispatch deny — capability not negotiated, or serving side has no registered handler (peer answers `op_unsupported`; `capability_missing` is the same class) | `Rejected { code: "CAPABILITY_PORT_MISSING", kind: None, wire_code: Some("op_unsupported") }` | D13 deny matrix + D7 dispatch-deny row |
| Peer answers `auth_failed` (envelope-auth failure) | `Rejected { code: "INTERNAL_ERROR", kind: Some("envelope_auth_missing" \| "envelope_auth_invalid" \| "envelope_auth_session_unbound") }` | D13 serving pipeline + D7 envelope-auth row |
| Peer answers `invalid_sequence` | `Rejected { code: "INVALID_INPUT", kind: None, wire_code: Some("invalid_sequence") }` | D13 deny matrix + D7 unknown-wire-code row |
| Router has no capable peer for the tool | `Rejected { code: "CAPABILITY_PORT_MISSING", kind: Some("no_capable_peer"), wire_code: Some("no_capable_peer") }`, capability id in `message` | D14 router tool face |
| Per-waiter invoke timeout | `Rejected { code: "INTERNAL_ERROR", kind: Some("timeout") }` — waiter only, session stays usable | D7 invoke-timeout row |
| Session closed / invoke after `close` | `Rejected { code: "INTERNAL_ERROR", kind: Some("session_closed") }` | D7 session-closed row |
| Transport I/O failure during the invoke | `Rejected { code: "INTERNAL_ERROR", kind: Some("transport") }` | D7 transport row |
| Success payload without a `result` key | `Rejected { code: "INTERNAL_ERROR", kind: Some("transport") }` — session stays usable | D13 success-payload gate |
| Response correlation mismatch | `Rejected { code: "INTERNAL_ERROR", kind: Some("correlation_mismatch") }` | D7 correlation-mismatch row |
| Response envelope-auth verify failure (forged / tampered / stripped-signature / session-unbound response) | `Rejected { code: "INTERNAL_ERROR", kind: Some("envelope_auth_missing" \| "envelope_auth_invalid" \| "envelope_auth_session_unbound") }` — waiter only, session stays usable | D7 envelope-auth row (D10 enforcement) |
| Sequence exhaustion at send allocation | `Rejected { code: "INTERNAL_ERROR", kind: Some("sequence_exhausted") }`; the session closes | D7 sequence-exhaustion row |
| Panic inside the block-on-async invoke future | `Rejected { code: "INTERNAL_ERROR", kind: Some("panic") }` — waiter only, never unwinds across the FFI boundary; `message` carries the raw panic payload | D7 panic-containment row (D12 runtime) |

### D16 — Responder / tool-serving FFI face (ToolHandler + ConnectResponderFFI)

The tool-serving library faces — `ToolHandler` registration and serving on `RemoteAdapter` / `ConnectResponder`, and the `connect_responder` factory (D13/D14) — cross to native binding hosts as the `ToolHandler` callback interface, `RemoteAdapterFFI.register_tool_handler`, and the `ConnectResponderFFI` object, joining the D12/D15 surface on the `spoke-connect` cdylib (`ffi` + `remote-adapter` features; generated into the same five native binding channels — C#, Go, Kotlin, Python, Swift). Every method is a sync block-on-async call over the cdylib-owned runtime (D12). `RemoteAdapterFFI.register_tool_handler` is the dialer-side serving face — the registration surface for responder→dialer reverse invokes, the serving direction the bidirectional loopback smokes exercise. Tool arguments and results cross as JSON strings, keys as raw byte arrays, peer ids as strings (D12 boundary conventions).

**Names:** the foreign callback interface is **`ToolHandler`**, unprefixed — the callback-`Transport` naming precedent: a callback face carries the library-face concept name, while wrapper objects carry the `FFI` suffix (`RemoteAdapterFFI`, `ConnectResponderFFI`). The Rust-side name collision with the library handler type resolves by import alias (`use crate::remote::ToolHandler as RemoteToolHandler`), the mirror of the existing `use super::foreign_transport::Transport as FfiTransport`. Boundary-type discipline follows the D12 compile face: callback parameters are `Box<dyn Trait>` (the `connect_remote_adapter_ffi(transport: Box<dyn FfiTransport>, …)` precedent; `Arc<dyn …>` parameters are not a compile precedent here), with Rust converting to a long-held `Arc` via `Arc::from(box)` exactly as dial does. Data crosses the boundary only as `String` / `Vec<u8>` / `Vec<String>` / `HashMap<String, Vec<u8>>` / `Option<u64>`, plus object handles and the two callback interfaces.

```rust
// D16 — tool-serving faces
#[uniffi::export(callback_interface)]
trait ToolHandler: Send + Sync {
    /// Sync foreign callback; the bridge runs each call on the cdylib runtime's
    /// `spawn_blocking` pool (the `ForeignCallbackTransport` bridge, including
    /// the JoinError mapping; see the threading model below).
    /// Ok(json) → SpokeResult::Ok; Err(Rejected{..}) → the application
    /// SpokeReject passes through verbatim; Err(Dial{..}) (non-contract) /
    /// foreign exception / panic → INTERNAL_ERROR reject (details: None,
    /// mirroring the library `catch_unwind` containment).
    fn handle(&self, arguments_json: String) -> Result<String, FfiError>;
}

impl RemoteAdapterFFI {
    /// Dialer-side serving registration (serves responder→dialer reverse
    /// invokes): the capability id is pre-validated via
    /// `parse_tool_capability_id` — an invalid id rejects
    /// `FfiError::Rejected { code: "INVALID_INPUT", .. }` with zero wire
    /// traffic; a valid id registers on the library face (last-wins; the
    /// registry never mutates the manifest).
    fn register_tool_handler(&self, capability_id: String, handler: Box<dyn ToolHandler>) -> Result<(), FfiError>;
}

#[derive(uniffi::Object)]
struct ConnectResponderFFI { inner: Arc<ConnectResponder> }

#[uniffi::export]
fn connect_responder_ffi(
    transport: Box<dyn Transport>,        // connected (host-accepted) callback Transport
    seed: Vec<u8>,                        // 32-byte Ed25519 seed (Vec<u8> boundary convention; length validated inside Rust)
    manifest_json: String,                // HostCapabilityManifest JSON (optional tools[] included)
    allowlist: Vec<String>,               // fail-closed dialer allowlist
    peer_keys: HashMap<String, Vec<u8>>,  // peer_id → 32-byte Ed25519 pubkey (responder-owned trust config)
    invoke_timeout_ms: Option<u64>,       // None → the library default (DEFAULT_INVOKE_TIMEOUT_MS = 5000 ms)
) -> Result<Arc<ConnectResponderFFI>, FfiError>;

impl ConnectResponderFFI {
    fn register_tool_handler(&self, capability_id: String, handler: Box<dyn ToolHandler>) -> Result<(), FfiError>;
    fn invoke_tool(&self, capability_id: String, arguments_json: String) -> Result<String, FfiError>;
    fn state(&self) -> String;            // "Disconnected" | "Handshaking" | "Established" | "Closed"
    fn session_id(&self) -> Option<String>;
    fn remote_peer_id(&self) -> Option<String>;
    fn remote_manifest(&self) -> Option<String>;  // manifest JSON string (same as RemoteAdapterFFI)
    fn close(&self);
}
```

**Options mirror (1:1 with the shipped `ConnectResponderOptions`, `crates/spoke-connect/src/remote/responder.rs`):**

| Library field | FFI parameter | Notes |
|---|---|---|
| `transport: Arc<dyn Transport>` | `transport: Box<dyn Transport>` | callback → `Arc::from` wrapped in the `ForeignCallbackTransport` bridge (dial same) |
| `identity: RemoteIdentity` (`seed: [u8;32]`) | `seed: Vec<u8>` | FFI layer validates exactly 32 bytes; failure → `Dial { kind: "config" }` (dial seed-length precedent) |
| `manifest: HostCapabilityManifest` | `manifest_json: String` | parsed inside Rust with `serde_json`; failure → `Dial { kind: "config" }` (dial precedent) |
| `allowlist: Vec<String>` | `allowlist: Vec<String>` | passed through |
| `peer_keys: HashMap<String, [u8;32]>` | `peer_keys: HashMap<String, Vec<u8>>` | each value validated as 32 bytes (failure → `Dial { kind: "config" }`). **Mandatory parameter**: an allowlisted peer without a preconfigured key fails the handshake fail-closed — an FFI responder without this parameter could never establish a session |
| `ports: Option<Arc<dyn BaselinePorts>>` | (no parameter; pinned `None`) | no foreign-callback ports face; `port.*` invokes hit the library's documented absent-ports deny branch (non-goals below) |
| `invoke_timeout_ms: Option<u64>` | `invoke_timeout_ms: Option<u64>` | passed through (None → library default 5000 ms) |

**Constructor semantics (library-faithful — not a block-on handshake):** the library `connect_responder` factory returns a `Handshaking` responder immediately (the handshake runs on the background serve loop; the dialer hello is the sync point; the factory has **no error path** — the intentional TS↔Rust divergence recorded in D14). The FFI constructor carries the same semantics: the block-on covers only the factory future, which completes immediately, and the returned responder is in `Handshaking`. The `Result` slot carries FFI-side config-validation failures only — manifest JSON, seed length, or peer-key length → `Dial { kind: "config" }`. Handshake failures (allowlist deny, hello-verify deny, peer never appearing) produce **no error row**: they surface as `state() → "Closed"` with the transport closing, and hosts / tests / smokes poll `state()` to `Established` under a bounded wait, matching the TS and Rust consumers.

**Grammar pre-validation on registration (resolved):** over FFI, `capability_id` is foreign input, so `register_tool_handler` returns `Result<(), FfiError>` — a non-`tools.` grammar id rejects `Rejected { code: "INVALID_INPUT", kind: None, wire_code: None }` with the offending id in `message` and zero wire traffic. The wrapper pre-validates with `parse_tool_capability_id` (the `spoke-operations` public API) before the library call, so the library's grammar panic stays unreachable through the FFI boundary. Rationale: the `ffi.rs` convention validates foreign input before calling the library (`parse_json_field` / seed-length precedents); the mapping is symmetric with `invoke_tool` on the same grammar failure (same input, same row); a panic is not a contractable cross-uniffi error channel; and bindings smokes assert an error row instead of catching a crash. The library faces are untouched — TS throws / Rust panics remain the programmer-error semantics for native TS/Rust consumers.

**`ToolHandler.handle` error type (resolved):** the callback throws `FfiError` — the existing exported error enum is reused, adding no new error surface. uniffi 0.32 callback methods support `Result<T, E>` over an exported error enum, so the reuse is zero-surface, and a single error vocabulary lowers the error-mapping cost across the five bindings. Guardrails: a `Dial` throw from a handler is non-contract — the bridge routes it to the `INTERNAL_ERROR` containment branch — and handler documentation states that handlers should only throw `Rejected`. The residual risk — whether the vendored C#/Go fork binders fully support fielded error enums in callbacks — is covered by a callback-error positive control in the core suite plus per-language handler-error smokes in every binding channel.

**Threading and containment model:** every FFI method is a sync block-on-async call over the cdylib-owned tokio runtime (D12); concurrent calls from multiple foreign threads are allowed — each blocks its own calling thread while the shared runtime multiplexes the library futures (D6). Foreign callbacks (`Transport::recv`, `ToolHandler::handle`) bridge through `spawn_blocking` so a blocking foreign call never monopolizes an async worker (D12 precedent — the `ForeignCallbackTransport` bridge: `spawn_blocking` plus `JoinError → error` mapping). The `ToolHandler` bridge stores the callback via `Arc::from(box)` in the closure and runs each call as `runtime.spawn_blocking(move || handler.handle(json))`: `Ok(json)` parses with `serde_json` into a `Value` (malformed → containment branch); `Err(Rejected)` constructs a `SpokeReject` (kind / wire_code re-hung onto `details` — the inverse of `map_spoke_reject`) and passes through; `Err(Dial)` (non-contract) and `JoinError` (foreign-crash signal) map to the `INTERNAL_ERROR` reject with `details: None`. The library serve loop's `catch_unwind` at the future-poll boundary remains the last line of defense — two containment layers with the same semantics, so the serve loop never dies from a foreign crash; the bridge treats every non-`Rejected` `Err` as containment regardless of uniffi's internal exception handling.

**Error rows (D7 extension by reference — `FfiError` shape unchanged):** `ConnectResponderFFI.invoke_tool` (the responder→dialer reverse invoke) follows the D15 table verbatim — grammar fail-fast, dispatch deny, deny codes, per-waiter timeout, session-closed, transport I/O, success-payload gate, correlation, panic containment. The D16-specific rows:

| Failure | `FfiError` row |
|---|---|
| `register_tool_handler` non-`tools.` grammar (either wrapper object) | `Rejected { code: "INVALID_INPUT", kind: None, wire_code: None }`, offending id in `message`, zero wire traffic — FFI-layer pre-validation via `parse_tool_capability_id`; the library grammar reject's `details.capability_id` is not surfaced separately (`map_spoke_reject` extracts only `kind` / `wire_code`) |
| Foreign `ToolHandler` returns `Err(Rejected{..})` | Application reject passes through verbatim — `code` / `message` preserved; `kind` / `wire_code` re-hung onto `details` (the inverse of `map_spoke_reject`) |
| Foreign `ToolHandler` returns `Err(Dial{..})` / a non-contract exception / panics / the bridge sees `JoinError` | `Rejected { code: "INTERNAL_ERROR", kind: None }` — containment, `details: None` mirroring the library `catch_unwind`; the serve loop survives |
| `connect_responder_ffi` config-validation failure (manifest JSON / seed length / peer-key length) | `Dial { kind: "config", message }` — the constructor `Result` slot's only inhabitant |
| Responder handshake failure (allowlist deny / hello-verify deny / peer never appears) | **No error row** — `state() → "Closed"`, `session_id() → None` (library semantics passthrough; the D14 divergence) |
| FFI responder (`ports` pinned `None`) receives a `port.*` invoke | Peer answers the documented deny branch → the caller gets `Rejected { code: "CAPABILITY_PORT_MISSING", kind: None, wire_code: Some("op_unsupported") }` |

**Accept topology (the responder FFI face's one new shape):** the FFI surface never builds a listener. The host product process owns listen/accept in its own language's network stack, wraps the connected socket as its callback `Transport` implementation, and passes that to `connect_responder_ffi` — symmetric with dial: `connect_remote_adapter_ffi` takes a connected outbound `Transport`; `connect_responder_ffi` takes a connected inbound `Transport`. Loopback proofs run over `loopback_transport_pair()`: one end dials through the dialer FFI face, the other serves through the responder FFI face, with invokes in both directions; tests and smokes poll `state()` to `Established` (the constructor does not wait for the handshake).

**Non-goals (this decision):**

- The `FfiError` shape is unchanged; no new error enums are added (including the handler error type — `FfiError` is reused).
- The session-core parity surface is untouched; the six envelope schemas are unchanged; no new wire codes.
- No host-callable verify/sign helpers are exported (the encapsulation hard rule stands).
- Responder-side `port.*` serving does not cross the FFI boundary: `ports` is pinned `None` and `port.*` invokes hit the library's documented absent-ports deny branch. A foreign-callback `BaselinePorts` face stays out of scope for this decision (a recorded capability gap).
- True-async FFI, listeners inside the FFI surface, and `port.computable.*` / `port.fork.*` FFI faces are out of scope.

## TS↔Rust parity

The TS and Rust RemoteAdapter/Transport are behaviorally aligned (loopback interop suites in both languages):

- **Public surface:** async `BaselinePorts` (six families) + dial (`connectRemoteAdapter` / `connect_remote_adapter`) + tool registry (`registerToolHandler` / `register_tool_handler`) + tool invoke face (`invokeTool` / `invoke_tool` on the adapter, the responder, and the router) + read-only session info (`sessionId`, `remotePeerId`, `remoteManifest`, `state`; Rust returns `Option` where TS returns `""` / throws) + `close` (TS sync, Rust sync; the Rust `Transport::close` itself is async, matching the trait).
- **Signature mapping:** TS `async`/`Promise` ↔ Rust `#[async_trait] async fn → SpokeResult` (Send futures). `orchestrateUpsert(remoteAdapter, req)` / `orchestrate_upsert(&remote_adapter, req)` compile against the same `BaselinePorts` surface.
- **Port-method → invoke mapping:** identical `port.*` catalogue, snake_case payloads, `ConnectInvokeRequest`/`ConnectInvokeResponse` reuse, no core-op reuse.
- **Encapsulation:** all connect verification (hello sign/verify, allowlist, nonce, sequence, correlation, dispatch awareness, capability-token attach, protocol_version 2 per-envelope auth sign/verify) is internal in both; consumers never call it.
- **Multi-peer parity:** identical `MultiPeerRouter` surface in both languages — registry (`registerPeer` / `unregisterPeer` / `listPeers`), selection inputs + algorithm (D11), composed/per-peer aggregation, and the `no_capable_peer` + as-is failure policy; dual-peer loopback routing proofs in both languages (disjoint manifests route `orchestrateUpsert` / `orchestrateCheck` to the capable peer, the deterministic tie-break resolves a third matching peer, and a no-match request returns `no_capable_peer`).
- **Tool serving (D13):** `registerToolHandler` / `register_tool_handler` — grammar-asserted (TS throws / Rust panics on a non-`tools.` id), last-wins overwrite, registry never mutates the manifest; handler containment (TS `toErrorEnvelope` / Rust `catch_unwind` at the future-poll boundary) with the application `SpokeReject` passing through verbatim; deny-code matrix identical (`op_unsupported` gate-fail / no-handler, `auth_failed`, `invalid_sequence`); signed responses with correlation echo. Handler type parity: TS `(args: Record<string, unknown>) => Promise<SpokeResult<unknown>>` (parameter named `args` — `arguments` is rejected by TypeScript strict mode; positional contract unchanged) ↔ Rust `Arc<dyn Fn(Value) -> BoxFuture<'static, SpokeResult<Value>> + Send + Sync>`.
- **Tool invoke face (D13/D14):** `invokeTool` / `invoke_tool` on `RemoteAdapter`, `ConnectResponder`, and `MultiPeerRouter` — grammar fail-fast (`INVALID_INPUT` with `details.capability_id`, no wire traffic), `#invokeOp` / `invoke_op` wire-order serialization, deferred-send poison-close, per-waiter timeout (waiter-only), D7 deny mapping (`CAPABILITY_PORT_MISSING` + `details.wire_code`), `{ result }` success-payload gate (`INTERNAL_ERROR` `details.kind = "transport"` on malformed success).
- **Responder (D14):** `connectResponder` / `connect_responder` — same options shape, allowlist-first handshake recipe, `port.*` serving via the D4 catalogue, reverse tool face, session info + `close`; **intentional divergence** — TS returns `Promise<ConnectResponder>` (instance), Rust returns `Arc<ConnectResponder>` with **no `Result`** (the TS seed-length throw is type-enforced via `[u8; 32]`, so the error slot is uninhabitable); handshake failures surface via `state → Closed` + transport close in both.
- **Router tool face (D14):** `RoutedRemoteAdapter.invokeTool` / `invoke_tool` + `MultiPeerRouter.invokeTool` / `invoke_tool` — exact-capability hard filter over the cached hello manifest, lowest-`peer_id` UTF-8 byte-order tie-break, terminal `no_capable_peer` reject (`CAPABILITY_PORT_MISSING` + `details.op = capability_id`), selected-peer reject as-is; composed manifest `tools[]` union (dedup by `capability_id`, lexicographic order for stability).
- **Dispatch prefix rule + guard hardening (D13):** `requiredCapability` / `required_capability` return the op string itself for the `tools.` prefix (Rust widens `Option<&'static str>` → `Option<&str>`, source-compatible — `ffi.rs` untouched); parity golden vector in both languages (authorized / not-negotiated / token-membership); TS `isConnectInvokeResponse` and Rust `wire_response_correlation` reject `op`-bearing documents (request-first classification).
- **Deviations (both languages, by design):** `remotePubkey` in dial options (hello verification requires the remote public key; the derived peer_id is checked against the allowlist — key acquisition is transport-adapter-owned); `close()` and `state` on the public surface (lifecycle resource release + read-only session info, not verification helpers).
- **Surface non-widening:** RemoteAdapter / responder / router surfaces sit strictly above session-core and are D13/D14 parity-table additions, not session-core; the session-core parity table is touched exactly once — the `tools.` dispatch-gate prefix rule (TS + Rust in lockstep, golden vectors in both languages). Transports remain intentionally asymmetric and outside parity.

## Non-goals

- WebSocket (or libp2p) implementation inside the Transport interface module beyond loopback tests  
- New connect or ops JSON Schema files  
- Silent sync compatibility shims on operations  

## Staged follow-ons

1. Optional computable/fork port ops (`port.computable.*`, `port.fork.*`) when product needs remote optional capabilities
2. Consumer-side WebSocket Transport packages
3. Reverse-invoke FFI surface — the `invokeTool` / `invoke_tool` reverse tool-invoke face over FFI for native binding hosts (deferred from the tool wire + library surfaces: no `ffi.rs` edits, no bindings regen); trigger: the TS + Rust tool-invoke library surfaces validated by consumers
4. Responder / tool-serving native surfaces — `connectResponder` / `connect_responder` and `registerToolHandler` / `register_tool_handler` over FFI for native hosts that serve `tools.*` ops (same trigger; the tool wire + library surfaces ship TS + Rust library surfaces only)

## Acceptance

- [x] Transport is message-oriented; framing delimiter ownership unchanged  
- [x] Port-method mapping reuses opaque invoke payload; no schema change  
- [x] Encapsulation boundary: public surface = `BaselinePorts` + dial/accept constructors + session info + `close` + tool registry & invoke faces (`registerToolHandler` / `register_tool_handler` + `invokeTool` / `invoke_tool`) + responder/router tool surfaces (D13/D14); all session/auth/sequence internals stay private  
- [x] Concurrent invoke + error mapping rules recorded  
- [x] Session-core parity surface non-widening recorded  
- [x] Loopback interop suites land with the connect remote module in TS (`./remote`) and Rust (`remote-adapter` feature); both green  
- [x] TS↔Rust parity statement recorded above (surface, signature mapping, deviations)  
- [x] Per-envelope auth (protocol_version 2) enforced on establish + invoke in both languages; enforcement recorded in D10  
- [x] Multi-peer capability router shipped (TS+Rust): registry + capability selection, HostManifest aggregation (composed view + per-peer array), failure policy (terminal `no_capable_peer` + selected-peer-down as-is), and public API (`connectMultiPeerRouter` / `registerPeer` / `unregisterPeer` / `listPeers`) recorded in D11  
- [x] Consumer docs describe language-native client + native binding wording (no internal path labels)
- [x] Synchronous `RemoteAdapterFFI` (D12): dial + signed-hello handshake through the foreign-callback `Transport`; the established handle exposes session info (`state` / `session_id` / `remote_peer_id` / `remote_manifest`) and the `BaselinePorts` six families as block-on-async calls over the cdylib-owned tokio runtime
- [x] Foreign-callback `Transport` interface: one envelope per `send` / `recv` call, blocking `recv` until an envelope arrives or the connection closes, idempotent `close`; `loopback_transport_pair()` / `LoopbackTransport` available over FFI
- [x] `FfiError` carries the D7 mapping: `Dial { kind, message }` for constructor / dial failures (`config` / `handshake` / `timeout` / `protocol_version_mismatch`), `Rejected { code, message, kind, wire_code }` for invoke-path `SpokeResult` rejects, and `TransportError` (`Closed` / `Io`) for callback transport failures
- [x] `MultiPeerRouterFFI` exposes the registry (`register_peer` / `unregister_peer` / `list_peers`), the `BaselinePorts` six families, and the composed/per-peer `HostManifestPort` views as synchronous calls over the same runtime
- [x] All five bindings (C#, Go, Kotlin, Python, Swift) generate the FFI surface and pass loopback smokes driving the callback `Transport` + `RemoteAdapterFFI` flow
- [x] D13: reverse invoke + RemoteAdapter tool serving — request-first classification (TS `isConnectInvokeResponse` / Rust `wire_response_correlation` reject `op`-bearing documents), frozen serving pipeline (stray → peek → verify → advance → gate → handler → signed response), deny-code matrix with no new wire codes (`op_unsupported` gate-fail / no-handler, `auth_failed`, `invalid_sequence`), handler registry (`registerToolHandler` / `register_tool_handler`, grammar-asserted, last-wins, never mutates the manifest) with throw/panic containment, forward `invokeTool` / `invoke_tool` face (wire-order serialization, poison-close, D7 deny mapping, `{ result }` success-payload gate)
- [x] D14: `connectResponder` / `connect_responder` — allowlist-first handshake recipe, `port.*` serving via the D4 catalogue (absent `ports` answers the dispatch-deny branch), reverse `invokeTool` / `invoke_tool`, public surface (session info + `close`), carried-over demo behaviors documented (empty-intersection snapshot fallback, unparseable-inbound closes the connection); intentional TS↔Rust divergence recorded (`Promise<ConnectResponder>` instance vs `Arc<ConnectResponder>` with no `Result`)
- [x] Router tool awareness: `MultiPeerRouter.invokeTool` / `invoke_tool` with exact-capability hard filter, lowest-`peer_id` tie-break, terminal `no_capable_peer` reject (`details.op = capability_id`), and composed-manifest `tools[]` union (dedup by `capability_id`, lexicographic order)
- [x] TS↔Rust parity rows for the serving / responder / router-tool surfaces recorded above; loopback bidirectional proofs in both languages (TS `reverse-invoke` / `responder` / `multi-peer-router` suites; Rust `remote_loopback` + `multi_peer_router` modules) with full suites green (D6 regression unmodified)
