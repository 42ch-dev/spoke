//! Node lifecycle: transport composition, hello exchange, accept gate,
//! sessions, and op invocation.
//!
//! Composition (locked): **noise** (authenticated transport) + **yamux**
//! (multiplexing) + **request-response** (hello exchange and op invocation) +
//! **identify** (peer metadata — carries the remote public key used to verify
//! hello signatures). Discovery is **explicit peering**: nodes are configured
//! with static listen addresses and dial each other directly; LAN discovery
//! is a future discovery iteration.
//!
//! Hello flow: on connection establishment both sides send their signed
//! `ConnectHello` (fresh nonce per send). The allowlist gate runs at
//! `ConnectionEstablished` — a non-allowlisted peer gets no hello, no
//! buffering, and an immediate disconnect. An inbound hello from an
//! allowlisted peer is accepted only when protocol version, claimed-peer
//! binding, verify-key binding, allowlist, signature, and nonce all pass;
//! rejection closes the hello stream (no error envelope). Remote public keys
//! come from identify and may lag the hello — inbound hellos are buffered
//! per peer up to a fixed cap until the key is known or the handshake timeout
//! hits.
//!
//! Dial flow: `connect` dials are **single-flight** and bound to their
//! connection: the pending dial records the dial's `ConnectionId` and
//! expected `PeerId` once the connection establishes, and only the matching
//! connection can complete or fail it. The event loop owns the dial deadline
//! and clears the entry deterministically (timeout, caller cancellation, or
//! loop exit all resolve the pending reply).
//!
//! Session + invoke flow: once both hellos of a connection are confirmed
//! (remote ack received **and** remote hello accepted), the loop creates a
//! [`crate::PeerSession`] (outbound sequence 0) and completes the pending
//! `connect` only when the session belongs to the dial's peer. `invoke`
//! requests are sent over the invoke protocol
//! (`/spoke/connect/invoke/1.0.0`); responses are correlated by
//! request-response id, their wire echo (`session_id`, `sequence`,
//! `request_id`) is verified, and the caller receives `InvokeSuccess` or
//! `InvokeError` (wire error branch → `InvokeError::Wire`). Inbound invokes
//! are checked for per-session monotonicity before dispatch: the loop tracks
//! the next expected inbound sequence (starts at 0) per sessioned peer and
//! answers a replayed or out-of-order sequence with an `invalid_sequence`
//! wire envelope — no handler side effect runs for it (normative ordering
//! rule, `.mstar/specs/spoke-connect.md` §Ordering semantics). Accepted
//! invokes are dispatched to the configured `invoke_handler` (spike-scoped
//! dispatcher hook; adapter-owned in products). When a peer's last
//! connection closes, live session handles are marked closed and their
//! pending invokes fail fast.

use crate::config::ConnectConfig;
use crate::error::{ConnectError, InvokeError};
use crate::gate::{gate_hello, is_allowlisted, NonceStore};
use crate::hello::{generate_nonce, sign_hello};
use crate::protocol::{HelloAck, HELLO_PROTOCOL, INVOKE_PROTOCOL, MAX_SEQUENCE};
use crate::runtime::{
    generate_request_id, map_invoke_response, InvokeCorrelation, InvokeSuccess, LoopCommand,
    SessionHandle,
};
use crate::session::PeerSession;
use futures::StreamExt;
use libp2p::identity::{Keypair, PublicKey};
use libp2p::request_response::{self, InboundRequestId, OutboundRequestId, ResponseChannel};
use libp2p::swarm::dial_opts::DialOpts;
use libp2p::swarm::{ConnectionId, NetworkBehaviour, Swarm, SwarmEvent};
use libp2p::SwarmBuilder;
use libp2p::{identify, noise, tcp, yamux, Multiaddr, PeerId, StreamProtocol};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::{ConnectInvokeResponse, ErrorEnvelope};
use spoke_schemas::connect::ConnectHello;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch};

/// How long `start` waits for the configured listeners to report resolved
/// addresses before failing.
const LISTEN_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `shutdown` waits for the event loop to stop before aborting it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval for expiring buffered hellos whose identify key never arrived
/// and for sweeping the pending-dial deadline.
const PENDING_HELLO_SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// Fixed per-peer cap on buffered inbound hellos awaiting an identify key.
/// The handshake expects exactly one hello per connection; the cap bounds
/// memory under adversarial senders.
const MAX_PENDING_HELLOS_PER_PEER: usize = 8;

/// Composed libp2p behaviour for a connect node.
#[derive(NetworkBehaviour)]
struct ConnectBehaviour {
    identify: identify::Behaviour,
    hello: request_response::json::Behaviour<ConnectHello, HelloAck>,
    invoke: request_response::json::Behaviour<ConnectInvokeRequest, ConnectInvokeResponse>,
}

impl ConnectBehaviour {
    fn new(keypair: &Keypair, config: &ConnectConfig) -> Self {
        let identify = identify::Behaviour::new(identify::Config::new(
            format!("spoke-connect/{}", env!("CARGO_PKG_VERSION")),
            keypair.public(),
        ));
        let timeout = config.effective_handshake_timeout();
        let hello = request_response::json::Behaviour::<ConnectHello, HelloAck>::new(
            [(
                StreamProtocol::new(HELLO_PROTOCOL),
                request_response::ProtocolSupport::Full,
            )],
            request_response::Config::default().with_request_timeout(timeout),
        );
        let invoke =
            request_response::json::Behaviour::<ConnectInvokeRequest, ConnectInvokeResponse>::new(
                [(
                    StreamProtocol::new(INVOKE_PROTOCOL),
                    request_response::ProtocolSupport::Full,
                )],
                request_response::Config::default().with_request_timeout(timeout),
            );
        Self {
            identify,
            hello,
            invoke,
        }
    }
}

/// A buffered inbound hello awaiting the remote public key.
struct PendingHello {
    request_id: InboundRequestId,
    hello: ConnectHello,
    channel: ResponseChannel<HelloAck>,
    received_at: Instant,
}

/// Hello handshake state for one peer.
#[derive(Default)]
struct PeerHandshake {
    /// We received an `accepted` ack for our outbound hello.
    hello_acked: bool,
    /// We accepted the peer's inbound hello (allowlist + signature + nonce).
    remote_accepted: bool,
    /// The manifest carried by the accepted hello.
    remote_manifest: Option<HostCapabilityManifest>,
}

/// An in-flight outbound invoke awaiting its correlated response.
struct PendingInvoke {
    correlation: InvokeCorrelation,
    reply: oneshot::Sender<Result<InvokeSuccess, InvokeError>>,
}

/// The single in-flight `connect` (spike: serial dials only).
///
/// The entry is **bound to its dial**: `connection` and `peer` are recorded
/// from the dial's own `ConnectionEstablished` event, and only that
/// connection / peer can complete or fail the dial. `deadline` is owned by
/// the event loop, which clears the entry deterministically when the dial
/// times out (even when no failure event ever arrives — e.g. identify never
/// materializing).
struct PendingConnect {
    /// The dial's connection, once its `ConnectionEstablished` arrives.
    connection: Option<ConnectionId>,
    /// The dial's peer, once known from the connection.
    peer: Option<PeerId>,
    /// The event loop clears the entry (and closes the connection) at this
    /// deadline if the handshake has not completed.
    deadline: Instant,
    addr: Multiaddr,
    reply: oneshot::Sender<Result<Arc<SessionHandle>, ConnectError>>,
}

struct EventLoop {
    swarm: Swarm<ConnectBehaviour>,
    identity: Keypair,
    config: ConnectConfig,
    /// Remote public keys from identify, keyed by noise-authenticated peer.
    remote_keys: HashMap<PeerId, PublicKey>,
    /// Inbound hellos waiting for `remote_keys[peer]`.
    pending_hellos: HashMap<PeerId, Vec<PendingHello>>,
    /// Accepted `(peer_id, nonce)` pairs for the life of the process.
    nonces: NonceStore,
    /// Resolved listener addresses (drained by `start` during settle).
    listen_tx: mpsc::Sender<Multiaddr>,
    /// Commands from `connect` / `invoke` callers.
    cmd_rx: mpsc::Receiver<LoopCommand>,
    /// Sender clone handed to new sessions so they can route invokes back.
    cmd_tx: mpsc::Sender<LoopCommand>,
    /// The in-flight `connect` (spike: at most one at a time).
    pending_connect: Option<PendingConnect>,
    /// Hello handshake state per peer.
    handshakes: HashMap<PeerId, PeerHandshake>,
    /// Established sessions per peer (created once both hellos confirm).
    sessions: HashMap<PeerId, Arc<SessionHandle>>,
    /// Next expected inbound invoke sequence per sessioned peer (receiver-side
    /// monotonicity; starts at 0 per session — normative ordering rule).
    /// Maintained in lockstep with `sessions`: created in
    /// `maybe_complete_session`, removed when the peer's last connection
    /// closes.
    inbound_sequences: HashMap<PeerId, u64>,
    /// The dial address each live session was established through (dialer
    /// side only). Used to complete duplicate `connect` calls against the
    /// same address without opening a surplus connection.
    peer_listen_addrs: HashMap<PeerId, Multiaddr>,
    /// In-flight outbound invokes keyed by request-response id.
    pending_invokes: HashMap<OutboundRequestId, PendingInvoke>,
}

impl EventLoop {
    async fn run(mut self, mut shutdown_rx: watch::Receiver<bool>) {
        let mut sweep = tokio::time::interval(PENDING_HELLO_SWEEP_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                _ = sweep.tick() => {
                    self.expire_pending_hellos();
                    self.expire_pending_connect();
                }
                cmd = self.cmd_rx.recv() => match cmd {
                    Some(cmd) => self.handle_command(cmd),
                    // All command senders dropped (node dropped without
                    // shutdown): nothing left to serve.
                    None => break,
                },
                event = self.swarm.next() => match event {
                    Some(event) => self.handle_event(event),
                    None => break,
                },
            }
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<ConnectBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                let _ = self.listen_tx.try_send(address);
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                // Bind the pending dial to its own connection: only an
                // outbound connection that has not yet been attributed can be
                // the dial's outcome. Concurrent inbound connections never
                // claim a pending dial.
                if let Some(pending) = self.pending_connect.as_mut() {
                    if pending.connection.is_none() && endpoint.is_dialer() {
                        pending.peer = Some(peer_id);
                        pending.connection = Some(connection_id);
                    }
                }
                // The allowlist gate runs as early as the noise-authenticated
                // peer id is known: a non-allowlisted peer receives no hello,
                // consumes no buffering, and is disconnected immediately
                // (fail-closed before disclosure or resource use).
                if !is_allowlisted(&self.config.peer_allowlist, &peer_id) {
                    self.fail_pending_connect(
                        Some(connection_id),
                        Some(peer_id),
                        ConnectError::NotAllowlisted { peer_id },
                    );
                    let _ = self.swarm.close_connection(connection_id);
                    return;
                }
                self.send_hello(&peer_id);
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                num_established,
                ..
            } => {
                self.fail_pending_connect(
                    Some(connection_id),
                    Some(peer_id),
                    ConnectError::HandshakeFailed {
                        reason: format!(
                            "connection to {peer_id} closed before session established"
                        ),
                    },
                );
                // A peer may hold several connections (e.g. a duplicate dial).
                // Sessions are per-peer in this spike, so full teardown —
                // keys, buffered hellos, handshake state, and the session —
                // happens only when the peer's LAST connection closes.
                if num_established > 0 {
                    return;
                }
                self.remote_keys.remove(&peer_id);
                self.pending_hellos.remove(&peer_id);
                self.handshakes.remove(&peer_id);
                self.inbound_sequences.remove(&peer_id);
                if let Some(handle) = self.sessions.remove(&peer_id) {
                    // Mark the handle closed before removal: live session
                    // clones fail fast instead of enqueueing on a dead
                    // connection.
                    handle.close();
                }
                self.fail_pending_invokes(&peer_id);
            }
            SwarmEvent::OutgoingConnectionError {
                connection_id,
                peer_id,
                error,
                ..
            } => {
                self.fail_pending_connect(
                    Some(connection_id),
                    peer_id,
                    ConnectError::Transport(format!("dial failed: {error}")),
                );
            }
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                // The identify public key must derive the noise-authenticated
                // peer id: an unbound key is never stored (it could not
                // attest that peer's identity anyway, but dropping it here
                // keeps the verify path honest).
                if info.public_key.to_peer_id() != peer_id {
                    return;
                }
                self.remote_keys.insert(peer_id, info.public_key);
                self.drain_pending_hellos(&peer_id);
            }
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Hello(event)) => {
                self.handle_hello_event(event);
            }
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Invoke(event)) => {
                self.handle_invoke_event(event);
            }
            // Listener errors, dial failures, address churn, bandwidth and
            // other identify events are not part of the hello/invoke path.
            _ => {}
        }
    }

    fn handle_command(&mut self, cmd: LoopCommand) {
        match cmd {
            LoopCommand::Connect { addr, reply } => self.handle_connect(addr, reply),
            LoopCommand::Invoke {
                peer,
                request,
                reply,
            } => self.handle_outbound_invoke(peer, request, reply),
        }
    }

    fn handle_connect(
        &mut self,
        addr: Multiaddr,
        reply: oneshot::Sender<Result<Arc<SessionHandle>, ConnectError>>,
    ) {
        // simplify: single-flight dials. Parallel connects would need a
        // pending-connect map keyed by address; the spike is serial.
        if self.pending_connect.is_some() {
            let _ = reply.send(Err(ConnectError::Transport(
                "a connect is already in progress (spike: single-flight dials)".into(),
            )));
            return;
        }
        // Already-connected fast path: connecting to the recorded listen
        // address of a live session completes with that session (duplicate
        // dial) instead of opening a surplus connection. This is the
        // sanctioned "already connected" duplicate-dial semantic.
        if let Some(handle) = self
            .peer_listen_addrs
            .iter()
            .filter(|(_, recorded)| **recorded == addr)
            .filter_map(|(peer, _)| self.sessions.get(peer))
            .next()
        {
            let _ = reply.send(Ok(handle.clone()));
            return;
        }
        // Dials allocate a fresh source port (`allocate_new_port`): port
        // reuse (the libp2p dial default) sets SO_REUSEPORT on the dial
        // socket, which on macOS can collide with a local listener port and
        // fail repeat loopback dials with EADDRINUSE.
        let opts = DialOpts::unknown_peer_id()
            .address(addr.clone())
            .allocate_new_port()
            .build();
        if let Err(e) = self.swarm.dial(opts) {
            let _ = reply.send(Err(ConnectError::Transport(format!(
                "dial {addr} failed: {e}"
            ))));
            return;
        }
        self.pending_connect = Some(PendingConnect {
            connection: None,
            peer: None,
            deadline: Instant::now() + self.config.effective_handshake_timeout(),
            addr,
            reply,
        });
    }

    fn handle_outbound_invoke(
        &mut self,
        peer: PeerId,
        request: ConnectInvokeRequest,
        reply: oneshot::Sender<Result<InvokeSuccess, InvokeError>>,
    ) {
        // Correlate with the small echo fields only; the full request is
        // moved into the wire send.
        let correlation = InvokeCorrelation {
            peer,
            session_id: request.session_id.to_string(),
            sequence: request.sequence,
            request_id: request.request_id.to_string(),
        };
        let request_id = self
            .swarm
            .behaviour_mut()
            .invoke
            .send_request(&peer, request);
        self.pending_invokes
            .insert(request_id, PendingInvoke { correlation, reply });
    }

    fn handle_hello_event(&mut self, event: request_response::Event<ConnectHello, HelloAck>) {
        match event {
            request_response::Event::Message {
                peer,
                message:
                    request_response::Message::Request {
                        request_id,
                        request,
                        channel,
                    },
                ..
            } => self.handle_inbound_hello(peer, request_id, request, channel),
            request_response::Event::Message {
                peer,
                message:
                    request_response::Message::Response {
                        request_id: _request_id,
                        response,
                    },
                ..
            } => {
                // Ack for our outbound hello.
                if response.accepted {
                    self.handshakes.entry(peer).or_default().hello_acked = true;
                    self.maybe_complete_session(&peer);
                } else {
                    // Protocol v1 acks are always `accepted: true`; a false
                    // ack means the peer rejected our hello.
                    self.fail_pending_connect(
                        None,
                        Some(peer),
                        ConnectError::HandshakeFailed {
                            reason: format!("hello rejected by peer {peer}"),
                        },
                    );
                }
            }
            request_response::Event::OutboundFailure { peer, error, .. } => {
                // Our outbound hello failed: the peer dropped the stream
                // (rejected our hello), timed out, or the connection died.
                self.fail_pending_connect(
                    None,
                    Some(peer),
                    ConnectError::HandshakeFailed {
                        reason: format!("hello exchange with {peer} failed: {error}"),
                    },
                );
            }
            request_response::Event::InboundFailure { .. }
            | request_response::Event::ResponseSent { .. } => {}
        }
    }

    fn handle_invoke_event(
        &mut self,
        event: request_response::Event<ConnectInvokeRequest, ConnectInvokeResponse>,
    ) {
        match event {
            request_response::Event::Message {
                peer,
                message:
                    request_response::Message::Request {
                        request_id,
                        request,
                        channel,
                    },
                ..
            } => self.handle_inbound_invoke(peer, request_id, request, channel),
            request_response::Event::Message {
                peer: _peer,
                message:
                    request_response::Message::Response {
                        request_id,
                        response,
                    },
                ..
            } => {
                if let Some(pending) = self.pending_invokes.remove(&request_id) {
                    let _ = pending
                        .reply
                        .send(map_invoke_response(&pending.correlation, response));
                }
            }
            request_response::Event::OutboundFailure {
                peer: _peer,
                request_id,
                error,
                ..
            } => {
                if let Some(pending) = self.pending_invokes.remove(&request_id) {
                    let err = match error {
                        request_response::OutboundFailure::Timeout => {
                            InvokeError::Transport(format!("invoke request timed out: {error}"))
                        }
                        _ => InvokeError::SessionClosed,
                    };
                    let _ = pending.reply.send(Err(err));
                }
            }
            request_response::Event::InboundFailure { .. }
            | request_response::Event::ResponseSent { .. } => {}
        }
    }

    /// Answer an inbound invoke through the configured handler hook; a peer
    /// without an established session is answered with a wire
    /// `ErrorEnvelope` (protocol v1 has no invoke-level transport error
    /// outside the response).
    ///
    /// Note: session ids are per-side in protocol v1 — each node records its
    /// own opaque id for the pairing (there is no session-announce message),
    /// so the responder validates by *peer*, not by id; the request's
    /// `session_id` is opaque correlation material that the response echoes.
    fn handle_inbound_invoke(
        &mut self,
        peer: PeerId,
        _request_id: InboundRequestId,
        request: ConnectInvokeRequest,
        channel: ResponseChannel<ConnectInvokeResponse>,
    ) {
        let response = if !inbound_sequence_valid(request.sequence) {
            // The generated `sequence` is a bare i64; the schema minimum
            // (0) and JSON-safe ceiling are enforced here on the wire path.
            self.error_response(
                &request,
                "invalid_sequence",
                format!("sequence {} is outside the wire range", request.sequence),
            )
        } else {
            match self.sessions.get(&peer) {
                Some(_session) => {
                    // Inbound monotonicity (normative ordering rule): the
                    // receiver tracks the next expected inbound sequence per
                    // session, starting at 0. A replayed or out-of-order
                    // sequence is answered with a wire `invalid_sequence`
                    // envelope and is never dispatched — no duplicate or
                    // reordered handler side effects.
                    let expected = *self
                        .inbound_sequences
                        .get(&peer)
                        .expect("session and its inbound sequence state are created and removed together");
                    match inbound_sequence_advance(request.sequence, expected) {
                        None => self.error_response(
                            &request,
                            "invalid_sequence",
                            format!(
                                "sequence {} is not the next expected inbound sequence {expected} (replay or out-of-order)",
                                request.sequence
                            ),
                        ),
                        Some(next) => {
                            // The sequence is consumed once accepted, whatever
                            // the response outcome (mirrors the outbound
                            // direction: a failed invoke still consumes its
                            // sequence).
                            self.inbound_sequences.insert(peer, next);
                            match &self.config.invoke_handler {
                                None => self.error_response(
                                    &request,
                                    "op_unsupported",
                                    "no invoke handler configured on this node".into(),
                                ),
                                Some(handler) => {
                                    // The handler runs synchronously on the
                                    // event loop: it must return promptly and
                                    // must not block on I/O (see
                                    // ConnectConfig::invoke_handler). Panics
                                    // are contained so a misbehaving adapter
                                    // cannot kill the node; the invoke is
                                    // answered with an `internal_error` wire
                                    // envelope.
                                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                        || handler(&request.op, request.payload.clone()),
                                    ));
                                    match result {
                                        Ok(Ok(payload)) => ConnectInvokeResponse::Variant0 {
                                            session_id: request.session_id.to_string(),
                                            sequence: request.sequence,
                                            request_id: request.request_id.to_string(),
                                            payload,
                                            extensions: Default::default(),
                                        },
                                        Ok(Err(error)) => ConnectInvokeResponse::Variant1 {
                                            session_id: request.session_id.to_string(),
                                            sequence: request.sequence,
                                            request_id: request.request_id.to_string(),
                                            error,
                                            extensions: Default::default(),
                                        },
                                        Err(_) => self.error_response(
                                            &request,
                                            "internal_error",
                                            "invoke handler panicked".into(),
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
                _ => self.error_response(
                    &request,
                    "session_not_found",
                    format!("no session with {peer} for this request"),
                ),
            }
        };
        let _ = self
            .swarm
            .behaviour_mut()
            .invoke
            .send_response(channel, response);
    }

    /// Build a `ConnectInvokeResponse` error branch echoing the request.
    fn error_response(
        &self,
        request: &ConnectInvokeRequest,
        code: &str,
        message: String,
    ) -> ConnectInvokeResponse {
        ConnectInvokeResponse::Variant1 {
            session_id: request.session_id.to_string(),
            sequence: request.sequence,
            request_id: request.request_id.to_string(),
            error: ErrorEnvelope {
                code: code.into(),
                message,
                details: Default::default(),
                extensions: Default::default(),
            },
            extensions: Default::default(),
        }
    }

    /// Create the session once the peer's side of the handshake confirms.
    ///
    /// - **Dialer side** (this peer is the target of the pending connect):
    ///   both hellos must confirm — the remote acknowledged ours (they
    ///   allowlist us) and we accepted theirs.
    /// - **Responder side** (inbound connection): we have fully validated the
    ///   peer (allowlist + signature + nonce) the moment their hello is
    ///   accepted, so the session exists from then on. This ordering is what
    ///   guarantees the dialer's first invoke finds the responder's session:
    ///   the responder creates it before sending the hello ack, and the
    ///   dialer can only invoke after receiving that ack.
    ///
    /// The pending `connect` is consumed **only** when the completed session
    /// belongs to the dial's peer; a responder-side session for an unrelated
    /// peer never completes or consumes the pending dial.
    fn maybe_complete_session(&mut self, peer: &PeerId) {
        let Some(handshake) = self.handshakes.get(peer) else {
            return;
        };
        let is_pending_peer = self
            .pending_connect
            .as_ref()
            .is_some_and(|pending| pending.peer == Some(*peer));
        if is_pending_peer {
            if !(handshake.hello_acked && handshake.remote_accepted) {
                return;
            }
        } else if !handshake.remote_accepted {
            return;
        }
        if let Some(existing) = self.sessions.get(peer) {
            if is_pending_peer {
                // Duplicate dial: the peer already has a live session.
                // Complete the pending connect with a clone of the existing
                // handle and close the surplus connection the dial just
                // established. The existing session stays on its own
                // connection and keeps working.
                if let Some(pending) = self.pending_connect.take() {
                    if let Some(connection) = pending.connection {
                        let _ = self.swarm.close_connection(connection);
                    }
                    let _ = pending.reply.send(Ok(existing.clone()));
                }
            }
            return;
        }
        let Some(remote_manifest) = handshake.remote_manifest.clone() else {
            return;
        };
        let Ok(session_id) = generate_request_id() else {
            self.fail_pending_connect(
                None,
                Some(*peer),
                ConnectError::Transport("CSPRNG failure while generating session id".into()),
            );
            return;
        };
        let handle = Arc::new(SessionHandle::new(
            session_id,
            *peer,
            remote_manifest,
            self.config.effective_handshake_timeout(),
            self.cmd_tx.clone(),
        ));
        self.sessions.insert(*peer, handle.clone());
        // The receiver-side inbound expectation starts at 0 with the session
        // (lockstep with `sessions`; removed together on connection close).
        self.inbound_sequences.insert(*peer, 0);
        if is_pending_peer {
            if let Some(pending) = self.pending_connect.take() {
                self.peer_listen_addrs.insert(*peer, pending.addr.clone());
                let _ = pending.reply.send(Ok(handle));
            }
        }
    }

    /// Fail the in-flight `connect`, if any, that matches the given dial.
    ///
    /// Matching is strict to keep stale failures from one dial from leaking
    /// into a later one:
    /// - a connection-scoped failure belongs to exactly the dial's own
    ///   connection (recorded from its `ConnectionEstablished`);
    /// - a hello-level failure is attributed by peer id;
    /// - a dial that never establishes is resolved **only** by the deadline
    ///   sweep (`expire_pending_connect`), never by anonymous failures — a
    ///   late failure event from an expired dial must not fail the next
    ///   dial.
    fn fail_pending_connect(
        &mut self,
        connection: Option<ConnectionId>,
        peer: Option<PeerId>,
        err: ConnectError,
    ) {
        let matches =
            self.pending_connect
                .as_ref()
                .is_some_and(|pending| match (connection, peer) {
                    (Some(conn), _) => pending.connection == Some(conn),
                    (None, Some(peer)) => pending.peer == Some(peer),
                    (None, None) => false,
                });
        if matches {
            if let Some(pending) = self.pending_connect.take() {
                let _ = pending.reply.send(Err(err));
            }
        }
    }

    /// Fail every pending invoke for `peer` (called when the peer's last
    /// connection closes; the session is already marked closed).
    fn fail_pending_invokes(&mut self, peer: &PeerId) {
        let expired: Vec<OutboundRequestId> = self
            .pending_invokes
            .iter()
            .filter(|(_, pending)| pending.correlation.peer == *peer)
            .map(|(request_id, _)| *request_id)
            .collect();
        for request_id in expired {
            if let Some(pending) = self.pending_invokes.remove(&request_id) {
                let _ = pending.reply.send(Err(InvokeError::SessionClosed));
            }
        }
    }

    fn handle_inbound_hello(
        &mut self,
        peer: PeerId,
        request_id: InboundRequestId,
        hello: ConnectHello,
        channel: ResponseChannel<HelloAck>,
    ) {
        // Fail closed before buffering: a non-allowlisted peer must not
        // consume buffering or hello responses. `ConnectionEstablished`
        // already disconnects such peers; this guard covers the same window.
        if !is_allowlisted(&self.config.peer_allowlist, &peer) {
            drop(channel);
            return;
        }
        let Some(public_key) = self.remote_keys.get(&peer).cloned() else {
            if !pending_hello_capacity_available(self.pending_hellos.get(&peer).map_or(0, Vec::len))
            {
                // Bounded overflow: reject the newest hello instead of
                // growing the buffer.
                // simplify: fixed per-peer cap. When connection-level limits
                // or identify backpressure land, this cap can ride on them.
                drop(channel);
                return;
            }
            self.pending_hellos
                .entry(peer)
                .or_default()
                .push(PendingHello {
                    request_id,
                    hello,
                    channel,
                    received_at: Instant::now(),
                });
            return;
        };
        self.check_hello(&peer, &public_key, hello, Some((request_id, channel)));
    }

    fn check_hello(
        &mut self,
        peer: &PeerId,
        public_key: &PublicKey,
        hello: ConnectHello,
        respond: Option<(InboundRequestId, ResponseChannel<HelloAck>)>,
    ) {
        match gate_hello(
            peer,
            public_key,
            &self.config.peer_allowlist,
            &mut self.nonces,
            &hello,
        ) {
            Ok(()) => {
                let handshake = self.handshakes.entry(*peer).or_default();
                handshake.remote_accepted = true;
                handshake.remote_manifest = Some(hello.host.clone());
                self.maybe_complete_session(peer);
                if let Some((_request_id, channel)) = respond {
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .hello
                        .send_response(channel, HelloAck::accepted());
                }
            }
            // Rejection closes the stream: dropping the response channel omits
            // the response, surfacing as an inbound failure to the peer.
            // Protocol v1 defines no hello error envelope. A rejected hello
            // from the peer of a pending connect fails that connect fast.
            Err(reason) => {
                self.fail_pending_connect(None, Some(*peer), reason);
                drop(respond);
            }
        }
    }

    fn drain_pending_hellos(&mut self, peer: &PeerId) {
        let Some(public_key) = self.remote_keys.get(peer).cloned() else {
            return;
        };
        if let Some(pending) = self.pending_hellos.remove(peer) {
            for entry in pending {
                self.check_hello(
                    peer,
                    &public_key,
                    entry.hello,
                    Some((entry.request_id, entry.channel)),
                );
            }
        }
    }

    fn expire_pending_hellos(&mut self) {
        let timeout = self.config.effective_handshake_timeout();
        for pending in self.pending_hellos.values_mut() {
            pending.retain(|entry| entry.received_at.elapsed() < timeout);
        }
        self.pending_hellos.retain(|_, pending| !pending.is_empty());
    }

    /// Deterministically clear the pending dial when its deadline passes:
    /// the event loop owns the deadline, so a connect can never leave a
    /// stale entry behind (caller cancellation and missing failure events
    /// included). The dial's connection is closed if it is still alive.
    fn expire_pending_connect(&mut self) {
        let Some(pending) = self.pending_connect.as_ref() else {
            return;
        };
        if pending.deadline > Instant::now() {
            return;
        }
        let err = ConnectError::Timeout(format!("session established with {}", pending.addr));
        let entry = self.pending_connect.take().expect("checked above");
        if let Some(connection) = entry.connection {
            let _ = self.swarm.close_connection(connection);
        }
        let _ = entry.reply.send(Err(err));
    }

    fn send_hello(&mut self, peer: &PeerId) {
        // CSPRNG or signing failures are not recoverable inside the event
        // loop; skip the send (the peer will simply not see a hello).
        let Ok(nonce) = generate_nonce() else { return };
        let Ok(hello) = sign_hello(&self.identity, &nonce, &self.config.local_manifest) else {
            return;
        };
        self.swarm.behaviour_mut().hello.send_request(peer, hello);
    }
}

/// Whether an inbound invoke's wire `sequence` is within the schema range.
///
/// The generated `ConnectInvokeRequest.sequence` is a bare `i64` — typify
/// does not enforce the schema's `minimum: 0` / JSON-safe maximum. The wire
/// path validates before dispatching to any handler.
fn inbound_sequence_valid(sequence: i64) -> bool {
    sequence >= 0 && (sequence as u64) <= MAX_SEQUENCE
}

/// Inbound monotonicity gate: returns the advanced next-expected value when
/// `sequence` equals `next_expected` (the sequential case), or `None` for a
/// replayed or out-of-order sequence.
///
/// The receiver tracks one expectation per session (starts at 0); a
/// replayed sequence (already consumed) and an out-of-order sequence
/// (skipping ahead) both fail the gate and must be rejected with a wire
/// `invalid_sequence` envelope — never dispatched. On `Some`, the sequence
/// is consumed exactly once.
fn inbound_sequence_advance(sequence: i64, next_expected: u64) -> Option<u64> {
    (sequence >= 0 && (sequence as u64) == next_expected).then_some(next_expected + 1)
}

/// Whether another pending hello may be buffered for a peer.
fn pending_hello_capacity_available(pending: usize) -> bool {
    pending < MAX_PENDING_HELLOS_PER_PEER
}

/// A running connect node.
///
/// `Send + Sync`: all shared state lives behind `Arc`; the libp2p swarm is
/// confined to a single tokio task owned by this node.
#[derive(Debug)]
pub struct SpokeConnectNode {
    inner: Arc<NodeInner>,
}

#[derive(Debug)]
struct NodeInner {
    local_peer_id: PeerId,
    local_manifest: HostCapabilityManifest,
    listen_addrs: Vec<Multiaddr>,
    shutdown_tx: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    cmd_tx: mpsc::Sender<LoopCommand>,
}

impl SpokeConnectNode {
    /// Start listening and compose noise + yamux + identify + request-response.
    ///
    /// Resolves the configured listen addresses before returning, so
    /// [`SpokeConnectNode::listen_addrs`] is immediately usable (loopback
    /// tests dial the resolved `127.0.0.1/tcp/<port>`).
    pub async fn start(config: ConnectConfig) -> Result<Self, ConnectError> {
        config.validate()?;
        let local_peer_id = config.identity.public().to_peer_id();
        let local_manifest = config.local_manifest.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (listen_tx, mut listen_rx) = mpsc::channel::<Multiaddr>(16);
        let (cmd_tx, cmd_rx) = mpsc::channel::<LoopCommand>(32);

        let swarm = SwarmBuilder::with_existing_identity(config.identity.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| ConnectError::Transport(format!("transport setup failed: {e}")))?
            .with_behaviour(|keypair| ConnectBehaviour::new(keypair, &config))
            .map_err(|e| ConnectError::Transport(format!("behaviour setup failed: {e}")))?
            .with_swarm_config(|cfg| cfg)
            .with_connection_timeout(config.effective_handshake_timeout())
            .build();

        let mut swarm = swarm;
        for addr in &config.listen_addrs {
            swarm
                .listen_on(addr.clone())
                .map_err(|e| ConnectError::Transport(format!("listen on {addr} failed: {e}")))?;
        }

        let event_loop = EventLoop {
            swarm,
            identity: config.identity.clone(),
            config,
            remote_keys: HashMap::new(),
            pending_hellos: HashMap::new(),
            nonces: NonceStore::new(),
            listen_tx,
            cmd_rx,
            cmd_tx: cmd_tx.clone(),
            pending_connect: None,
            handshakes: HashMap::new(),
            sessions: HashMap::new(),
            inbound_sequences: HashMap::new(),
            peer_listen_addrs: HashMap::new(),
            pending_invokes: HashMap::new(),
        };
        let requested_listeners = event_loop.config.listen_addrs.len();
        let task = tokio::spawn(event_loop.run(shutdown_rx));

        // Settle resolved listener addresses. A listener may report more than
        // one address; we stop early when every configured listener reported
        // at least one, or when the settle window expires (still succeeding
        // with what arrived).
        // simplify: settlement counts resolved addresses, not listeners; the
        // spike accepts a one-listener ceiling. Carry ListenerId through the
        // channel and wait for one address per configured listener when
        // multi-listener support lands.
        let mut listen_addrs = Vec::new();
        let deadline = Instant::now() + LISTEN_SETTLE_TIMEOUT;
        while listen_addrs.len() < requested_listeners {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, listen_rx.recv()).await {
                Ok(Some(addr)) => listen_addrs.push(addr),
                Ok(None) => {
                    return Err(ConnectError::Transport(
                        "event loop exited before listening".into(),
                    ));
                }
                Err(_) => break,
            }
        }
        if listen_addrs.is_empty() {
            return Err(ConnectError::Timeout(
                "listen addresses to become available".into(),
            ));
        }

        Ok(Self {
            inner: Arc::new(NodeInner {
                local_peer_id,
                local_manifest,
                listen_addrs,
                shutdown_tx,
                task: Mutex::new(Some(task)),
                cmd_tx,
            }),
        })
    }

    /// This node's `PeerId` (from the configured identity keypair).
    #[must_use]
    pub fn local_peer_id(&self) -> PeerId {
        self.inner.local_peer_id
    }

    /// The local `HostCapabilityManifest` advertised in signed hellos.
    #[must_use]
    pub fn local_manifest(&self) -> &HostCapabilityManifest {
        &self.inner.local_manifest
    }

    /// Resolved listen addresses as of `start` (post-start resolved
    /// addresses are not tracked in this spike).
    #[must_use]
    pub fn listen_addrs(&self) -> &[Multiaddr] {
        &self.inner.listen_addrs
    }

    /// Explicitly dial `addr` and complete the hello handshake.
    ///
    /// Returns a [`PeerSession`] once **both** hellos of the connection are
    /// confirmed: the remote acknowledged ours (i.e. the remote allowlists
    /// us) and we accepted theirs (i.e. they are on our allowlist with a
    /// valid signature and fresh nonce). Non-allowlisted peers are rejected
    /// with [`ConnectError::NotAllowlisted`] (when our gate rejects their
    /// hello) or `HandshakeFailed` (when they reject ours). A duplicate dial
    /// to a peer with a live session completes with a clone of the existing
    /// session and closes the surplus connection.
    ///
    /// Spike note: dials are single-flight — a second `connect` while one is
    /// in progress fails immediately. The event loop owns the handshake
    /// deadline and resolves the pending dial deterministically, so the
    /// caller-side await is bounded by the configured handshake timeout.
    pub async fn connect(&self, addr: Multiaddr) -> Result<PeerSession, ConnectError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.inner
            .cmd_tx
            .send(LoopCommand::Connect {
                addr: addr.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| ConnectError::Transport("node is not running".into()))?;
        // The event loop resolves this channel deterministically — on
        // success, on failure, on its deadline sweep, or on loop exit.
        let handle = reply_rx
            .await
            .map_err(|_| ConnectError::Transport("event loop stopped while connecting".into()))??;
        Ok(PeerSession::new(handle))
    }

    /// Gracefully stop the event loop and release the listener.
    ///
    /// Propagates event-loop task failure (e.g. an unexpected panic in the
    /// loop) as a transport error instead of swallowing it.
    pub async fn shutdown(self) -> Result<(), ConnectError> {
        let _ = self.inner.shutdown_tx.send(true);
        let task = self
            .inner
            .task
            .lock()
            .expect("task mutex is only held by shutdown")
            .take();
        match task {
            Some(mut task) => match tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut task).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(join_err)) => Err(ConnectError::Transport(format!(
                    "event loop task failed: {join_err}"
                ))),
                Err(_) => {
                    task.abort();
                    Err(ConnectError::Timeout("node shutdown".into()))
                }
            },
            None => Ok(()),
        }
    }
}

/// Multiaddr parsing helper for tests and callers.
///
/// Convenience: parse a string like `"/ip4/127.0.0.1/tcp/0"` into a
/// [`Multiaddr`].
pub fn parse_multiaddr(addr: &str) -> Result<Multiaddr, ConnectError> {
    addr.parse::<Multiaddr>()
        .map_err(|e| ConnectError::Config(format!("invalid multiaddr {addr:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConnectConfig;
    use libp2p::identity::Keypair;

    fn manifest(host_id: &str) -> HostCapabilityManifest {
        HostCapabilityManifest {
            authority: None,
            capabilities: vec!["spoke-baseline".into()],
            extensions: Default::default(),
            host_id: host_id.parse().expect("host id parses"),
            namespaces: Vec::new(),
            roles: vec!["data-store".into()],
            schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
        }
    }

    fn config(identity: Keypair, allowlist: Vec<PeerId>) -> ConnectConfig {
        ConnectConfig {
            identity,
            peer_allowlist: allowlist,
            listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
            local_manifest: manifest("test-host"),
            handshake_timeout: Some(Duration::from_secs(5)),
            invoke_handler: None,
        }
    }

    #[tokio::test]
    async fn start_reports_identity_manifest_and_resolved_listen_addr() {
        let identity = Keypair::generate_ed25519();
        let cfg = config(identity.clone(), Vec::new());
        let node = SpokeConnectNode::start(cfg.clone()).await.expect("start");

        assert_eq!(node.local_peer_id(), identity.public().to_peer_id());
        assert!(!node.listen_addrs().is_empty());
        assert!(node
            .listen_addrs()
            .iter()
            .all(|a| a.to_string().contains("/ip4/127.0.0.1/tcp/")));

        // local_manifest returns the configured manifest (round-trip through
        // the wire type; HostCapabilityManifest has no PartialEq derive).
        assert_eq!(
            serde_json::to_value(node.local_manifest()).expect("serialize"),
            serde_json::to_value(&cfg.local_manifest).expect("serialize"),
        );

        node.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn start_rejects_empty_listen_config() {
        let mut cfg = config(Keypair::generate_ed25519(), Vec::new());
        cfg.listen_addrs.clear();
        let err = SpokeConnectNode::start(cfg)
            .await
            .expect_err("start must fail");
        assert!(matches!(err, ConnectError::Config(_)));
    }

    #[tokio::test]
    async fn shutdown_stops_the_event_loop_gracefully() {
        let node = SpokeConnectNode::start(config(Keypair::generate_ed25519(), Vec::new()))
            .await
            .expect("start");
        node.shutdown().await.expect("shutdown");
    }

    #[test]
    fn inbound_sequence_range_is_enforced() {
        assert!(inbound_sequence_valid(0));
        assert!(inbound_sequence_valid(MAX_SEQUENCE as i64));
        assert!(!inbound_sequence_valid(-1));
        assert!(!inbound_sequence_valid((MAX_SEQUENCE + 1) as i64));
    }

    #[test]
    fn inbound_sequence_gate_rejects_replay_and_out_of_order() {
        // Sequential 0, 1, 2 advance the expectation (the wire path accepts
        // them; covered end-to-end by the two-node integration tests).
        let mut next_expected = 0;
        for sequence in 0..=2 {
            next_expected = inbound_sequence_advance(sequence, next_expected)
                .expect("sequential inbound sequences are accepted");
        }
        assert_eq!(next_expected, 3);
        // A replayed sequence (already consumed) is rejected.
        assert!(inbound_sequence_advance(1, next_expected).is_none());
        // An out-of-order sequence (skipping ahead) is rejected.
        assert!(inbound_sequence_advance((next_expected + 1) as i64, next_expected).is_none());
        // Negative sequences never advance (defense in depth; the range
        // check rejects them first on the wire path).
        assert!(inbound_sequence_advance(-1, 0).is_none());
    }

    #[test]
    fn pending_hello_buffer_is_bounded_per_peer() {
        assert!(pending_hello_capacity_available(0));
        assert!(pending_hello_capacity_available(
            MAX_PENDING_HELLOS_PER_PEER - 1
        ));
        assert!(!pending_hello_capacity_available(
            MAX_PENDING_HELLOS_PER_PEER
        ));
    }
}
