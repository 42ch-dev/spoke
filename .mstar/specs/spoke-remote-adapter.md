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

**Aggregation:** the router implements `HostManifestPort` over the composed peer set with two views, both aggregated locally from the cached per-peer manifests. `getHostCapabilityManifest` returns a synthesized manifest carrying the router's own `host_id`, the set-union (deduplicated) of connected peers' `capabilities` / `roles` / `namespaces`, and `extensions.router.peers` listing the contributing `peer_id`s in lexicographic UTF-8 byte order; per-peer authority reads use the per-peer view. `listPeerHostCapabilityManifests` returns one entry per connected peer — each peer's own cached hello manifest — ordered by `peer_id` in lexicographic UTF-8 byte order; a router with zero registered peers returns `[]`. The composed view is an introspection surface and stays separate from selection, which reads each peer's own cached manifest.

**Failure policy:** when no registered peer passes the hard filters, the router rejects with the locked terminal reject — `SpokeResult` `CAPABILITY_PORT_MISSING` with `details.wire_code = "no_capable_peer"` and `details.kind = "no_capable_peer"`. The reject is deterministic for a given peer set and terminal for that request; the consumer registers a satisfying peer and re-invokes with a fresh `request_id`. When the selected peer's session fails mid-operation, the router returns the underlying `SpokeResult` reject exactly as the peer's adapter produced it (`INTERNAL_ERROR` with `details.kind` preserved, e.g. `transport` or `session_closed`), keeping the failure attributable to the peer that served the call; the router re-selects against the current peer set on the consumer's next invocation.

**Public surface:** `connectMultiPeerRouter(options)` / `connect_multi_peer_router(options)` plus `registerPeer` / `register_peer`, `unregisterPeer` / `unregister_peer`, `listPeers` / `list_peers`, and the async `BaselinePorts` six families. Per-peer adapters stay encapsulated after registration — the router holds only the adapter references the consumer dialed. The constructor carries no dial options; consumers dial each peer's `RemoteAdapter` and register the established adapter, and an unconfigured router uses the `multi-peer-router` host id.

### D12 — Native-binding FFI surface (delivered)

The RemoteAdapter contract ships over FFI as a synchronous surface on the `spoke-connect` cdylib (`ffi` + `remote-adapter` features), so native binding hosts consume the same adapter lifecycle as the Rust and TypeScript clients. The surface is generated into all five native binding channels (C#, Go, Kotlin, Python, Swift).

- **Cdylib-owned runtime:** the cdylib owns a process-wide multi-thread tokio runtime, initialized once. Every exported adapter call is a synchronous block-on-async call over that runtime; foreign callbacks run through the runtime's `spawn_blocking` pool, so a blocking `recv` never monopolizes an async worker.
- **`RemoteAdapterFFI` object:** `connect_remote_adapter_ffi(transport, local_seed, local_manifest_json, remote_pubkey, allowlist, invoke_timeout_ms)` dials through the binding's transport (signed-hello handshake, allowlist, session establishment) and returns an established adapter handle. The object exposes session info — `state` / `session_id` / `remote_peer_id` / `remote_manifest` — and the async `BaselinePorts` methods as synchronous calls. Payloads and manifests cross the boundary as JSON strings, keys as raw 32-byte arrays, peer ids as strings.
- **Foreign-callback `Transport` interface:** the binding implements the message-oriented `Transport` (`send` accepts exactly one envelope's bytes; `recv` blocks until one envelope arrives or the connection closes; `close` is idempotent resource release). `loopback_transport_pair()` / `LoopbackTransport` / `LoopbackTransportPair` expose the in-memory loopback over FFI so bindings exercise the surface without a network carrier.
- **`FfiError` (D7 mapping):** `Dial { kind, message }` for constructor / dial failures before an adapter exists (`config` / `handshake` / `timeout` / `protocol_version_mismatch` kinds); `Rejected { code, message, kind, wire_code }` for invoke-path `SpokeResult` rejects — application codes preserved, `INTERNAL_ERROR` rows carry `kind`, dispatch deny and unknown wire codes carry `wire_code`. The callback transport's own failures surface as `TransportError` (`Closed` / `Io`).
- **`MultiPeerRouterFFI`:** `new_multi_peer_router_ffi()` returns the router as a synchronous object — registry (`register_peer` / `unregister_peer` / `list_peers`), the async `BaselinePorts` six families, and the two `HostManifestPort` aggregation views, all block-on-async over the same runtime. `register_peer` accepts an established `RemoteAdapterFFI` handle and returns its `peer_id`.

## TS↔Rust parity

The TS and Rust RemoteAdapter/Transport are behaviorally aligned (loopback interop suites in both languages):

- **Public surface:** async `BaselinePorts` (six families) + dial (`connectRemoteAdapter` / `connect_remote_adapter`) + read-only session info (`sessionId`, `remotePeerId`, `remoteManifest`, `state`; Rust returns `Option` where TS returns `""` / throws) + `close` (TS sync, Rust sync; the Rust `Transport::close` itself is async, matching the trait).
- **Signature mapping:** TS `async`/`Promise` ↔ Rust `#[async_trait] async fn → SpokeResult` (Send futures). `orchestrateUpsert(remoteAdapter, req)` / `orchestrate_upsert(&remote_adapter, req)` compile against the same `BaselinePorts` surface.
- **Port-method → invoke mapping:** identical `port.*` catalogue, snake_case payloads, `ConnectInvokeRequest`/`ConnectInvokeResponse` reuse, no core-op reuse.
- **Encapsulation:** all connect verification (hello sign/verify, allowlist, nonce, sequence, correlation, dispatch awareness, capability-token attach, protocol_version 2 per-envelope auth sign/verify) is internal in both; consumers never call it.
- **Multi-peer parity:** identical `MultiPeerRouter` surface in both languages — registry (`registerPeer` / `unregisterPeer` / `listPeers`), selection inputs + algorithm (D11), composed/per-peer aggregation, and the `no_capable_peer` + as-is failure policy; dual-peer loopback routing proofs in both languages (disjoint manifests route `orchestrateUpsert` / `orchestrateCheck` to the capable peer, the deterministic tie-break resolves a third matching peer, and a no-match request returns `no_capable_peer`).
- **Deviations (both languages, by design):** `remotePubkey` in dial options (hello verification requires the remote public key; the derived peer_id is checked against the allowlist — key acquisition is transport-adapter-owned); `close()` and `state` on the public surface (lifecycle resource release + read-only session info, not verification helpers).
- **Surface non-widening:** session-core parity table unchanged; RemoteAdapter sits strictly above session-core; transports remain intentionally asymmetric and outside parity.

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
- [x] Encapsulation boundary: BaselinePorts + dial only on the public surface  
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
