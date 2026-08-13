---
title: Use RemoteAdapter from a native binding
---

# Use RemoteAdapter from a native binding

**Native bindings** expose the remote Adapter contract as a synchronous FFI surface: your host code implements a message-oriented `Transport`; the adapter dials and exchanges envelopes through it, and you then call the same `BaselinePorts` methods the Rust reference and the TypeScript language-native client call. The shared library owns a process-wide tokio runtime; every exported call is a synchronous block-on-async call over that runtime, and the session core stays encapsulated on the Rust side — hello sign/verify, allowlist, nonce single-use, sequence, correlation, and envelope authentication all run inside the binding, while your host code supplies the `Transport` and invokes the port methods.

The exported objects are `RemoteAdapterFFI` (single peer) and `MultiPeerRouterFFI` (multi-peer routing). This page walks the full flow with the Python binding; the same surface exists in C#, Go, Kotlin, and Swift with language-idiomatic names (see the [symbol map](#symbol-map-across-the-bindings)). The generic RemoteAdapter contract — the message-oriented `Transport` seam, dial options, and error mapping shared with the TypeScript and Rust libraries — is in [RemoteAdapter over a Transport](/how-to/connect-remote-adapter); this page covers the FFI surface.

## 1. Implement the callback `Transport`

Your host code implements the message-oriented `Transport` interface:

| Method | Behavior |
|--------|----------|
| `send(envelope)` | Accepts exactly one connect envelope's bytes |
| `recv()` | Returns the next inbound envelope; blocks until one arrives or the connection closes |
| `close()` | Releases resources; idempotent |

The crate exports `loopback_transport_pair()`, an in-memory client/server pair, so you can run the whole flow with no network. The reference loopback smokes ship one `LoopbackTransport` end into a test-only smoke host and drive the adapter from the other end through a callback transport:

```python
import spoke_connect

class LoopbackCallbackTransport:
    """Foreign-callback transport delegating to the client end of a loopback pair."""

    def __init__(self, inner: spoke_connect.LoopbackTransport) -> None:
        self._inner = inner

    def send(self, envelope: bytes) -> None:
        self._inner.send(envelope)

    def recv(self) -> bytes:
        return self._inner.recv()

    def close(self) -> None:
        self._inner.close()
```

For a real deployment, implement the same three methods over your carrier — a socket, a WebSocket, or a message channel. The Transport delivers exactly one envelope per `send` / `recv` call; byte-stream carriers apply length-prefix (or equivalent) delimiting before handing envelopes to the adapter.

## 2. Dial and construct `RemoteAdapterFFI`

`connect_remote_adapter_ffi` performs the dial and signed-hello handshake through your transport and returns an established adapter handle:

```python
pair = spoke_connect.loopback_transport_pair()

adapter = spoke_connect.connect_remote_adapter_ffi(
    LoopbackCallbackTransport(pair.client()),
    seed_client,           # local Ed25519 identity seed, exactly 32 bytes
    client_manifest_json,  # local HostCapabilityManifest as JSON
    pubkey_host,           # remote peer's Ed25519 public key, exactly 32 bytes
    [peer_id_host],        # allowlist: the remote peer_id
    None,                  # invoke timeout in ms; None uses the default
)
```

| Argument | Meaning |
|----------|---------|
| `transport` | Your `Transport` implementation; the adapter sends and receives envelopes through it |
| `local_seed` | 32-byte Ed25519 seed for the local identity (raw bytes) |
| `local_manifest_json` | Your `HostCapabilityManifest` as a JSON string |
| `remote_pubkey` | The remote peer's 32-byte Ed25519 public key (raw bytes) |
| `allowlist` | Peer ids this adapter accepts; the remote `peer_id` must be listed |
| `invoke_timeout_ms` | Optional per-invoke timeout; on elapse only that call fails and the session stays usable |

The constructor fails before an adapter exists when configuration, the handshake, a version mismatch, or the dial timeout fails — the error is `FfiError.Dial` with a `kind` of `config`, `handshake`, `protocol_version_mismatch`, or `timeout`. Your transport's own failures map to `TransportError` (`Closed` when the connection closes, `Io` for I/O errors).

## 3. Invoke a port method

Port methods take JSON payloads and return JSON strings. A `put` / `get` round trip over the established session:

```python
entry_json = json.dumps({
    "schema_version": 1,
    "entry_id": entry_id,
    "entry_type": "character",
    "canonical_name": "Ada",
    "status": "provisional",
    "body": {"summary": "Upserted over the connect session"},
    "extensions": {},
})

put_json = adapter.put_knowledge_entry(entry_json, None)
get_json = adapter.get_knowledge_entry(entry_id)
```

Each `BaselinePorts` family maps to a method with the same JSON-in / JSON-out shape: `get_host_capability_manifest`, `get_relation` / `put_relation`, `list_knowledge_entries` / `list_timeline_events`, `put_findings`, `list_rules`, and `list_peer_host_capability_manifests`. Invoke-path failures surface as `FfiError.Rejected` with the `SpokeResult` code preserved, plus `kind` and `wire_code` where the mapping defines them (for example `INTERNAL_ERROR` with `kind = "transport"`, or `CAPABILITY_PORT_MISSING` with `wire_code = "no_capable_peer"`).

Concurrent calls on one adapter are allowed; responses demultiplex on `request_id` and may arrive out of order.

## 4. Read session info

The adapter exposes read-only session metadata:

```python
adapter.state()            # "Established", "Handshaking", "Closed", ...
adapter.session_id()       # the session id, once established
adapter.remote_peer_id()   # the authenticated remote peer_id
adapter.remote_manifest()  # the remote peer's HostCapabilityManifest as JSON
```

`session_id` / `remote_peer_id` / `remote_manifest` are populated once the session establishes; the session info comes from the authenticated hello and the session core, captured at establish time.

## 5. Route across multiple peers with `MultiPeerRouterFFI`

`new_multi_peer_router_ffi()` returns an empty router. Dial each peer's `RemoteAdapterFFI` (step 2), register the established handles, and every port call routes to exactly one capable peer:

```python
router = spoke_connect.new_multi_peer_router_ffi()

north_id = router.register_peer(north_adapter)  # returns the remote peer_id
router.register_peer(south_adapter)             # idempotent on peer_id

router.list_peers()       # registered peer_ids, registration order
router.unregister_peer(north_id)  # drops it from selection; the adapter stays open

result_json = router.get_knowledge_entry(entry_id)  # routed to a capable peer
```

Selection reads each registered peer's cached `HostCapabilityManifest` — hard gates on the operation's required capability and exact namespace, a soft role preference, and a deterministic lowest-`peer_id` tie-break. When no registered peer passes the hard gates, the call rejects with `CAPABILITY_PORT_MISSING` and `wire_code = "no_capable_peer"`; register a satisfying peer and re-invoke with a fresh `request_id`. The router also exposes the composed and per-peer `HostManifestPort` views (`get_host_capability_manifest` and `list_peer_host_capability_manifests`). The full selection contract is in [Route across multiple peers](/how-to/multi-peer-routing).

## 6. Errors

Every FFI call settles through the `FfiError` surface — dial failures before an adapter exists, invoke-path `SpokeResult` rejects, and the callback transport's own failures:

### `FfiError.Dial` — constructor / dial failures

`{ kind, message }`, returned when the dial fails before an adapter exists:

| `kind` | When |
|--------|------|
| `config` | Local seed or remote public key not exactly 32 bytes, invalid local manifest JSON, or the remote `peer_id` not on the allowlist (fail-closed) |
| `handshake` | Hello signature failure, nonce single-use violation, dial-binding assert, or `ConnectSession` snapshot verification failure (version mismatches surface as `protocol_version_mismatch`) |
| `protocol_version_mismatch` | A hello advertising a mixed or unknown `protocol_version` — the version gate is hello verification step 1, before signature verification, so the kind fires on any mismatched-version hello, valid signature or not |
| `timeout` | The dial deadline elapsed (bounded-wait handshake) |

### `FfiError.Rejected` — invoke-path `SpokeResult` rejects

`{ code, message, kind, wire_code }`, returned by port methods on the established surface:

| Row | Shape |
|-----|-------|
| Application rejects | `code` preserved verbatim (for example `KNOWLEDGE_ENTRY_NOT_FOUND`) |
| Payload JSON parse failure | `INVALID_INPUT`, no `kind` / `wire_code` |
| `INTERNAL_ERROR` rows | `kind` ∈ {`transport`, `session_closed`, `timeout`, `panic`, `correlation_mismatch`, `sequence_exhausted`, `envelope_auth_missing`, `envelope_auth_invalid`, `envelope_auth_session_unbound`} |
| Dispatch deny | `CAPABILITY_PORT_MISSING` with `wire_code` = `op_unsupported` / `capability_missing` |
| Unknown wire codes | `INVALID_INPUT` with `wire_code` |
| Router terminal reject | `CAPABILITY_PORT_MISSING` with `wire_code` = `kind` = `no_capable_peer` |

`kind = "panic"` is the panic-containment row: a panic caught around an exported block-on-async call fails only that waiter and never unwinds across the FFI boundary — the message carries the raw panic payload. A foreign-callback panic inside the `spawn_blocking` pool surfaces as a transport `Io` failure instead.

### `TransportError` — callback transport failures

| Variant | When |
|---------|------|
| `Closed` | The connection closed; a pending `recv` fails fast |
| `Io` | Transport-level I/O failure, including a foreign-callback panic failing the blocking join |

## Symbol map across the bindings

| Surface | C# | Go | Kotlin | Python | Swift |
|---------|----|----|--------|--------|-------|
| Dial + construct | `ConnectRemoteAdapterFfi(...)` | `ConnectRemoteAdapterFfi(...)` | `connectRemoteAdapterFfi(...)` | `connect_remote_adapter_ffi(...)` | `connectRemoteAdapterFfi(...)` |
| Adapter object | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` |
| Router constructor | `NewMultiPeerRouterFfi()` | `NewMultiPeerRouterFfi()` | `newMultiPeerRouterFfi()` | `new_multi_peer_router_ffi()` | `newMultiPeerRouterFfi()` |
| Router object | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` |
| Port methods | PascalCase (`GetKnowledgeEntry`) | PascalCase (`GetKnowledgeEntry`) | camelCase (`getKnowledgeEntry`) | snake_case (`get_knowledge_entry`) | camelCase (`getKnowledgeEntry`) |

Every binding ships the same surface: golden-parity smokes assert byte-identical session-core behavior, and loopback smokes drive the callback `Transport` + `RemoteAdapterFFI` flow from each host side.

## Next steps

- [Route across multiple peers](/how-to/multi-peer-routing) — the selection contract behind `MultiPeerRouterFFI`.
- [Connect from native bindings](/how-to/connect-native-bindings) — install and authenticate each binding channel.
- [Connect from the TypeScript client](/how-to/connect-ts-client) — the language-native client that shares the same session-core rules.
- [Connect wire reference](/reference/connect) — envelope signing, identity binding, and session-core rules.
