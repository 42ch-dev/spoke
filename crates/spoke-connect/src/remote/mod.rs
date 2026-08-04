//! RemoteAdapter + message-oriented Transport (frozen contract:
//! `.mstar/iterations/v0-iter030/guides/remote-adapter-contract.md`).
//!
//! This module is gated behind the `remote-adapter` cargo feature so default
//! connect builds stay lean (no `spoke-operations` dependency); the TS
//! equivalent is the `./remote` subpath export.
//!
//! Public surface: [`RemoteAdapter`] (async `BaselinePorts` + read-only
//! session info + `close`), [`connect_remote_adapter`] (dial), the
//! [`Transport`] trait, and the in-repo loopback transport pair for tests.

mod remote_adapter;
pub mod transport;

pub use remote_adapter::{
    connect_remote_adapter, RemoteAdapter, RemoteAdapterError, RemoteAdapterOptions,
    RemoteAdapterState, RemoteIdentity,
};
pub use transport::{
    loopback_transport_pair, LoopbackTransport, LoopbackTransportPair, Transport, TransportError,
};
