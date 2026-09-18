# C ABI ⇄ production facade parity

The C channel is an export face over the existing Rust facade, and the C++
header-only layer is an ownership and error face over that C channel — neither
is a second implementation. This table records, member by member, where each
production facade member lands in `crates/spoke-connect/bindings/cpp/include/spoke_connect.h`
and which C++ counterpart in `include/spoke_connect.hpp` consumes it.

- **Left column (source of truth):** the public production facade
  `crates/spoke-connect/src/ffi.rs` — cited by line — plus the two
  callback-trait families it exposes (`Transport`, `PortsHandler`,
  `ToolHandler`).
- **Middle column:** the hand-written C header, the single C contract. The
  line-number citation lives with the left column.
- **Right column:** the C++17 convenience layer. A business member keeps its
  facade name — the `spoke_connect_` and object prefixes drop — and returns
  `Result<…>`; release and ownership become RAII. The layer calls only these
  declarations, so it adds no C export and no facade capability.
- **Executable proof:** `tooling/connect/cpp-symbol-check.mjs` compares the
  header's declaration set against the carrier library's exported
  `spoke_connect_*` symbols in both directions, compiles/links a C probe that
  holds a typed function pointer to every declaration, compiles a C++17 probe
  that includes both headers — twice, covering repeated inclusion — and
  instantiates the convenience layer's values, its full handle set and its
  three callback factories under the consumer's no-exception flags, and pins
  the record and callback-table layouts. The symbol pass compares names; the
  layout pass takes the size, alignment and field offsets of every
  `#[repr(C)]` mirror from the carrier itself and asserts this header's
  `sizeof` / `_Alignof` / `offsetof` against them, in both directions — a
  declared record or member with no carrier report fails, and a reported
  record or member that is not declared here fails. Ownership rules stay as the
  carrier's Rust battery and the C++ smoke describe them.

## Conclusion

**Every production facade member, callback and error variant has a C ABI
counterpart, and every C declaration has a C++ counterpart in
`include/spoke_connect.hpp`. The unmatched-row count is zero in both columns:
no production capability is left without a C++ surface, and the convenience
layer adds no C export.**

Concretely: the 83 declarations in the header account for all 82 facade
members (8 core functions, 6 core object members, 22 `RemoteAdapterFFI`
members, 15 `MultiPeerRouterFFI` members, 8 `ConnectResponderFFI` members, the
3 `Transport` callbacks, the 13 `PortsHandler` callbacks, the 1 `ToolHandler`
callback, the 6 loopback helpers) and all 15 facade error variants. Every one
of those declarations is consumed by name in the C++ column below: the eight
core functions keep their facade name, each of the 11 opaque handles becomes
one move-only class whose destructor performs that handle's release
declaration, the three callback-table constructors become `Transport::create` /
`PortsHandler::create` / `ToolHandler::create` over the `TransportCallbacks` /
`PortsCallbacks` / `ToolCallbacks` records, and the four boundary-only
primitives become `abi_version()`, `Buffer`'s destructor, and the two internal
records that own an optional buffer and an error record for one call. Each row
is marked as business API (a method returning `Result<…>`) or RAII mechanics (a
move-only class that releases through the matching carrier function) — a
destructor only frees, and only `RemoteAdapter::close` /
`ConnectResponder::close` / `LoopbackTransport::close` end a session.

The remaining declarations are C-owned mechanics rather than business API: 11
handle-release functions (a Rust `Arc` has no explicit free), the 3 callback
table constructors (a facade callback trait crosses as a boxed trait object),
and 4 boundary-only declarations (an ABI-version reporter and the buffer /
error release primitives) that exist only because C has no Rust drop glue and
no status/out convention of its own. Their C++ counterparts are likewise
mechanics rather than business API — a destructor, a `create` factory, or
`abi_version()` — so the classification covers the whole header: no facade
capability is left without a C entry point, and no C declaration is left
without a C++ counterpart.

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

| Production facade (`crates/spoke-connect/src/ffi.rs`) | C declaration (`include/spoke_connect.h`) | C++ counterpart (`include/spoke_connect.hpp`) |
|---|---|---|
| `derive_peer_id_from_ed25519_pubkey` (:1877) | `spoke_connect_derive_peer_id_from_ed25519_pubkey` | `derive_peer_id_from_ed25519_pubkey` |
| `sign_hello_ed25519` (:1889) | `spoke_connect_sign_hello_ed25519` | `sign_hello_ed25519` |
| `verify_hello_ed25519` (:1918) | `spoke_connect_verify_hello_ed25519` | `verify_hello_ed25519` |
| `is_allowlisted` (:1966) | `spoke_connect_is_allowlisted` | `is_allowlisted` |
| `check_response_correlation` (:2038) | `spoke_connect_check_response_correlation` | `check_response_correlation` |
| `dispatch_allowed` (:2072) | `spoke_connect_dispatch_allowed` | `dispatch_allowed` |
| `required_capability` (:2080) | `spoke_connect_required_capability` | `required_capability` |
| `protocol_version` (:2088) | `spoke_connect_protocol_version` | `protocol_version` |

Each is a free function in `spoke::connect` returning `Result<…>`, and each text
parameter is a `std::string_view` while key material stays a
`SpokeConnectSlice` built by `slice` / `bytes`.

## Session core — objects

Each object keeps its "one handle, one release" shape: the C carriage owns a
boxed `Arc` of the facade object, so every constructor has a matching release
declaration.

| Production facade (`ffi.rs`) | C declaration | C++ counterpart |
|---|---|---|
| `NonceStore::new` (:1944) | `spoke_connect_nonce_store_new` | `NonceStore::create` |
| `NonceStore::check_and_record` (:1954) | `spoke_connect_nonce_store_check_and_record` | `NonceStore::check_and_record` |
| — (C ownership: boxed handle release) | `spoke_connect_nonce_store_free` | `NonceStore`'s destructor and move-assignment (RAII) |
| `OutboundSequence::new` (:1983) | `spoke_connect_outbound_sequence_new` | `OutboundSequence::create` |
| `OutboundSequence::allocate` (:1992) | `spoke_connect_outbound_sequence_allocate` | `OutboundSequence::allocate` |
| — (C ownership: boxed handle release) | `spoke_connect_outbound_sequence_free` | `OutboundSequence`'s destructor and move-assignment (RAII) |
| `InboundSequence::new` (:2014) | `spoke_connect_inbound_sequence_new` | `InboundSequence::create` |
| `InboundSequence::advance` (:2025) | `spoke_connect_inbound_sequence_advance` | `InboundSequence::advance` |
| — (C ownership: boxed handle release) | `spoke_connect_inbound_sequence_free` | `InboundSequence`'s destructor and move-assignment (RAII) |

## Remote adapter

Constructor: `connect_remote_adapter_ffi` (`ffi.rs:1185`) →
`spoke_connect_remote_adapter_new` (transport borrowed and cloned, seed +
manifest + remote key + allowlist + optional invoke timeout).
Handle release: `spoke_connect_remote_adapter_free` (C ownership; a Rust `Arc`
has no explicit free).
C++ counterpart: `RemoteAdapter::connect` takes the same arguments — the
transport as a `Transport` handle — and the `RemoteAdapter` destructor performs
the release. `RemoteAdapter::close` is the explicit session end.

| `RemoteAdapterFFI` method (`ffi.rs`) | C declaration | C++ counterpart |
|---|---|---|
| `state` (:997) | `spoke_connect_remote_adapter_state` | `RemoteAdapter::state` |
| `session_id` (:1001) | `spoke_connect_remote_adapter_session_id` | `RemoteAdapter::session_id` |
| `remote_peer_id` (:1005) | `spoke_connect_remote_adapter_remote_peer_id` | `RemoteAdapter::remote_peer_id` |
| `remote_manifest` (:1009) | `spoke_connect_remote_adapter_remote_manifest` | `RemoteAdapter::remote_manifest` |
| `get_host_capability_manifest` (:1015) | `spoke_connect_remote_adapter_get_host_capability_manifest` | `RemoteAdapter::get_host_capability_manifest` |
| `get_knowledge_entry` (:1019) | `spoke_connect_remote_adapter_get_knowledge_entry` | `RemoteAdapter::get_knowledge_entry` |
| `put_knowledge_entry` (:1023) | `spoke_connect_remote_adapter_put_knowledge_entry` | `RemoteAdapter::put_knowledge_entry` |
| `get_relation` (:1035) | `spoke_connect_remote_adapter_get_relation` | `RemoteAdapter::get_relation` |
| `put_relation` (:1039) | `spoke_connect_remote_adapter_put_relation` | `RemoteAdapter::put_relation` |
| `list_knowledge_entries` (:1050) | `spoke_connect_remote_adapter_list_knowledge_entries` | `RemoteAdapter::list_knowledge_entries` |
| `list_timeline_events` (:1055) | `spoke_connect_remote_adapter_list_timeline_events` | `RemoteAdapter::list_timeline_events` |
| `put_findings` (:1060) | `spoke_connect_remote_adapter_put_findings` | `RemoteAdapter::put_findings` |
| `list_rules` (:1065) | `spoke_connect_remote_adapter_list_rules` | `RemoteAdapter::list_rules` |
| `list_peer_host_capability_manifests` (:1069) | `spoke_connect_remote_adapter_list_peer_host_capability_manifests` | `RemoteAdapter::list_peer_host_capability_manifests` |
| `project` (:1082) | `spoke_connect_remote_adapter_project` | `RemoteAdapter::project` |
| `compute` (:1090) | `spoke_connect_remote_adapter_compute` | `RemoteAdapter::compute` |
| `list_fork_timeline_events` (:1099) | `spoke_connect_remote_adapter_list_fork_timeline_events` | `RemoteAdapter::list_fork_timeline_events` |
| `extract` (:1116) | `spoke_connect_remote_adapter_extract` | `RemoteAdapter::extract` |
| `invoke_tool` (:1134) | `spoke_connect_remote_adapter_invoke_tool` | `RemoteAdapter::invoke_tool` |
| `register_tool_handler` (:1157) | `spoke_connect_remote_adapter_register_tool_handler` | `RemoteAdapter::register_tool_handler` |
| `close` (:1171) | `spoke_connect_remote_adapter_close` | `RemoteAdapter::close` |

## Multi-peer router

Constructor: `new_multi_peer_router_ffi` (`ffi.rs:1650`) →
`spoke_connect_multi_peer_router_new`. Handle release:
`spoke_connect_multi_peer_router_free` (releases the router's adapter
references; it never closes caller-owned adapters).
C++ counterpart: `MultiPeerRouter::create` and the `MultiPeerRouter`
destructor. Registration borrows and retains an adapter, so the destructor
releases the router's own references and leaves every caller-owned adapter
open — `MultiPeerRouter` has no `close` because the production C surface has
none.

| `MultiPeerRouterFFI` method (`ffi.rs`) | C declaration | C++ counterpart |
|---|---|---|
| `register_peer` (:1550) | `spoke_connect_multi_peer_router_register_peer` | `MultiPeerRouter::register_peer` |
| `unregister_peer` (:1556) | `spoke_connect_multi_peer_router_unregister_peer` | `MultiPeerRouter::unregister_peer` |
| `list_peers` (:1560) | `spoke_connect_multi_peer_router_list_peers` | `MultiPeerRouter::list_peers` |
| `get_host_capability_manifest` (:1564) | `spoke_connect_multi_peer_router_get_host_capability_manifest` | `MultiPeerRouter::get_host_capability_manifest` |
| `get_knowledge_entry` (:1568) | `spoke_connect_multi_peer_router_get_knowledge_entry` | `MultiPeerRouter::get_knowledge_entry` |
| `put_knowledge_entry` (:1572) | `spoke_connect_multi_peer_router_put_knowledge_entry` | `MultiPeerRouter::put_knowledge_entry` |
| `get_relation` (:1584) | `spoke_connect_multi_peer_router_get_relation` | `MultiPeerRouter::get_relation` |
| `put_relation` (:1588) | `spoke_connect_multi_peer_router_put_relation` | `MultiPeerRouter::put_relation` |
| `list_knowledge_entries` (:1599) | `spoke_connect_multi_peer_router_list_knowledge_entries` | `MultiPeerRouter::list_knowledge_entries` |
| `list_timeline_events` (:1604) | `spoke_connect_multi_peer_router_list_timeline_events` | `MultiPeerRouter::list_timeline_events` |
| `put_findings` (:1609) | `spoke_connect_multi_peer_router_put_findings` | `MultiPeerRouter::put_findings` |
| `list_rules` (:1614) | `spoke_connect_multi_peer_router_list_rules` | `MultiPeerRouter::list_rules` |
| `list_peer_host_capability_manifests` (:1618) | `spoke_connect_multi_peer_router_list_peer_host_capability_manifests` | `MultiPeerRouter::list_peer_host_capability_manifests` |
| `invoke_tool` (:1637) | `spoke_connect_multi_peer_router_invoke_tool` | `MultiPeerRouter::invoke_tool` |

The router has no `extract` member and none is invented: `extract` is a
per-peer adapter face (`spoke_remote-adapter.md` D4/D14 scope), reached by
listing peers and driving the corresponding adapter handle. The same holds for
`project`, `compute`, `list_fork_timeline_events` and the fork face — the
production C surface routes only the baseline ports and the tool invoke, so
`MultiPeerRouter` wraps only those and adds no router-only member.

## Responder (serving side)

Constructor: `connect_responder_ffi` (`ffi.rs:1466`) →
`spoke_connect_responder_new` (transport borrowed and cloned, seed + manifest +
allowlist + peer-key table + optional ports handler + optional invoke timeout).
Handle release: `spoke_connect_responder_free`.
C++ counterpart: `ConnectResponder::serve` — the ports provider is an optional
`PortsHandler` pointer, `nullptr` meaning the server serves no ports — and the
`ConnectResponder` destructor for the release. `ConnectResponder::close` is the
explicit session end.

| `ConnectResponderFFI` method (`ffi.rs`) | C declaration | C++ counterpart |
|---|---|---|
| `state` (:1380) | `spoke_connect_responder_state` | `ConnectResponder::state` |
| `session_id` (:1384) | `spoke_connect_responder_session_id` | `ConnectResponder::session_id` |
| `remote_peer_id` (:1388) | `spoke_connect_responder_remote_peer_id` | `ConnectResponder::remote_peer_id` |
| `remote_manifest` (:1395) | `spoke_connect_responder_remote_manifest` | `ConnectResponder::remote_manifest` |
| `register_tool_handler` (:1408) | `spoke_connect_responder_register_tool_handler` | `ConnectResponder::register_tool_handler` |
| `invoke_tool` (:1425) | `spoke_connect_responder_invoke_tool` | `ConnectResponder::invoke_tool` |
| `close` (:1436) | `spoke_connect_responder_close` | `ConnectResponder::close` |

## Callback families

Each facade callback trait is mirrored by a C table with one pointer per trait
method plus the context destructor. Every pointer on a present table is
required, and a provider declines a single unsupported method by returning
`SPOKE_CONNECT_FFI_REJECTED` — the distinction between absent ports and an
explicitly refusing callback is preserved.

| Facade trait method (`ffi.rs`) | C callback slot | C++ counterpart |
|---|---|---|
| `Transport::send` (:143) | `SpokeConnectTransportTable.send` (`SpokeConnectTransportSendFn`) | `TransportCallbacks::send` (`std::string_view` → `Result<void>`) |
| `Transport::recv` (:146) | `SpokeConnectTransportTable.recv` (`SpokeConnectTransportRecvFn`) | `TransportCallbacks::recv` (→ `Result<std::string>`) |
| `Transport::close` (:148) | `SpokeConnectTransportTable.close` (`SpokeConnectTransportCloseFn`) | `TransportCallbacks::close` (→ `Result<void>`) |
| context destruction | `SpokeConnectTransportTable.destroy` (`SpokeConnectTransportDestroyFn`) | the `TransportCallbacks` record is owned by the carrier and deleted by that `destroy` slot — no C++ member |
| `ToolHandler::handle` (:329) | `SpokeConnectToolHandlerTable.handle` (`SpokeConnectToolHandleFn`) | `ToolCallbacks::handle` (`std::string_view` → `Result<std::string>`) |
| context destruction | `SpokeConnectToolHandlerTable.destroy` (`SpokeConnectCallbackDestroyFn`) | the `ToolCallbacks` record is owned by the carrier and deleted by that `destroy` slot — no C++ member |
| `PortsHandler::get_knowledge_entry` (:488) | `SpokeConnectPortsHandlerTable.get_knowledge_entry` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::get_knowledge_entry` |
| `PortsHandler::put_knowledge_entry` (:489) | `SpokeConnectPortsHandlerTable.put_knowledge_entry` (`SpokeConnectPortsRevisionFn`) | `PortsCallbacks::put_knowledge_entry` (`std::optional<uint64_t>` revision) |
| `PortsHandler::get_relation` (:494) | `SpokeConnectPortsHandlerTable.get_relation` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::get_relation` |
| `PortsHandler::put_relation` (:495) | `SpokeConnectPortsHandlerTable.put_relation` (`SpokeConnectPortsRevisionFn`) | `PortsCallbacks::put_relation` |
| `PortsHandler::list_knowledge_entries` (:500) | `SpokeConnectPortsHandlerTable.list_knowledge_entries` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::list_knowledge_entries` |
| `PortsHandler::list_timeline_events` (:501) | `SpokeConnectPortsHandlerTable.list_timeline_events` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::list_timeline_events` |
| `PortsHandler::put_findings` (:502) | `SpokeConnectPortsHandlerTable.put_findings` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::put_findings` |
| `PortsHandler::list_rules` (:503) | `SpokeConnectPortsHandlerTable.list_rules` (`SpokeConnectPortsListRulesFn`) | `PortsCallbacks::list_rules` (borrowed `SpokeConnectSlice*` + count) |
| `PortsHandler::list_peer_host_capability_manifests` (:504) | `SpokeConnectPortsHandlerTable.list_peer_host_capability_manifests` (`SpokeConnectPortsNoInputFn`) | `PortsCallbacks::list_peer_host_capability_manifests` |
| `PortsHandler::project` (:505) | `SpokeConnectPortsHandlerTable.project` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::project` |
| `PortsHandler::compute` (:506) | `SpokeConnectPortsHandlerTable.compute` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::compute` |
| `PortsHandler::list_fork_timeline_events` (:507) | `SpokeConnectPortsHandlerTable.list_fork_timeline_events` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::list_fork_timeline_events` |
| `PortsHandler::extract` (:522) | `SpokeConnectPortsHandlerTable.extract` (`SpokeConnectPortsTextFn`) | `PortsCallbacks::extract` |
| context destruction | `SpokeConnectPortsHandlerTable.destroy` (`SpokeConnectCallbackDestroyFn`) | the `PortsCallbacks` record is owned by the carrier and deleted by that `destroy` slot — no C++ member |

Table construction and handle release:

| Facade seam (`ffi.rs`) | C declaration | C++ counterpart |
|---|---|---|
| callback `Transport` accepted by `connect_remote_adapter_ffi` / `connect_responder_ffi` (wrapped by `ForeignCallbackTransport`, :164-171) | `spoke_connect_transport_new` / `spoke_connect_transport_free` | `Transport::create(std::unique_ptr<TransportCallbacks>&)` — it renders the complete table and consumes the record only on success — plus `Transport`'s destructor |
| callback `PortsHandler` accepted by `connect_responder_ffi` (wrapped by `into_remote_serve_ports`) | `spoke_connect_ports_handler_new` / `spoke_connect_ports_handler_free` | `PortsHandler::create(std::unique_ptr<PortsCallbacks>&)` plus `PortsHandler`'s destructor |
| callback `ToolHandler` accepted by `register_tool_handler` on both faces (wrapped by `into_remote_handler`) | `spoke_connect_tool_handler_new` / `spoke_connect_tool_handler_free` | `ToolHandler::create(std::unique_ptr<ToolCallbacks>&)` plus `ToolHandler`'s destructor |

The `extract` slot is part of the complete ports callback: a provider that
does not serve it returns a reject rather than leaving the pointer NULL, which
is why the slot is required and why "absent ports" (NULL handler at
construction) stays distinguishable.

## Loopback helpers

| Facade helper (`ffi.rs`) | C declaration | C++ counterpart |
|---|---|---|
| `loopback_transport_pair` (:275) | `spoke_connect_loopback_transport_pair_new` | `LoopbackTransportPair::create` |
| `LoopbackTransportPair::client` (:263) | `spoke_connect_loopback_transport_pair_client` | `LoopbackTransportPair::client` |
| `LoopbackTransportPair::server` (:268) | `spoke_connect_loopback_transport_pair_server` | `LoopbackTransportPair::server` |
| — (C ownership: boxed handle release) | `spoke_connect_loopback_transport_pair_free` | `LoopbackTransportPair`'s destructor and move-assignment (RAII) |
| `LoopbackTransport::send` (:224) | `spoke_connect_loopback_transport_send` | `LoopbackTransport::send` |
| `LoopbackTransport::recv` (:230) | `spoke_connect_loopback_transport_recv` | `LoopbackTransport::recv` |
| `LoopbackTransport::close` (:235) | `spoke_connect_loopback_transport_close` | `LoopbackTransport::close` |
| — (C ownership: boxed handle release) | `spoke_connect_loopback_transport_free` | `LoopbackTransport`'s destructor and move-assignment (RAII) |

The ends stay independent helpers: an end is never presented as a callback
`Transport`, so a host that holds one drives it directly.

These are production helpers, not test infrastructure: the feature-gated
smoke-host exports (`start_loopback_smoke_host*`,
`LoopbackSmokeHost`, `ffi-smoke-host`) are intentionally not part of this
parity table and have no C declaration.

## Error mapping (variant level)

| Facade variant (`ffi.rs`) | C status | Error record fields | C++ counterpart |
|---|---|---|---|
| — (boundary) | `SPOKE_CONNECT_OK` (0) | none | `Result<T>::success(…)` |
| — (boundary: NULL out pointer, NULL/non-zero span, non-UTF-8 text, key length, duplicate peer id, bad `present`) | `SPOKE_CONNECT_INVALID_ARGUMENT` (1) | none | `Result<T>::failure(Error)` — `status` (also an incomplete callback record) |
| — (boundary: contained panic) | `SPOKE_CONNECT_PANIC` (2) | `message` | `Error::status` + `Error::message` |
| `CoreError::InvalidHelloSignature` (:1787) | `SPOKE_CONNECT_INVALID_HELLO_SIGNATURE` (100) | `message` | `Error::status` + `Error::message` |
| `CoreError::NonceReplay` (:1790) | `SPOKE_CONNECT_NONCE_REPLAY` (101) | `message` | `Error::status` + `Error::message` |
| `CoreError::HandshakeFailed { reason }` (:1793) | `SPOKE_CONNECT_HANDSHAKE_FAILED` (102) | `message` = reason | `Error::status` + `Error::message` |
| `CoreError::InvalidNonce { message }` (:1796) | `SPOKE_CONNECT_INVALID_NONCE` (103) | `message` | `Error::status` + `Error::message` |
| `CoreError::Crypto { message }` (:1799) | `SPOKE_CONNECT_CRYPTO` (104) | `message` | `Error::status` + `Error::message` |
| `CoreError::Jcs { message }` (:1803) | `SPOKE_CONNECT_JCS` (105) | `message` | `Error::status` + `Error::message` |
| `CoreError::TokenInvalid { message }` (:1808) | `SPOKE_CONNECT_TOKEN_INVALID` (106) | `message` | `Error::status` + `Error::message` |
| `CoreError::ProtocolVersionMismatch { reason }` (:1816) | `SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH` (107) | `message` = reason | `Error::status` + `Error::message` |
| `CoreInvokeError::SequenceExhausted` (:1842) | `SPOKE_CONNECT_SEQUENCE_EXHAUSTED` (200) | `message` | `Error::status` + `Error::message` |
| `CoreInvokeError::InboundSequenceMismatch { expected, actual }` (:1846) | `SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH` (201) | `message`, `expected`, `actual` | `Error::status`, `message`, `expected`, `actual` |
| `CoreInvokeError::CorrelationMismatch` (:1850) | `SPOKE_CONNECT_CORRELATION_MISMATCH` (202) | `message` | `Error::status` + `Error::message` |
| `FfiError::Dial { kind, message }` (:902) | `SPOKE_CONNECT_FFI_DIAL` (300) | `kind`, `message` | `Error::status`, `kind`, `message` |
| `FfiError::Rejected { code, message, kind, wire_code }` (:904) | `SPOKE_CONNECT_FFI_REJECTED` (301) | `code`, `message`, optional `kind` / `wire_code` | `Error::status`, `code`, `message`, optional `kind` / `wire_code` |
| `TransportError::Closed` (:109) | `SPOKE_CONNECT_TRANSPORT_CLOSED` (400) | `message` | `Error::status` + `Error::message` |
| `TransportError::Io(String)` (:112) | `SPOKE_CONNECT_TRANSPORT_IO` (401) | `message` | `Error::status` + `Error::message` |

Variant count: 8 `CoreError` + 3 `CoreInvokeError` + 2 `FfiError` + 2
`TransportError` = 15 facade variants, all mapped; 0 unmapped.

The C++ counterpart is one struct, not a second enumeration: `Error::status`
carries the existing `SPOKE_CONNECT_*` constant and the remaining members carry
the same record fields, with pointer presence deciding whether `code` / `kind` /
`wire_code` are present — a present empty string stays present, an absent field
stays `nullopt`.

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

| C declaration | Why it exists | C++ counterpart |
|---|---|---|
| `spoke_connect_abi_version` | Reports the boundary revision (1). The facade has no ABI-version concept — its nearest relative is the hello protocol version, which is a separate declaration. | `abi_version()` |
| `spoke_connect_buffer_free` | Releases an owned `SpokeConnectBuffer` (and an optional buffer's contained buffer). Rust drops owned results itself. | `Buffer`'s destructor and move-assignment |
| `spoke_connect_optional_buffer_free` | Releases an owned optional buffer and marks it absent. | `detail::OptionalBufferRecord`'s destructor — the optional-buffer out-record of one call |
| `spoke_connect_error_free` | Releases the four owned textual fields of an error record. Rust returns errors by value. | `detail::CErrorRecord`'s destructor — it guards the copy into `Error` so a failing copy cannot leak the record |

## Accounting

| Group | Facade members | C declarations | C++ counterparts |
|---|---|---|---|
| Core free functions | 8 | 8 | 8 free functions — business API |
| Core objects (3 constructors + 3 methods) | 6 | 9 (6 + 3 releases) | `NonceStore` / `OutboundSequence` / `InboundSequence`: `create` + one method each (business API) + 3 destructors (RAII) |
| `RemoteAdapterFFI` (constructor + 21 methods) | 22 | 23 (22 + release) | `RemoteAdapter`: `connect` + 21 methods (business API) + its destructor (RAII) |
| `MultiPeerRouterFFI` (constructor + 14 methods) | 15 | 16 (15 + release) | `MultiPeerRouter`: `create` + 14 methods (business API) + its destructor (RAII) |
| `ConnectResponderFFI` (constructor + 7 methods) | 8 | 9 (8 + release) | `ConnectResponder`: `serve` + 7 methods (business API) + its destructor (RAII) |
| `Transport` callbacks + table seam | 3 | 2 (`transport_new` / `transport_free`; the callbacks are table slots) | `TransportCallbacks` (3 `std::function` members) + `Transport::create` + `Transport`'s destructor |
| `PortsHandler` callbacks + table seam | 13 | 2 (`ports_handler_new` / `ports_handler_free`) | `PortsCallbacks` (13 members, each required) + `PortsHandler::create` + its destructor |
| `ToolHandler` callback + table seam | 1 | 2 (`tool_handler_new` / `tool_handler_free`) | `ToolCallbacks` (1 member) + `ToolHandler::create` + its destructor |
| Loopback helpers | 6 | 8 (6 + pair/end releases) | `LoopbackTransportPair::create` / `client` / `server`, `LoopbackTransport::send` / `recv` / `close`, and both destructors |
| Error enumerations | 15 variants | mapped to statuses 100–107 / 200–202 / 300 / 301 / 400 / 401 | `Error` — `status` uses the existing constants, plus `message` / `code` / `kind` / `wire_code` / `expected` / `actual` inside `Result<…>` |
| Boundary-only | — | 4 | `abi_version()`, `Buffer`'s destructor, `detail::OptionalBufferRecord`, `detail::CErrorRecord` |
| **Total** | **82 facade members + 15 error variants** | **83 declarations, 0 unmatched** | **83 declarations consumed, 0 without a counterpart** |

## Re-running the gate

```sh
node tooling/connect/cpp-build.mjs --target aarch64-apple-darwin --toolchain nightly
node tooling/connect/cpp-symbol-check.mjs \
  --header crates/spoke-connect/bindings/cpp/include/spoke_connect.h \
  --library crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib
```

The second command re-derives the declaration set from this header, the export
set from the staged library, and both sides of every record and callback-table
layout (the carrier reports its `#[repr(C)]` mirrors through
`cargo test -p spoke-connect-capi --lib abi_layout`, so cargo has to be on
PATH; the local nightly convention is picked up automatically). A header edit
that misses an export, an export that misses a declaration, or a record or
member whose layout no longer matches the mirror fails the gate rather than
landing.

The C++ column is covered by the same run: the C++17 inclusion probe compiles
`spoke_connect.hpp` (twice, with exceptions and RTTI disabled) and instantiates
the value layer, the full handle set and the three callback factories, so a
wrapper that stopped compiling fails the gate; `--self-test` adds a `.hpp`
mutation in a temporary copy of the whole header tree and requires that probe to
fail on it. The behavior behind the column is proven by

```sh
node tooling/connect/cpp-smoke.mjs --rid osx-arm64
```

which links a translation unit that consumes the layer and runs both
configurations — exceptions disabled and exceptions enabled — each against its
own ordered banner list.
