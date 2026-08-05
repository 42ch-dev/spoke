//! RemoteAdapter + message-oriented Transport (frozen contract:
//! `.mstar/iterations/v0-iter030/guides/remote-adapter-contract.md`).
//!
//! This module is gated behind the `remote-adapter` cargo feature so default
//! connect builds stay lean (no `spoke-operations` dependency); the TS
//! equivalent is the `./remote` subpath export.
//!
//! Public surface: [`RemoteAdapter`] (async `BaselinePorts` + read-only
//! session info + `close`), [`connect_remote_adapter`] (dial), the
//! [`Transport`] trait, the in-repo loopback transport pair for tests, and
//! the multi-peer capability router ([`MultiPeerRouter`] +
//! [`connect_multi_peer_router`] — capability-selected async `BaselinePorts`
//! over N registered per-peer adapters, frozen multi-peer routing contract).

mod multi_peer_router;
mod remote_adapter;
pub mod transport;

pub use multi_peer_router::{
    connect_multi_peer_router, select_peer_for_op, MultiPeerRouter, MultiPeerRouterError,
    MultiPeerRouterOptions, RoutedRemoteAdapter, SelectablePeer,
};
pub use remote_adapter::{
    connect_remote_adapter, reset_accepted_server_hellos_for_test, RemoteAdapter,
    RemoteAdapterError, RemoteAdapterOptions, RemoteAdapterState, RemoteIdentity,
};
pub use transport::{
    loopback_transport_pair, LoopbackTransport, LoopbackTransportPair, Transport, TransportError,
};
