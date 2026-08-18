---
title: RemoteAdapter over a Transport
---

# RemoteAdapter over a Transport

A **RemoteAdapter** turns a remote SPOKE connect peer into a drop-in async `BaselinePorts` surface: you supply a message-oriented `Transport`, the adapter dials and completes the signed-hello handshake through it, and you then call the same port methods — `getKnowledgeEntry`, `putRelation`, `listTimelineEvents`, and the rest — as if the peer were local. `orchestrateUpsert(adapter, req)` and the other `orchestrate*` calls run unchanged on the caller.

The adapter ships in two packages: the **TypeScript** `@42ch/spoke-connect/remote` subpath and the **Rust** `remote-adapter` cargo feature of `spoke-connect`. Both enforce protocol version 2 envelope authentication internally (see [Envelope authentication](/explanation/connect#envelope-authentication) in the Connect architecture).

## 1. The `Transport` seam

A `Transport` is a consumer-implemented seam that carries connect envelopes between the adapter and the remote peer. It is **message-oriented**: one call moves exactly one connect envelope.

| Method | Contract |
|--------|----------|
| `send(envelope)` | Accepts exactly one connect envelope's bytes |
| `recv()` | Returns the next inbound envelope; blocks until one arrives or the connection closes |
| `close()` | Releases resources; idempotent |

One envelope per call — a byte-stream carrier applies length-prefix (or equivalent) delimiting before handing envelopes to the adapter. The packages ship an in-memory loopback pair for tests: `loopbackTransportPair()` (TypeScript) / `loopback_transport_pair()` (Rust) return the client and server ends of one connection, and closing either end fails the peer's pending `recv` exactly like a real connection drop. The loopback pair is **test-only**; WebSocket and other carriers are consumer-side implementations of the same three methods.

## 2. TypeScript — `@42ch/spoke-connect/remote`

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

`connectRemoteAdapter` dials through your transport — signed hello exchange, allowlist check, session snapshot verification — and resolves to an established adapter:

```ts
import { derivePeerIdFromEd25519Pubkey, getPublicKeyEd25519 } from "@42ch/spoke-connect";
import { connectRemoteAdapter } from "@42ch/spoke-connect/remote";

const adapter = await connectRemoteAdapter({
  transport,           // your Transport implementation
  localIdentity: { seed }, // 32-byte Ed25519 seed
  localManifest,       // your HostCapabilityManifest
  remotePubkey,        // the remote peer's 32-byte Ed25519 public key
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
  invokeTimeoutMs: 5000, // optional; bounds the handshake and each invoke
});
```

| Option | Meaning |
|--------|---------|
| `transport` | Your `Transport` implementation; the adapter sends and receives envelopes through it |
| `localIdentity.seed` | 32-byte Ed25519 seed for the local connect identity |
| `localManifest` | Your `HostCapabilityManifest`, advertised in the signed hello |
| `remotePubkey` | The remote peer's 32-byte Ed25519 public key; the remote `peer_id` is derived from it and must be on the allowlist (fail-closed) |
| `allowlist` | Peer ids this adapter accepts; the remote `peer_id` must be listed |
| `invokeTimeoutMs` | Optional per-invoke timeout; on elapse only that call fails and the session stays usable (default 5000) |

The established adapter exposes read-only session info — `state`, `sessionId`, `remotePeerId`, `remoteManifest` — and `close()`:

```ts
adapter.state;          // "Established"
adapter.sessionId;      // the remote-assigned session id
adapter.remotePeerId;   // the authenticated remote peer_id
adapter.remoteManifest; // the remote peer's HostCapabilityManifest

adapter.close();        // releases the session; idempotent
```

A dial failure — configuration error, handshake rejection, or dial timeout — rejects the `connectRemoteAdapter` promise; no adapter instance exists.

## 3. Rust — the `remote-adapter` feature

```bash
cargo add spoke-connect --features remote-adapter
cargo add async-trait
```

The `Transport` trait mirrors the TypeScript interface (`async fn` methods, `Send + Sync`; `close` defaults to a no-op):

```rust
use async_trait::async_trait;
use spoke_connect::remote::{Transport, TransportError};

#[async_trait]
impl Transport for MyTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        // deliver exactly one envelope's bytes to the peer
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        // return the next inbound envelope, or Err(TransportError::Closed)
        // when the connection closes
        Ok(Vec::new())
    }

    async fn close(&self) -> Result<(), TransportError> {
        // release resources; idempotent
        Ok(())
    }
}
```

`connect_remote_adapter` performs the dial and returns `Arc<RemoteAdapter>`:

```rust
use std::sync::Arc;
use spoke_connect::remote::{
    connect_remote_adapter, RemoteAdapterOptions, RemoteIdentity,
};

let adapter = connect_remote_adapter(RemoteAdapterOptions {
    transport: Arc::new(my_transport),
    local_identity: RemoteIdentity { seed: client_seed }, // 32-byte Ed25519 seed
    local_manifest: client_manifest,                      // your HostCapabilityManifest
    remote_pubkey: host_pubkey,                           // remote peer's 32-byte Ed25519 public key
    allowlist: vec![peer_id_host.into()],
    invoke_timeout_ms: None,  // None uses the default (5000 ms)
    capability_token: None,   // optional capability-token proof attached as `auth`
})?;
```

Session info comes back as `Option`s (populated once the session establishes); `close()` is synchronous:

```rust
adapter.state();           // RemoteAdapterState::Established
adapter.session_id();      // Option<String>
adapter.remote_peer_id();  // Option<String>
adapter.remote_manifest(); // Option<HostCapabilityManifest>

adapter.close();
```

## 4. Call the `BaselinePorts` methods

The adapter implements the same async `BaselinePorts` six families as a local adapter — knowledge entries, relations, scope queries, findings, rules, and the host manifest views — so `orchestrate*` calls run unchanged on the caller:

```ts
import { orchestrateUpsert } from "@42ch/spoke-operations";

const response = await orchestrateUpsert(adapter, upsertRequest);
```

```rust
use spoke_operations::{orchestrate_upsert, SpokeResult};

// SpokeResult is a plain Ok/Reject enum — there is no `?` support, so match
// it explicitly (the same pattern the crate's own tests use).
let response = match orchestrate_upsert(adapter.as_ref(), upsert_request).await {
    SpokeResult::Ok(response) => response,
    SpokeResult::Reject(reject) => return Err(reject), // your error path
};
```

You can also call the port methods directly:

```ts
const put = await adapter.putKnowledgeEntry(entry, null);
const got = await adapter.getKnowledgeEntry(entry.entry_id);
```

Each port method maps to a reserved `port.*` product op (`port.knowledge.put`, `port.relation.get`, …) carried as the invoke `op`; the mapping is internal to the adapter. See [Port-method ops](/reference/connect#port-method-ops-remoteadapter) in the wire reference.

### Optional port families

Beyond the baseline six families, the adapter ships the optional `l2-computable` (`project` / `compute`) and `l5-fork` (`listForkTimelineEvents`) faces. They are plain port methods on the same established session — the demo client drives all three round-trips over a real WebSocket (`examples/connect-demo/client/src/main.ts`):

```ts
    const projectedResult = requireOk(
      await adapter.project({
        session_id: COMPUTABLE_SESSION_ID,
        entry_id: COMPUTABLE_ENTRY_ID,
        state: { ...PROJECT_STATE },
      }),
    );

    const computedResult = requireOk(
      await adapter.compute({
        session_id: COMPUTABLE_SESSION_ID,
        entry_id: COMPUTABLE_ENTRY_ID,
        computable: { ...COMPUTE_DELTA },
        settle: true,
      }),
    );

    forkEvents = requireOk(
      await adapter.listForkTimelineEvents({
        scope_id: DEMO_SCOPE_ID,
        fork_id: DEMO_STORM_FORK_ID,
      }),
    );
```

The family must be **negotiated**: both manifests declare it in `capabilities[]`, so the session's `negotiated_capabilities` contains it. The demo gates its optional steps on its own manifest's declarations — the negotiated set is the intersection, so a server that did not declare a family denies loudly instead of being skipped. Denials map through the shared dispatch-deny row: wire `op_unsupported` / `capability_missing` → `CAPABILITY_PORT_MISSING` reject with `details.wire_code` preserved (section 5 below).

The Rust adapter exposes the same faces as `project` / `compute` / `list_fork_timeline_events`; over FFI the same methods live on `RemoteAdapterFFI` (per-language casing in the [symbol map](/how-to/remote-adapter-native-binding#symbol-map-across-the-bindings)). Serving the families on the responder side — the library `ports` option, the Rust `RemoteServePorts` seam, and the foreign-callback `PortsHandler` — is documented in [Optional port families](/reference/connect#optional-port-families).

## 5. Concurrency and errors

Concurrent port calls on one established session are allowed: outbound `sequence` is allocated at send time, responses demultiplex on `request_id`, and completions may arrive out of order. Each pending invoke carries an adapter-owned timeout; on elapse only that call fails and the session stays usable.

Port calls settle to `SpokeResult`; invoke-path failures surface as rejects:

| Failure | Surface |
|---------|---------|
| Transport I/O | `INTERNAL_ERROR` reject with `details.kind = "transport"` |
| Session closed / connection loss | `INTERNAL_ERROR` reject with `details.kind = "session_closed"`; the adapter transitions to `Closed` and every pending invoke fails |
| Invoke timeout | `INTERNAL_ERROR` reject with `details.kind = "timeout"` (that waiter only) |
| Correlation mismatch | `INTERNAL_ERROR` reject with `details.kind = "correlation_mismatch"` |
| Sequence exhaustion | `INTERNAL_ERROR` reject with `details.kind = "sequence_exhausted"`; the session closes — open a new one |
| Envelope-auth rejection | `INTERNAL_ERROR` reject with `details.kind` ∈ {`envelope_auth_missing`, `envelope_auth_invalid`, `envelope_auth_session_unbound`} (that waiter only; the session stays usable) |
| Dispatch deny (`op_unsupported` / `capability_missing` wire codes) | `CAPABILITY_PORT_MISSING` reject with `details.wire_code` |
| Unknown wire codes | `INVALID_INPUT` reject with `details.wire_code` |

Dial / hello / allowlist / nonce failures happen before an adapter exists: `connectRemoteAdapter` rejects (TypeScript) or returns `Err(RemoteAdapterError)` with `Config` / `Handshake` / `ProtocolVersionMismatch` / `Timeout` variants (Rust).

## 6. Envelope authentication

The adapter enforces **protocol version 2** per-envelope authentication internally on every post-hello envelope, with nothing to configure:

- the dial verifies the responder's signed `ConnectSession` snapshot against the hello identity before establishing;
- every outbound invoke request carries a `spoke-connect-invoke-request-jcs-v1` signature;
- every inbound response runs the correlation echo check first, then `spoke-connect-invoke-response-jcs-v1` verification.

Envelope authenticity is a protocol-level property above the transport — it does not depend on TLS or Noise. See [Envelope authentication](/explanation/connect#envelope-authentication) in the Connect architecture, and the [wire reference](/reference/connect#envelope-authentication-protocol-version-2) for the signed field sets.

## 7. Loopback smoke

The in-repo loopback pair gives you the whole flow with no network: the server end is served by the repository's test loopback host ([`tests/remote/loopback-host.ts`](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-connect-ts/tests/remote/loopback-host.ts) — test-only), the client end is dialed by `connectRemoteAdapter`:

```ts
import { derivePeerIdFromEd25519Pubkey, getPublicKeyEd25519 } from "@42ch/spoke-connect";
import { connectRemoteAdapter, loopbackTransportPair } from "@42ch/spoke-connect/remote";
// startLoopbackHost is an in-repo test fixture, NOT a package export —
// consumers write their own host (or copy this one from the linked file).
import { startLoopbackHost } from "<repo>/packages/spoke-connect-ts/tests/remote/loopback-host.ts";

const clientSeed = /* your 32-byte Ed25519 seed */;
const hostSeed = /* the remote peer's 32-byte Ed25519 seed */;

const pair = loopbackTransportPair();

// Server end (test-only): the repository's loopback host serves a local
// async BaselinePorts adapter over the server end of the pair.
const host = await startLoopbackHost({
  transport: pair.server,
  seed: hostSeed,
  clientPubkey: getPublicKeyEd25519(clientSeed),
  allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(clientSeed))],
  adapter: toyWorldAdapter,
  hostManifest,
});

// Client end — the shipped consumer surface.
const adapter = await connectRemoteAdapter({
  transport: pair.client,
  localIdentity: { seed: clientSeed },
  localManifest,
  remotePubkey: getPublicKeyEd25519(hostSeed),
  allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(hostSeed))],
});

const put = await adapter.putKnowledgeEntry(entry, null);
const got = await adapter.getKnowledgeEntry(entry.entry_id);

adapter.close();
host.close();
```

The same flow is the journey's terminal step: [RemoteAdapter from native bindings](/how-to/remote-adapter-native-binding) drives the identical handshake over FFI with a foreign-callback `Transport`.

## Next steps

- [Integrate a RemoteAdapter against a live host](/tutorials/integrate-remote-adapter) — the step-by-step learning path through the same contract against the demo mock inference host.
- [Expose and invoke remote tools](/how-to/connect-remote-tools) — advertise tools in the manifest, register handlers on the dial, and let the host discover and reverse-invoke them.
- [Route across multiple peers](/how-to/multi-peer-routing) — a multi-peer router composes N registered adapters behind the same `BaselinePorts` surface.
- [RemoteAdapter from native bindings](/how-to/remote-adapter-native-binding) — the same adapter lifecycle as a synchronous FFI surface from C#, Go, Kotlin, Python, or Swift.
- [Connect from the TypeScript client](/how-to/connect-ts-client) — the language-native client surface, including the `./remote` entry point.
- [Connect wire reference](/reference/connect) — envelope field tables, envelope authentication, and the port-method ops catalogue.
- [Connect architecture](/explanation/connect) — session lifecycle, envelope authentication, and capability routing.
