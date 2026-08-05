# RemoteAdapter + message-oriented Transport

> **Owns:** Library-level decisions for a single-peer remote `BaselinePorts` adapter over connect, and the message-oriented Transport seam in spoke-connect.  
> **Status:** Implemented — normative surface in `@42ch/spoke-connect` (TS `./remote` subpath) and `spoke-connect` (Rust `remote-adapter` feature).  
> **Does not own:** Wire schema changes, multi-peer routing, native-binding FFI exposure, or in-repo WebSocket clients.

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

Session state is per peer so a later multi-peer registry can compose multiple RemoteAdapters without changing consumer orchestrator callsites.

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
| Correlation mismatch | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "correlation_mismatch"` |
| Sequence exhaustion | `SpokeResult` reject `INTERNAL_ERROR`, `details.kind = "sequence_exhausted"`; session closed |
| Dial / hello / allowlist / nonce failure | Constructor error — no adapter instance |

Port methods always settle to `SpokeResult`; typed transport errors stay on lower-level connect APIs.

### D8 — Async Rust ports use `async-trait` + Send futures

Operations port traits use the `async-trait` crate so existing dyn availability probes (`as_computable`, `as_fork_timeline`) stay object-safe. Port method futures are `Send`. The operations crate does not take a runtime dependency for the library itself. The Rust `Transport` trait and the `RemoteAdapter` port impls follow the same `async-trait` + Send convention.

### D9 — spoke-connect may depend on spoke-operations

spoke-connect depends on spoke-operations for port and result types used by RemoteAdapter — gated so default builds stay lean: the TS `./remote` subpath export (root bundle excludes RemoteAdapter symbols) and the Rust `remote-adapter` cargo feature (default `spoke-connect` builds do not require spoke-operations). Operations stays pure (no reverse dependency on connect).

## TS↔Rust parity

The TS and Rust RemoteAdapter/Transport are behaviorally aligned (loopback interop suites in both languages):

- **Public surface:** async `BaselinePorts` (six families) + dial (`connectRemoteAdapter` / `connect_remote_adapter`) + read-only session info (`sessionId`, `remotePeerId`, `remoteManifest`, `state`; Rust returns `Option` where TS returns `""` / throws) + `close` (TS sync, Rust sync; the Rust `Transport::close` itself is async, matching the trait).
- **Signature mapping:** TS `async`/`Promise` ↔ Rust `#[async_trait] async fn → SpokeResult` (Send futures). `orchestrateUpsert(remoteAdapter, req)` / `orchestrate_upsert(&remote_adapter, req)` compile against the same `BaselinePorts` surface.
- **Port-method → invoke mapping:** identical `port.*` catalogue, snake_case payloads, `ConnectInvokeRequest`/`ConnectInvokeResponse` reuse, no core-op reuse.
- **Encapsulation:** all connect verification (hello sign/verify, allowlist, nonce, sequence, correlation, dispatch awareness, capability-token attach) is internal in both; consumers never call it.
- **Deviations (both languages, by design):** `remotePubkey` in dial options (hello verification requires the remote public key; the derived peer_id is checked against the allowlist — key acquisition is transport-adapter-owned); `close()` and `state` on the public surface (lifecycle resource release + read-only session info, not verification helpers).
- **Surface non-widening:** session-core parity table unchanged; RemoteAdapter sits strictly above session-core; transports remain intentionally asymmetric and outside parity.

## Non-goals

- Multi-peer capability-based routing among N connected peers  
- Native-binding (FFI) exposure of RemoteAdapter  
- WebSocket (or libp2p) implementation inside the Transport interface module beyond loopback tests  
- New connect or ops JSON Schema files  
- Silent sync compatibility shims on operations  

## Staged follow-ons

1. **Multi-peer registry/composer over per-peer RemoteAdapter state** — *design-intent (not yet shipped)*. A multi-peer router (`connectMultiPeerRouter` / `connect_multi_peer_router`) sits above N per-peer `RemoteAdapter` instances, selects a peer per `BaselinePorts` call using `HostCapabilityManifest` fields (`capabilities` / `namespaces` / `roles` / `authority.scope_key`), and exposes the same async `BaselinePorts` surface so `orchestrate*(router, req)` works without per-op `peer_id`. Selection inputs, deterministic tie-break (lexicographic `peer_id`), no-match reject (`no_capable_peer`), `HostManifest` aggregation model (composed view for `getHostCapabilityManifest` + per-peer array for `listPeerHostCapabilityManifests`), and minimal failure/failover depth (no automatic alternate-retry) are architect-locked. Status here is **design-intent** — shipped-fact promotion lands in this ADR when the router ships. Envelope authentication is enforced on each per-peer session before the router selects.
2. Optional computable/fork port ops (`port.computable.*`, `port.fork.*`) when product needs remote optional capabilities  
3. FFI exposure after the TS/Rust contract stabilizes  
4. Consumer-side WebSocket Transport packages  

## Acceptance

- [x] Transport is message-oriented; framing delimiter ownership unchanged  
- [x] Port-method mapping reuses opaque invoke payload; no schema change  
- [x] Encapsulation boundary: BaselinePorts + dial only on the public surface  
- [x] Concurrent invoke + error mapping rules recorded  
- [x] Session-core parity surface non-widening recorded  
- [x] Loopback interop suites land with the connect remote module in TS (`./remote`) and Rust (`remote-adapter` feature); both green  
- [x] TS↔Rust parity statement recorded above (surface, signature mapping, deviations)  
- [x] Consumer docs describe language-native client + native bindings wording (no internal path labels)
