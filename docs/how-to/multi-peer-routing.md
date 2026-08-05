---
title: Route across multiple peers
---

# Route across multiple peers

A **multi-peer router** (`connectMultiPeerRouter` in TypeScript, `connect_multi_peer_router` in Rust) sits between your per-peer connect sessions and the `orchestrate*` calls. You register the peers your node has dialed, and every call — `orchestrateUpsert(router, req)` in TypeScript, `orchestrate_upsert(&router, req)` in Rust — is routed to exactly one registered peer: the request payload carries the operation only, and the router selects the peer. The `orchestrate*` entrypoints keep the same `BaselinePorts` surface as a single-peer adapter, so the router plugs into the same orchestration calls.

## 1. Dial and register peers

The router starts with zero peers. You dial each peer's `RemoteAdapter` yourself (signed-hello handshake, allowlist, session establishment), then register the established adapter. The router stores the adapter and caches the peer's `HostCapabilityManifest` — the `host` field from the authenticated hello — at registration time.

```ts
import { connectMultiPeerRouter } from "@42ch/spoke-connect/remote";

// The router starts with zero peers; register the adapters you dialed.
const router = connectMultiPeerRouter({ hostId: "storefront-router" });

const north = await connectRemoteAdapter({ /* dial options */ });
const south = await connectRemoteAdapter({ /* dial options */ });

const northId = router.registerPeer(north); // returns the remote peer_id
router.registerPeer(south);                 // idempotent on peer_id

router.listPeers();      // ["12D3KooW…", "12D3KooW…"] — registration order
router.unregisterPeer(northId); // removes it from selection; the adapter stays open
```

```rust
use spoke_connect::remote::{
    connect_multi_peer_router, connect_remote_adapter, MultiPeerRouterOptions,
};

let router = connect_multi_peer_router(MultiPeerRouterOptions {
    host_id: Some("storefront-router".into()),
});

let north = connect_remote_adapter(/* dial options */)?;
let south = connect_remote_adapter(/* dial options */)?;

let north_id = router.register_peer(north)?; // Ok(peer_id)
router.register_peer(south)?;                // idempotent on peer_id

router.list_peers();        // registration order
router.unregister_peer(&north_id); // selection drops it; the adapter stays open
```

| Operation | Behavior |
|-----------|----------|
| `registerPeer(adapter)` / `register_peer(&adapter)` | Accepts an established adapter, returns its `peer_id`, caches its manifest. Re-registering the same `peer_id` replaces the stored adapter (idempotent). The adapter must have an established session — dial first. |
| `unregisterPeer(peerId)` / `unregister_peer(&peer_id)` | Removes the peer from selection. Unregistering an unknown `peer_id` is a no-op. The adapter's lifecycle stays with the consumer — the router leaves the adapter open. |
| `listPeers()` / `list_peers()` | Registered `peer_id`s in registration order. |

A registered peer whose session transitions out of `Established` (for example to `Closed`, `Disconnected`, or `Handshaking`) is excluded from selection on the next call. The registry keeps the peer until you unregister it; exclusion is reactive, based on the session state the adapter reports.

## 2. Selection inputs

Selection reads each registered peer's cached `HostCapabilityManifest` and matches it against the operation. Four manifest fields drive the choice, split into hard filters and a soft preference:

| Input | Source | Filter kind | Role in selection |
|-------|--------|-------------|-------------------|
| `capabilities` | the peer's `capabilities[]` | **hard gate** | the peer must declare the operation's required capability |
| `namespaces` | the peer's `namespaces[]` | **hard gate** | the request's namespace (from the payload `Scope`) must match exactly |
| `roles` | the peer's `roles[]` | **soft preference** | peers with the operation's preferred role are preferred; capable peers without the role stay eligible |
| `authority.scope_key` | the peer's `authority.scope_key` | **hard gate when both sides declare** | exact match between the peer's scope key and the request's scope key |

Each operation family maps to a required capability:

| Operation family | Required capability |
|------------------|---------------------|
| `upsert`, `promote`, `relate`, `check`, `assemble` (and `port.*` baseline ops) | `spoke-baseline` |
| `project`, `compute` (and `port.computable.*`) | `l2-computable` |
| product-defined operations | the capability your product documents |

The request's namespace derives from the payload `Scope` when the operation carries one (for example `upsert-request.scope` or `check-request.scope`). Namespace matching is exact: a peer declaring `namespaces: ["*"]` declares the literal string `"*"`. When the request carries a scope key and the peer's manifest declares one, the two must match exactly; when only one side declares, that gate passes.

After the hard filters, peers with the operation's preferred role (for example `checker` for `check`, `assembler` for `assemble`) sort ahead of equally capable peers. When several peers survive, the router selects the lowest `peer_id` in lexicographic UTF-8 byte order — a pure function of the candidate set, so the same peers and the same request always select the same peer.

## 3. Failure outcomes

Every routing outcome is deterministic for a given peer set, and the router returns each outcome to the consumer as the call's result — all further activity starts with the consumer's next invocation.

### No peer matches the request

When no registered peer passes the hard filters, the router rejects with the locked terminal reject:

| Field | Value |
|-------|-------|
| `SpokeResult` reject code | `CAPABILITY_PORT_MISSING` |
| `details.wire_code` | `no_capable_peer` |
| `details.kind` | `no_capable_peer` |

The reject is terminal for that request: the router returns it and re-selects only when the consumer invokes again. Register a peer that satisfies the filters, then re-invoke with a fresh `request_id`.

### The selected peer fails mid-operation

When the selected peer's session fails during a delegated call, the router returns the underlying `SpokeResult` reject exactly as the peer's adapter produced it — `INTERNAL_ERROR` with `details.kind` preserved (for example `transport` or `session_closed`). The failure stays attributable to the peer that served the call.

Retry is consumer-owned. The consumer re-invokes with a fresh `request_id`; the router re-selects against the current peer set, and a peer whose session left `Established` drops out of the candidate set on the next selection. The consumer owns retry because the consumer knows each operation's idempotency semantics: a call may have been applied on the peer before the transport failed, and the consumer decides whether re-running the operation is safe. The protocol layer provides session-level correlation — `request_id`, `session_id`, and sequence — and per-operation idempotency decisions live with the consumer.

## 4. Envelope authenticity is protocol-level

Every connect envelope — hello, invoke, and response — is signed with the session peer's Ed25519 key over the RFC 8785 JCS-canonicalized object and verified inside the session core before dispatch. This verification runs on any ordered, reliable transport the adapter provides (TCP, WebSocket, yamux, or Noise), because authenticity lives in the envelope itself, above the transport. The router selects a peer after envelope authentication completes on each per-peer session; selection is a routing decision that operates within the session core's authenticated envelope flow.

## 5. Inspect what the router can reach

The router exposes the `HostManifestPort` over the composed peer set, with two distinct aggregation views:

```ts
// Composed view — the union of every connected peer's manifest:
const composed = await router.getHostCapabilityManifest();
// composed.value.host_id                  → the router's own host_id
// composed.value.capabilities             → set-union, deduplicated
// composed.value.roles                    → set-union, deduplicated
// composed.value.namespaces               → set-union, deduplicated
// composed.value.extensions.router.peers  → contributing peer_ids, UTF-8 byte order

// Per-peer view — each peer's own cached hello manifest:
const perPeer = await router.listPeerHostCapabilityManifests();
// perPeer.value[i].host_id → one peer's manifest per entry, sorted by peer_id
```

```rust
let composed = router.get_host_capability_manifest().await?;
// composed.host_id                  → the router's own host_id
// composed.capabilities             → set-union, deduplicated
// composed.roles                    → set-union, deduplicated
// composed.namespaces               → set-union, deduplicated
// composed.extensions["router"]["peers"] → contributing peer_ids, UTF-8 byte order

let per_peer = router.list_peer_host_capability_manifests().await?;
// per_peer[i].host_id → one peer's manifest per entry, sorted by peer_id
```

| View | Shape | Use |
|------|-------|-----|
| `getHostCapabilityManifest` | One synthesized manifest: the router's own `host_id`, set-union of `capabilities` / `roles` / `namespaces` across connected peers, and `extensions.router.peers` listing contributing `peer_id`s in lexicographic UTF-8 byte order. The composed view surfaces capability, role, and namespace unions; consumers needing a peer's `authority.scope_key` read the per-peer view. | Introspection: "what can this node reach?" |
| `listPeerHostCapabilityManifests` | One manifest per connected peer (each peer's own cached hello manifest), ordered by `peer_id` in lexicographic UTF-8 byte order. A router with zero registered peers returns `[]`. | Per-peer authority and manifest detail |

Routing reads each peer's own cached manifest; the composed view is an introspection surface and stays separate from selection.

## Next steps

- [Implement an adapter](/how-to/implement-adapter) — the per-peer port surface the router delegates to.
- [Orchestrate operations](/how-to/orchestrate-ops) — the `orchestrate*` calls that route through the router.
- [Connect from the TypeScript client](/how-to/connect-ts-client) — dialing each peer's session.
- [Connect wire reference](/reference/connect) — envelope signing, identity binding, and session-core rules.
