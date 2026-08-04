//! Message-oriented `Transport` seam for `RemoteAdapter` (frozen contract §2).
//!
//! One connect envelope = one `send` / `recv` call — no multi-envelope
//! batching, matching `spoke-connect.md` §Transport framing. The
//! `RemoteAdapter` owns a single receive loop that calls `recv` continuously
//! and demultiplexes by `request_id`; callers of `BaselinePorts` never call
//! `recv`.
//!
//! WebSocket and other product transports are consumer-side; this module
//! ships the trait plus the in-repo loopback implementation used by tests
//! (frozen contract §2.2: "Loopback | In-repo paired queues for tests; ships
//! in spoke-connect"). Byte-stream carriers (TCP, pipes) document their
//! length-prefix (or equivalent) delimiting responsibility as
//! transport-adapter-owned.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::oneshot;

/// Typed transport failure.
///
/// `RemoteAdapter` maps these to `SpokeResult` `INTERNAL_ERROR` rejects with
/// `details.kind = "transport"` (frozen contract §8.2) — they never surface
/// on `BaselinePorts` directly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The transport is closed. A pending `recv` must fail fast on connection
    /// loss so the adapter can fail its in-flight invokes instead of waiting
    /// out their timeout.
    #[error("transport is closed")]
    Closed,
    /// Transport-level I/O failure.
    #[error("transport I/O error: {0}")]
    Io(String),
}

/// Message-oriented transport: one connect envelope per `send` / `recv` call.
///
/// Mirrors the TS `Transport` interface (frozen contract §2.1): `send`
/// accepts exactly one envelope's bytes, `recv` returns the next inbound
/// envelope and fails fast on close, `close` is optional resource release
/// (idempotent; default no-op).
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send one envelope. Resolves when the transport has accepted the bytes.
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError>;
    /// Receive the next inbound envelope. Errors when the transport closes.
    async fn recv(&self) -> Result<Vec<u8>, TransportError>;
    /// Release resources. Idempotent.
    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

/// FIFO message channel: buffered pushes, awaitable pops. One direction of a
/// loopback connection. Closing rejects every pending and future pop
/// (buffered messages are lost on close, matching a real connection drop).
struct LoopbackChannel {
    state: Mutex<ChannelState>,
    closed: AtomicBool,
}

/// Shared channel state: buffered envelopes + parked `recv` waiters.
struct ChannelState {
    buffer: VecDeque<Vec<u8>>,
    waiters: VecDeque<oneshot::Sender<Vec<u8>>>,
}

impl LoopbackChannel {
    fn new() -> Self {
        Self {
            state: Mutex::new(ChannelState {
                buffer: VecDeque::new(),
                waiters: VecDeque::new(),
            }),
            closed: AtomicBool::new(false),
        }
    }

    /// Push one envelope; resolves the oldest waiting `recv` when one exists.
    fn push(&self, bytes: Vec<u8>) -> Result<(), TransportError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(TransportError::Closed);
        }
        let mut state = self.state.lock().expect("loopback channel lock");
        if let Some(waiter) = state.waiters.pop_front() {
            // The waiter task was cancelled → the message is dropped, same as
            // the TS loopback (resolve on a settled promise is a no-op).
            let _ = waiter.send(bytes);
            return Ok(());
        }
        state.buffer.push_back(bytes);
        Ok(())
    }

    /// Pop the next envelope. Resolves immediately when buffered, otherwise
    /// waits for the next push. Errors when the channel is closed.
    async fn pop(&self) -> Result<Vec<u8>, TransportError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(TransportError::Closed);
        }
        let rx = {
            let mut state = self.state.lock().expect("loopback channel lock");
            if let Some(bytes) = state.buffer.pop_front() {
                return Ok(bytes);
            }
            let (tx, rx) = oneshot::channel();
            state.waiters.push_back(tx);
            rx
        };
        rx.await.map_err(|_| TransportError::Closed)
    }

    /// Close the channel: reject every pending and future pop.
    fn close(&self) {
        if self.closed.swap(true, Ordering::Relaxed) {
            return;
        }
        let mut state = self.state.lock().expect("loopback channel lock");
        state.buffer.clear();
        // Dropping the senders rejects the parked `recv` futures.
        state.waiters.drain(..);
    }
}

/// Which end of the connection an end is.
#[derive(Debug, Clone, Copy)]
enum LoopbackDirection {
    ClientToServer,
    ServerToClient,
}

/// Shared bidirectional connection state. Both directions close together:
/// closing one end fails the peer's pending `recv` exactly like a real
/// connection close, so the `RemoteAdapter` sees transport loss.
struct LoopbackConnection {
    client_to_server: LoopbackChannel,
    server_to_client: LoopbackChannel,
}

impl LoopbackConnection {
    fn outbound_channel(&self, direction: LoopbackDirection) -> &LoopbackChannel {
        match direction {
            LoopbackDirection::ClientToServer => &self.client_to_server,
            LoopbackDirection::ServerToClient => &self.server_to_client,
        }
    }

    fn inbound_channel(&self, direction: LoopbackDirection) -> &LoopbackChannel {
        match direction {
            LoopbackDirection::ClientToServer => &self.server_to_client,
            LoopbackDirection::ServerToClient => &self.client_to_server,
        }
    }

    fn close(&self) {
        self.client_to_server.close();
        self.server_to_client.close();
    }
}

/// One end of an in-memory loopback connection. `send` delivers to the peer's
/// `recv`; `close` closes the whole connection (both directions).
#[derive(Clone)]
pub struct LoopbackTransport {
    connection: Arc<LoopbackConnection>,
    direction: LoopbackDirection,
}

#[async_trait]
impl Transport for LoopbackTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        self.connection
            .outbound_channel(self.direction)
            .push(envelope.to_vec())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.connection.inbound_channel(self.direction).pop().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.connection.close();
        Ok(())
    }
}

/// Back-to-back loopback transport pair — `client` and `server` ends of the
/// same in-memory connection. Used by loopback interop tests: the
/// `RemoteAdapter` dials the `client` end, the test host serves the `server`
/// end (mirror of TS `loopbackTransportPair`).
pub struct LoopbackTransportPair {
    pub client: LoopbackTransport,
    pub server: LoopbackTransport,
}

/// Create a back-to-back loopback transport pair (client + server ends).
#[must_use]
pub fn loopback_transport_pair() -> LoopbackTransportPair {
    let connection = Arc::new(LoopbackConnection {
        client_to_server: LoopbackChannel::new(),
        server_to_client: LoopbackChannel::new(),
    });
    LoopbackTransportPair {
        client: LoopbackTransport {
            connection: Arc::clone(&connection),
            direction: LoopbackDirection::ClientToServer,
        },
        server: LoopbackTransport {
            connection: Arc::clone(&connection),
            direction: LoopbackDirection::ServerToClient,
        },
    }
}
