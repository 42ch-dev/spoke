# RemoteAdapter + message-oriented Transport (decision record)

> **Owns:** Library-level decisions for a single-peer remote `BaselinePorts` adapter over connect, and the message-oriented Transport seam in spoke-connect.  
> **Status:** Decided for implementation. Runtime packages land when the async operations surface and connect remote module ship; until then this document is the normative design lock.  
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

After operations ports are async-native, `RemoteAdapter` implements async `BaselinePorts` so consumers call `orchestrateUpsert(remoteAdapter, request)` (and other baseline orchestrators) without transport or verification callsites. Public surface = async `BaselinePorts` + dial/connect constructor (+ optional read-only session metadata).

### D3 — Message-oriented Transport seam

spoke-connect defines a **message-oriented** `Transport` (`send` / `recv` of one connect envelope’s bytes per call). This matches §Transport framing in [`spoke-connect.md`](spoke-connect.md) (one JSON document = one envelope). WebSocket and other product transports are **external**; spoke-connect ships a loopback/in-memory Transport for tests. Byte-stream carriers document length-prefix (or equivalent) delimiting as transport-adapter-owned.

### D4 — Port-method ops over open vocabulary (no schema change)

Each BaselinePorts method maps to a connect invoke with:

- `op` under the reserved product prefix `port.*` (open string; not the core ops list `upsert` / `check` / …);
- `payload` as existing `OpaqueJson` carrying method arguments (snake_case field names);
- success `payload` carrying the success value `T`;
- failure via existing `ConnectInvokeResponse` error branch + shared `ErrorEnvelope`.

Core ops vocabulary remains for full ops-family remote calls. RemoteAdapter BaselinePorts proxy uses **port-method** ops so local `orchestrate*` can run on the caller and perform remote I/O per port call. Required capability for baseline port ops: `spoke-baseline`, authorized through the existing product `op_capability_requirements` map on hosts.

Catalogue (baseline):

| Method (conceptual) | `op` |
|---------------------|------|
| get knowledge entry | `port.knowledge.get` |
| put knowledge entry | `port.knowledge.put` |
| get relation | `port.relation.get` |
| put relation | `port.relation.put` |
| list knowledge entries | `port.scope.list_knowledge_entries` |
| list timeline events | `port.scope.list_timeline_events` |
| put findings | `port.finding.put` |
| list rules | `port.rule.list` |
| list peer manifests | `port.host.list_peer_manifests` |

`getHostCapabilityManifest` on RemoteAdapter returns the remote peer’s manifest from the authenticated hello (session cache), not a mandatory extra invoke.

**No `schemas/` changes** — open `op` + opaque `payload` already carry this design.

### D5 — HostManifestPort on a single-peer RemoteAdapter

- `getHostCapabilityManifest` → remote peer’s `HostCapabilityManifest` (hello `host`, cached at session establish). Not the local client manifest.
- `listPeerHostCapabilityManifests` → proxy to the remote via `port.host.list_peer_manifests` (peers-only semantics enforced on the host; empty list valid).

Session state is per peer so a later multi-peer registry can compose multiple RemoteAdapters without changing consumer orchestrator callsites.

### D6 — Concurrent invokes

Concurrent port calls on one established session are allowed. Outbound `sequence` is allocated atomically at send time. Responses demultiplex on `request_id` (plus echo checks for `session_id` and `sequence`). Completions may arrive out of order. RemoteAdapter owns one receive loop over `Transport.recv`; BaselinePorts callers do not call `recv` directly. Invoke timeout, retry, and duplicate policy remain adapter-owned per connect hard boundaries; `request_id` is the correlation handle.

### D7 — Error mapping

| Class | Surface |
|-------|---------|
| Application / lifecycle rejects (including remote `SpokeRejectCode` on the error branch) | `SpokeResult` reject (preserve codes via existing envelope mapping) |
| Dispatch deny / unknown wire codes | `SpokeResult` reject (`CAPABILITY_PORT_MISSING` or `INVALID_INPUT` with wire detail) |
| Transport I/O, session closed, invoke timeout, correlation mismatch, sequence exhaustion | `SpokeResult` reject `INTERNAL_ERROR` with structured `details.kind` |
| Dial / hello / allowlist failure | Constructor failure (no adapter instance) |

Ports remain `SpokeResult`-only; typed transport errors stay on lower-level connect APIs.

### D8 — Async Rust ports use `async-trait` + Send futures

Operations port traits use the `async-trait` crate so existing dyn availability probes (`as_computable`, `as_fork_timeline`) stay object-safe. Port method futures are `Send`. The operations crate does not take a runtime dependency for the library itself.

### D9 — spoke-connect may depend on spoke-operations

spoke-connect gains a dependency on spoke-operations for port and result types used by RemoteAdapter. Operations stays pure (no reverse dependency on connect).

## Non-goals (this design lock)

- Multi-peer capability-based routing among N connected peers  
- Native-binding (FFI) exposure of RemoteAdapter  
- WebSocket (or libp2p) implementation inside the Transport interface module beyond loopback tests  
- New connect or ops JSON Schema files  
- Silent sync compatibility shims on operations  

## Staged follow-ons

1. Multi-peer registry/composer over per-peer RemoteAdapter state  
2. Optional computable/fork port ops (`port.computable.*`, `port.fork.*`) when product needs remote optional capabilities  
3. FFI exposure after the TS/Rust contract stabilizes  
4. Consumer-side WebSocket Transport packages  

## Acceptance (design)

- [x] Transport is message-oriented; framing delimiter ownership unchanged  
- [x] Port-method mapping reuses opaque invoke payload; no schema change  
- [x] Encapsulation boundary: BaselinePorts + dial only on the public surface  
- [x] Concurrent invoke + error mapping rules recorded  
- [x] Session-core parity surface non-widening recorded  
- [ ] Package exports and loopback tests land with the connect remote module  
- [ ] Consumer docs describe language-native client + native bindings wording (no internal path labels) when the surface ships  
