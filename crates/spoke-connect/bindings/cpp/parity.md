# C ABI ⇄ production facade parity

The C channel is an export face over the existing Rust facade, not a second
implementation. This table records, member by member, where each production
facade member lands in `crates/spoke-connect/bindings/cpp/include/spoke_connect.h`.

- **Left column (source of truth):** the public production facade
  `crates/spoke-connect/src/ffi.rs` — cited by line — plus the two
  callback-trait families it exposes (`Transport`, `PortsHandler`,
  `ToolHandler`).
- **Right column:** the hand-written C header. The line-number citation lives
  with the left column; the header is the single contract for Tasks 5–8.
- **Executable proof:** `tooling/connect/cpp-symbol-check.mjs` compares the
  header's declaration set against the carrier library's exported
  `spoke_connect_*` symbols in both directions, and compiles/links a C probe
  that holds a typed function pointer to every declaration. Names, not
  signatures, are compared there; the layout/ownership assertions live in the
  carrier's Rust battery.

## Conclusion

**Every production facade member, callback and error variant has a C ABI
counterpart. The unmatched-row count is zero.**

Concretely: the 83 declarations in the header account for all 82 facade
members (8 core functions, 6 core object members, 22 `RemoteAdapterFFI`
members, 15 `MultiPeerRouterFFI` members, 8 `ConnectResponderFFI` members, the
3 `Transport` callbacks, the 13 `PortsHandler` callbacks, the 1 `ToolHandler`
callback, the 6 loopback helpers) and all 15 facade error variants. The
remaining declarations are C-owned mechanics with no facade counterpart: 11
handle-release functions (a Rust `Arc` has no explicit free), the 3 callback
table constructors (a facade callback trait crosses as a boxed trait object),
and 4 boundary-only declarations (an ABI-version reporter and the buffer /
error release primitives) that exist only because C has no Rust drop glue and
no status/out convention of its own. No facade capability is left without a C
entry point, and no C declaration is left without a facade member behind it.

Two capabilities deserve explicit mention because they are the ones the older
prose inventories missed:

- `RemoteAdapterFFI::register_tool_handler` (`ffi.rs:1157`) —
  dialer-side tool serving, so a pure-C dialer can answer a reverse invoke.
  Declared as `spoke_connect_remote_adapter_register_tool_handler`.
- `CoreError::ProtocolVersionMismatch` (`ffi.rs:1816`) — the eighth core
  error variant, appended after `TokenInvalid` so the earlier variants keep
  their ordinals. Declared as status `SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH`
  (107).

## Session core — free functions

| Production facade (`crates/spoke-connect/src/ffi.rs`) | C declaration (`include/spoke_connect.h`) |
|---|---|
| `derive_peer_id_from_ed25519_pubkey` (:1877) | `spoke_connect_derive_peer_id_from_ed25519_pubkey` |
| `sign_hello_ed25519` (:1889) | `spoke_connect_sign_hello_ed25519` |
| `verify_hello_ed25519` (:1918) | `spoke_connect_verify_hello_ed25519` |
| `is_allowlisted` (:1966) | `spoke_connect_is_allowlisted` |
| `check_response_correlation` (:2038) | `spoke_connect_check_response_correlation` |
| `dispatch_allowed` (:2072) | `spoke_connect_dispatch_allowed` |
| `required_capability` (:2080) | `spoke_connect_required_capability` |
| `protocol_version` (:2088) | `spoke_connect_protocol_version` |

## Session core — objects

Each object keeps its "one handle, one release" shape: the C carriage owns a
boxed `Arc` of the facade object, so every constructor has a matching
release declaration.

| Production facade (`ffi.rs`) | C declaration |
|---|---|
| `NonceStore::new` (:1944) | `spoke_connect_nonce_store_new` |
| `NonceStore::check_and_record` (:1954) | `spoke_connect_nonce_store_check_and_record` |
| — (C ownership: boxed handle release) | `spoke_connect_nonce_store_free` |
| `OutboundSequence::new` (:1983) | `spoke_connect_outbound_sequence_new` |
| `OutboundSequence::allocate` (:1992) | `spoke_connect_outbound_sequence_allocate` |
| — (C ownership: boxed handle release) | `spoke_connect_outbound_sequence_free` |
| `InboundSequence::new` (:2014) | `spoke_connect_inbound_sequence_new` |
| `InboundSequence::advance` (:2025) | `spoke_connect_inbound_sequence_advance` |
| — (C ownership: boxed handle release) | `spoke_connect_inbound_sequence_free` |

## Remote adapter

Constructor: `connect_remote_adapter_ffi` (`ffi.rs:1185`) →
`spoke_connect_remote_adapter_new` (transport borrowed and cloned, seed +
manifest + remote key + allowlist + optional invoke timeout).
Handle release: `spoke_connect_remote_adapter_free` (C ownership; a Rust `Arc`
has no explicit free).

| `RemoteAdapterFFI` method (`ffi.rs`) | C declaration |
|---|---|
| `state` (:997) | `spoke_connect_remote_adapter_state` |
| `session_id` (:1001) | `spoke_connect_remote_adapter_session_id` |
| `remote_peer_id` (:1005) | `spoke_connect_remote_adapter_remote_peer_id` |
| `remote_manifest` (:1009) | `spoke_connect_remote_adapter_remote_manifest` |
| `get_host_capability_manifest` (:1015) | `spoke_connect_remote_adapter_get_host_capability_manifest` |
| `get_knowledge_entry` (:1019) | `spoke_connect_remote_adapter_get_knowledge_entry` |
| `put_knowledge_entry` (:1023) | `spoke_connect_remote_adapter_put_knowledge_entry` |
| `get_relation` (:1035) | `spoke_connect_remote_adapter_get_relation` |
| `put_relation` (:1039) | `spoke_connect_remote_adapter_put_relation` |
| `list_knowledge_entries` (:1050) | `spoke_connect_remote_adapter_list_knowledge_entries` |
| `list_timeline_events` (:1055) | `spoke_connect_remote_adapter_list_timeline_events` |
| `put_findings` (:1060) | `spoke_connect_remote_adapter_put_findings` |
| `list_rules` (:1065) | `spoke_connect_remote_adapter_list_rules` |
| `list_peer_host_capability_manifests` (:1069) | `spoke_connect_remote_adapter_list_peer_host_capability_manifests` |
| `project` (:1082) | `spoke_connect_remote_adapter_project` |
| `compute` (:1090) | `spoke_connect_remote_adapter_compute` |
| `list_fork_timeline_events` (:1099) | `spoke_connect_remote_adapter_list_fork_timeline_events` |
| `extract` (:1116) | `spoke_connect_remote_adapter_extract` |
| `invoke_tool` (:1134) | `spoke_connect_remote_adapter_invoke_tool` |
| `register_tool_handler` (:1157) | `spoke_connect_remote_adapter_register_tool_handler` |
| `close` (:1171) | `spoke_connect_remote_adapter_close` |

## Multi-peer router

Constructor: `new_multi_peer_router_ffi` (`ffi.rs:1650`) →
`spoke_connect_multi_peer_router_new`. Handle release:
`spoke_connect_multi_peer_router_free` (releases the router's adapter
references; it never closes caller-owned adapters).

| `MultiPeerRouterFFI` method (`ffi.rs`) | C declaration |
|---|---|
| `register_peer` (:1550) | `spoke_connect_multi_peer_router_register_peer` |
| `unregister_peer` (:1556) | `spoke_connect_multi_peer_router_unregister_peer` |
| `list_peers` (:1560) | `spoke_connect_multi_peer_router_list_peers` |
| `get_host_capability_manifest` (:1564) | `spoke_connect_multi_peer_router_get_host_capability_manifest` |
| `get_knowledge_entry` (:1568) | `spoke_connect_multi_peer_router_get_knowledge_entry` |
| `put_knowledge_entry` (:1572) | `spoke_connect_multi_peer_router_put_knowledge_entry` |
| `get_relation` (:1584) | `spoke_connect_multi_peer_router_get_relation` |
| `put_relation` (:1588) | `spoke_connect_multi_peer_router_put_relation` |
| `list_knowledge_entries` (:1599) | `spoke_connect_multi_peer_router_list_knowledge_entries` |
| `list_timeline_events` (:1604) | `spoke_connect_multi_peer_router_list_timeline_events` |
| `put_findings` (:1609) | `spoke_connect_multi_peer_router_put_findings` |
| `list_rules` (:1614) | `spoke_connect_multi_peer_router_list_rules` |
| `list_peer_host_capability_manifests` (:1618) | `spoke_connect_multi_peer_router_list_peer_host_capability_manifests` |
| `invoke_tool` (:1637) | `spoke_connect_multi_peer_router_invoke_tool` |

The router has no `extract` member and none is invented: `extract` is a
per-peer adapter face (`spoke_remote-adapter.md` D4/D14 scope), reached by
listing peers and driving the corresponding adapter handle.

## Responder (serving side)

Constructor: `connect_responder_ffi` (`ffi.rs:1466`) →
`spoke_connect_responder_new` (transport borrowed and cloned, seed + manifest +
allowlist + peer-key table + optional ports handler + optional invoke timeout).
Handle release: `spoke_connect_responder_free`.

| `ConnectResponderFFI` method (`ffi.rs`) | C declaration |
|---|---|
| `state` (:1380) | `spoke_connect_responder_state` |
| `session_id` (:1384) | `spoke_connect_responder_session_id` |
| `remote_peer_id` (:1388) | `spoke_connect_responder_remote_peer_id` |
| `remote_manifest` (:1395) | `spoke_connect_responder_remote_manifest` |
| `register_tool_handler` (:1408) | `spoke_connect_responder_register_tool_handler` |
| `invoke_tool` (:1425) | `spoke_connect_responder_invoke_tool` |
| `close` (:1436) | `spoke_connect_responder_close` |

## Callback families

Each facade callback trait is mirrored by a C table with one pointer per trait
method plus the context destructor. Every pointer on a present table is
required, and a provider declines a single unsupported method by returning
`SPOKE_CONNECT_FFI_REJECTED` — the distinction between absent ports and an
explicitly refusing callback is preserved.

| Facade trait method (`ffi.rs`) | C callback slot |
|---|---|
| `Transport::send` (:143) | `SpokeConnectTransportTable.send` (`SpokeConnectTransportSendFn`) |
| `Transport::recv` (:146) | `SpokeConnectTransportTable.recv` (`SpokeConnectTransportRecvFn`) |
| `Transport::close` (:148) | `SpokeConnectTransportTable.close` (`SpokeConnectTransportCloseFn`) |
| context destruction | `SpokeConnectTransportTable.destroy` (`SpokeConnectTransportDestroyFn`) |
| `ToolHandler::handle` (:329) | `SpokeConnectToolHandlerTable.handle` (`SpokeConnectToolHandleFn`) |
| context destruction | `SpokeConnectToolHandlerTable.destroy` (`SpokeConnectCallbackDestroyFn`) |
| `PortsHandler::get_knowledge_entry` (:488) | `SpokeConnectPortsHandlerTable.get_knowledge_entry` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::put_knowledge_entry` (:489) | `SpokeConnectPortsHandlerTable.put_knowledge_entry` (`SpokeConnectPortsRevisionFn`) |
| `PortsHandler::get_relation` (:494) | `SpokeConnectPortsHandlerTable.get_relation` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::put_relation` (:495) | `SpokeConnectPortsHandlerTable.put_relation` (`SpokeConnectPortsRevisionFn`) |
| `PortsHandler::list_knowledge_entries` (:500) | `SpokeConnectPortsHandlerTable.list_knowledge_entries` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::list_timeline_events` (:501) | `SpokeConnectPortsHandlerTable.list_timeline_events` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::put_findings` (:502) | `SpokeConnectPortsHandlerTable.put_findings` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::list_rules` (:503) | `SpokeConnectPortsHandlerTable.list_rules` (`SpokeConnectPortsListRulesFn`) |
| `PortsHandler::list_peer_host_capability_manifests` (:504) | `SpokeConnectPortsHandlerTable.list_peer_host_capability_manifests` (`SpokeConnectPortsNoInputFn`) |
| `PortsHandler::project` (:505) | `SpokeConnectPortsHandlerTable.project` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::compute` (:506) | `SpokeConnectPortsHandlerTable.compute` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::list_fork_timeline_events` (:507) | `SpokeConnectPortsHandlerTable.list_fork_timeline_events` (`SpokeConnectPortsTextFn`) |
| `PortsHandler::extract` (:522) | `SpokeConnectPortsHandlerTable.extract` (`SpokeConnectPortsTextFn`) |
| context destruction | `SpokeConnectPortsHandlerTable.destroy` (`SpokeConnectCallbackDestroyFn`) |

Table construction and handle release:

| Facade seam (`ffi.rs`) | C declaration |
|---|---|
| callback `Transport` accepted by `connect_remote_adapter_ffi` / `connect_responder_ffi` (wrapped by `ForeignCallbackTransport`, :164-171) | `spoke_connect_transport_new` / `spoke_connect_transport_free` |
| callback `PortsHandler` accepted by `connect_responder_ffi` (wrapped by `into_remote_serve_ports`) | `spoke_connect_ports_handler_new` / `spoke_connect_ports_handler_free` |
| callback `ToolHandler` accepted by `register_tool_handler` on both faces (wrapped by `into_remote_handler`) | `spoke_connect_tool_handler_new` / `spoke_connect_tool_handler_free` |

The `extract` slot is part of the complete ports callback: a provider that
does not serve it returns a reject rather than leaving the pointer NULL, which
is why the slot is required and why "absent ports" (NULL handler at
construction) stays distinguishable.

## Loopback helpers

| Facade helper (`ffi.rs`) | C declaration |
|---|---|
| `loopback_transport_pair` (:275) | `spoke_connect_loopback_transport_pair_new` |
| `LoopbackTransportPair::client` (:263) | `spoke_connect_loopback_transport_pair_client` |
| `LoopbackTransportPair::server` (:268) | `spoke_connect_loopback_transport_pair_server` |
| — (C ownership: boxed handle release) | `spoke_connect_loopback_transport_pair_free` |
| `LoopbackTransport::send` (:224) | `spoke_connect_loopback_transport_send` |
| `LoopbackTransport::recv` (:230) | `spoke_connect_loopback_transport_recv` |
| `LoopbackTransport::close` (:235) | `spoke_connect_loopback_transport_close` |
| — (C ownership: boxed handle release) | `spoke_connect_loopback_transport_free` |

These are production helpers, not test infrastructure: the feature-gated
smoke-host exports (`start_loopback_smoke_host*`,
`LoopbackSmokeHost`, `ffi-smoke-host`) are intentionally not part of this
parity table and have no C declaration.

## Error mapping (variant level)

| Facade variant (`ffi.rs`) | C status | Error record fields |
|---|---|---|
| — (boundary) | `SPOKE_CONNECT_OK` (0) | none |
| — (boundary: NULL out pointer, NULL/non-zero span, non-UTF-8 text, key length, duplicate peer id, bad `present`) | `SPOKE_CONNECT_INVALID_ARGUMENT` (1) | none |
| — (boundary: contained panic) | `SPOKE_CONNECT_PANIC` (2) | `message` |
| `CoreError::InvalidHelloSignature` (:1787) | `SPOKE_CONNECT_INVALID_HELLO_SIGNATURE` (100) | `message` |
| `CoreError::NonceReplay` (:1790) | `SPOKE_CONNECT_NONCE_REPLAY` (101) | `message` |
| `CoreError::HandshakeFailed { reason }` (:1793) | `SPOKE_CONNECT_HANDSHAKE_FAILED` (102) | `message` = reason |
| `CoreError::InvalidNonce { message }` (:1796) | `SPOKE_CONNECT_INVALID_NONCE` (103) | `message` |
| `CoreError::Crypto { message }` (:1799) | `SPOKE_CONNECT_CRYPTO` (104) | `message` |
| `CoreError::Jcs { message }` (:1803) | `SPOKE_CONNECT_JCS` (105) | `message` |
| `CoreError::TokenInvalid { message }` (:1808) | `SPOKE_CONNECT_TOKEN_INVALID` (106) | `message` |
| `CoreError::ProtocolVersionMismatch { reason }` (:1816) | `SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH` (107) | `message` = reason |
| `CoreInvokeError::SequenceExhausted` (:1842) | `SPOKE_CONNECT_SEQUENCE_EXHAUSTED` (200) | `message` |
| `CoreInvokeError::InboundSequenceMismatch { expected, actual }` (:1846) | `SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH` (201) | `message`, `expected`, `actual` |
| `CoreInvokeError::CorrelationMismatch` (:1850) | `SPOKE_CONNECT_CORRELATION_MISMATCH` (202) | `message` |
| `FfiError::Dial { kind, message }` (:902) | `SPOKE_CONNECT_FFI_DIAL` (300) | `kind`, `message` |
| `FfiError::Rejected { code, message, kind, wire_code }` (:904) | `SPOKE_CONNECT_FFI_REJECTED` (301) | `code`, `message`, optional `kind` / `wire_code` |
| `TransportError::Closed` (:109) | `SPOKE_CONNECT_TRANSPORT_CLOSED` (400) | `message` |
| `TransportError::Io(String)` (:112) | `SPOKE_CONNECT_TRANSPORT_IO` (401) | `message` |

Variant count: 8 `CoreError` + 3 `CoreInvokeError` + 2 `FfiError` + 2
`TransportError` = 15 facade variants, all mapped; 0 unmapped.

`FfiError::Rejected` application reject codes are strings, not a second closed
C enum: D7 reject codes pass through unchanged. On the foreign-callback side
the same statuses carry foreign buffers instead of owned buffers, and the
accepted sets are fixed — transport callbacks may return 0 / 400 / 401;
ports and tool callbacks may return 0 / 301. Any other status, or a malformed
callback result, is contained through the facade's `INTERNAL_ERROR` path
exactly as the ports/tool bridge specifies.

## Boundary-only declarations

These four exist because the C boundary has no drop glue and no status/out
convention of its own; they have no facade counterpart and are therefore not
parity gaps.

| C declaration | Why it exists |
|---|---|
| `spoke_connect_abi_version` | Reports the boundary revision (1). The facade has no ABI-version concept — its nearest relative is the hello protocol version, which is a separate declaration. |
| `spoke_connect_buffer_free` | Releases an owned `SpokeConnectBuffer` (and an optional buffer's contained buffer). Rust drops owned results itself. |
| `spoke_connect_optional_buffer_free` | Releases an owned optional buffer and marks it absent. |
| `spoke_connect_error_free` | Releases the four owned textual fields of an error record. Rust returns errors by value. |

## Accounting

| Group | Facade members | C declarations |
|---|---|---|
| Core free functions | 8 | 8 |
| Core objects (3 constructors + 3 methods) | 6 | 9 (6 + 3 releases) |
| `RemoteAdapterFFI` (constructor + 21 methods) | 22 | 23 (22 + release) |
| `MultiPeerRouterFFI` (constructor + 14 methods) | 15 | 16 (15 + release) |
| `ConnectResponderFFI` (constructor + 7 methods) | 8 | 9 (8 + release) |
| `Transport` callbacks + table seam | 3 | 2 (`transport_new` / `transport_free`; the callbacks are table slots) |
| `PortsHandler` callbacks + table seam | 13 | 2 (`ports_handler_new` / `ports_handler_free`) |
| `ToolHandler` callback + table seam | 1 | 2 (`tool_handler_new` / `tool_handler_free`) |
| Loopback helpers | 6 | 8 (6 + pair/end releases) |
| Error enumerations | 15 variants | mapped to statuses 100–107 / 200–202 / 300 / 301 / 400 / 401 |
| Boundary-only | — | 4 |
| **Total** | **82 facade members + 15 error variants** | **83 declarations, 0 unmatched** |

## Re-running the gate

```sh
node tooling/connect/cpp-build.mjs --target aarch64-apple-darwin --toolchain nightly
node tooling/connect/cpp-symbol-check.mjs \
  --header crates/spoke-connect/bindings/cpp/include/spoke_connect.h \
  --library crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib
```

The second command re-derives the declaration set from this header and the
export set from the staged library, so a header edit that misses an export (or
an export that misses a declaration) fails the gate rather than landing.
