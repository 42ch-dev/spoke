# ADR: Remote adapter above session-core with a message-oriented transport

Status: implemented — one drop-in async `BaselinePorts` adapter per peer plus a capability router, over a one-envelope-per-call transport seam

Date: 2026-08-05

## Problem

Consumers needed to run SPOKE baseline orchestration against a remote peer. That left two unattractive paths: teach every callsite the connect handshake, allowlist, nonce, sequence, correlation, dispatch gate, and capability-token handling; or grow a second orchestration path for remote work that drifts from the local one. Carriers differ (WebSocket, libp2p, foreign callbacks over FFI), and a byte-stream seam would move framing into the adapter. Several peers must be reachable from the same orchestration call, and native binding hosts need the lifecycle without owning an async runtime.

## Decision

- `RemoteAdapter` sits **above** session-core and reuses it — allowlist, hello crypto, nonce, sequence, correlation, dispatch gate, capability token. It never widens the TS↔Rust session-core parity table, and transports stay outside that surface.
- The adapter implements the async `BaselinePorts` six families in TypeScript and Rust, so `orchestrateUpsert(remoteAdapter, request)` (and every other baseline orchestrator) runs unchanged. The public surface is async `BaselinePorts` + dial/connect constructor + read-only session metadata + `close`, later joined by the tool registry and tool-invoke faces.
- `Transport` is message-oriented: `send` and `recv` move exactly one connect envelope's bytes per call, `close` is optional and idempotent, and `recv` rejects on close. WebSocket and other product transports stay external; only an in-memory loopback pair ships for tests, and a byte-stream carrier documents its own length-prefix delimiting.
- Per-port-call work uses the reserved `port.*` op prefix with the existing opaque `payload`. Whole-operation delegation uses the core op string itself (`extract`) or a typed family prefix (`memory.*`), with no `port.*` catalogue entry, no `{ request }` / `{ result }` wrapper, and no schema change.
- A multi-peer router composes established per-peer adapters behind the same `BaselinePorts` surface. Selection is a pure function of the registered set: hard capability filter from the cached hello manifest, role soft preference, then lowest `peer_id` in UTF-8 byte order. No per-op `peer_id`; no capable peer is a terminal `no_capable_peer` reject.
- Refusals introduce no new wire codes. Dispatch denials answer existing codes and map at the dialer to `SpokeResult` reject `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved; port methods always settle to `SpokeResult`.
- Native bindings get a synchronous FFI surface over a cdylib-owned multi-thread runtime: adapter, router, responder, callback `Transport`, tool handler, and ports handler. Every exported call is a block-on-async call over that runtime, foreign callbacks run on its blocking pool, and panics are contained instead of unwinding across the boundary.

## Alternatives considered

**A dedicated remote client API** with per-operation methods and an explicit handshake — rejected because it moves transport and verification callsites into every consumer and creates a second orchestration path beside the local ports; implementing `BaselinePorts` keeps one orchestrator for local and remote work.

**A byte-stream `Transport` (`read` / `write`)** — rejected because the wire unit is one JSON document per envelope: a byte-oriented seam forces the adapter to own framing, which is precisely what carriers vary on. Carriers that stream bytes own their delimiting instead.

**Shipping a WebSocket (or libp2p) transport inside the library** — rejected as a product concern that would drag carrier dependencies and framing policy into an otherwise lean default build; only loopback ships, and consumer-side carrier packages are follow-ons.

**A per-op `peer_id` on the router** — rejected because it leaks peer topology into orchestrator callsites that never otherwise name a peer, and the fixed `BaselinePorts` surface has no slot for it; capability selection with a deterministic tie-break yields the same routing without a parameter.

**Adding transports, or transport-specific rules, to the TS↔Rust session-core parity table** — rejected because parity is about session-core behaviour; transports are intentionally asymmetric and verifying them there would force mirrored carrier stubs in both languages. The table is touched exactly once, for the `tools.` dispatch-gate prefix rule.

**New wire codes for remote-only refusals** (capability denies, timeouts, auth failures) — rejected because the connect error vocabulary is frozen and interoperating peers already recognise the existing codes; the dialer maps them locally and preserves `details.wire_code`, so no new `kind` and no new wire code is introduced.

**`port.extract` as a catalogue member with `{ request }` / `{ result }` wrappers** — rejected because `extract` delegates a whole operation whose serving host owns the loader and extractor: nothing from the in-process loader may cross the wire, so the request envelope itself is the payload and a wrapper would only invite smuggling host-local state.

**Returning `Result<Arc<ConnectResponder>>` in Rust to mirror the TypeScript promise-and-throw** — rejected because `RemoteIdentity.seed: [u8; 32]` makes the TypeScript seed-length throw type-unreachable, so the error slot would be uninhabited and would hide the one real failure mode (handshake), which surfaces through `state → Closed` in both languages.

**Exporting the pure validators (`validateManifestTools`, `validateToolArguments`) over FFI** — rejected because they are thin library conveniences outside the parity surface, while the wire path guarantees authentication, negotiation, and fail-closed gating rather than structural manifest validation; native hosts call them in their own language.

## Consequences

- One orchestration path: a local ports object and a remote adapter are interchangeable arguments, so baseline consumers do not branch on locality and the remote surface cannot drift into a second orchestration dialect.
- Session-core remains the single verified boundary. Every consumer-side verification, sequence, and correlation rule stays internal, and the surface above it is free to grow tool and responder faces without reopening the parity table.
- The message-oriented seam keeps the adapter framing-free; a carrier that loses a message must close so the peer's pending `recv` rejects, rather than half-delivering an envelope.
- Concurrency is bounded by session mechanics, not by call sites: the outbound sequence is allocated atomically at send time, one receive loop owns `recv`, pending waiters demultiplex on `request_id` with echo checks, and a waiter timeout fails only that waiter. Protocol v1 defines no retry, so recovery is a fresh `request_id` from the caller.
- Capability blame stays two-layered and legible: a dispatch deny arrives as connect-layer `CAPABILITY_PORT_MISSING` with `details.wire_code`, distinct from the operations-layer port-family absence that carries `details.capability`.
- Routing trusts the signed hello, not the registry: a peer's manifest is cached at registration from the authenticated hello, a peer that left `Established` drops out of selection, and an ownership-filter miss is the same terminal reject as a capability miss.
- The router surface stays deliberately fixed (six baseline families plus tool invoke). Optional families such as `project` / `compute` / `extract` ride the per-peer adapter, so consumers needing them on several peers keep the per-peer handles instead of getting a widened router.
- The FFI face adds a blocking bridge: foreign callbacks consume one blocking-pool thread each, a re-entrant call back into the FFI surface from inside a callback is forbidden, and a panicking callback is contained so the serve loop survives.
- Cost of the parallel surfaces: TypeScript and Rust must stay in behavioural lockstep, and loopback interop suites are the only parity evidence. Every surface addition is therefore a parity decision, not a local convenience.
- Serve-side timeouts are local and silent to the peer: an expiry answers exactly one signed `INTERNAL_ERROR` / `details.kind = "timeout"` response, keeps the session `Established` and the consumed inbound sequence, and discards the late provider completion — no retry, counter rollback, capability mutation, or forced close follows.
