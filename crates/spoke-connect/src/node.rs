//! Node lifecycle: transport composition, hello exchange, accept gate,
//! sessions, and op invocation.
//!
//! Composition (locked): **noise** (authenticated transport) + **yamux**
//! (multiplexing) + **request-response** (hello exchange and op invocation) +
//! **identify** (peer metadata — carries the remote public key used to verify
//! hello signatures). Discovery defaults to explicit peering; mDNS is a
//! non-default cargo feature.
//!
//! Hello flow: on connection establishment both sides send their signed
//! `ConnectHello` (fresh nonce per send). An inbound hello is accepted only
//! when protocol version, claimed-peer binding, allowlist, signature, and
//! nonce all pass; rejection closes the hello stream (no error envelope).
//! Remote public keys come from identify and may lag the hello — inbound
//! hellos are buffered until the key is known or the handshake timeout hits.
//!
//! Session + invoke flow: once both hellos of a connection are confirmed
//! (remote ack received **and** remote hello accepted), the loop creates a
//! [`crate::PeerSession`] (outbound sequence 0) and completes any pending
//! `connect`. `invoke` requests are sent over the invoke protocol
//! (`/spoke/connect/invoke/1.0.0`); responses are correlated by
//! request-response id, their wire echo (`session_id`, `sequence`,
//! `request_id`) is verified, and the caller receives `InvokeSuccess` or
//! `InvokeError` (wire error branch → `InvokeError::Wire`). Inbound invokes
//! are dispatched to the configured `invoke_handler` (spike-scoped dispatcher
//! hook; adapter-owned in products).

use crate::config::ConnectConfig;
use crate::error::{ConnectError, InvokeError};
use crate::gate::{gate_hello, NonceStore};
use crate::hello::{generate_nonce, sign_hello};
use crate::protocol::{HelloAck, HELLO_PROTOCOL, INVOKE_PROTOCOL};
use crate::session::{
    generate_request_id, map_invoke_response, InvokeSuccess, PeerSession, SessionHandle,
};
use futures::StreamExt;
use libp2p::identity::{Keypair, PublicKey};
use libp2p::request_response::{self, InboundRequestId, OutboundRequestId, ResponseChannel};
use libp2p::swarm::{NetworkBehaviour, Swarm, SwarmEvent};
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

/// Interval for expiring buffered hellos whose identify key never arrived.
const PENDING_HELLO_SWEEP_INTERVAL: Duration = Duration::from_secs(1);

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

/// Commands from public handles to the node's event loop.
///
/// The invoke variant carries the full request envelope (HashMap + opaque
/// Value), making it much larger than the connect variant; the enum is short-
/// lived channel payload, so the size difference is not worth boxing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum LoopCommand {
    /// Dial an address and complete the hello handshake into a session.
    Connect {
        addr: Multiaddr,
        reply: oneshot::Sender<Result<PeerSession, ConnectError>>,
    },
    /// Send an invoke request to `peer` and correlate the response.
    Invoke {
        peer: PeerId,
        request: ConnectInvokeRequest,
        reply: oneshot::Sender<Result<InvokeSuccess, InvokeError>>,
    },
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
    request: ConnectInvokeRequest,
    reply: oneshot::Sender<Result<InvokeSuccess, InvokeError>>,
}

/// The single in-flight `connect` (spike: serial dials only).
struct PendingConnect {
    /// The dialed peer once known (set on `ConnectionEstablished`).
    peer: Option<PeerId>,
    reply: oneshot::Sender<Result<PeerSession, ConnectError>>,
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
    /// In-flight outbound invokes keyed by request-response id.
    pending_invokes: HashMap<OutboundRequestId, PendingInvoke>,
}

impl EventLoop {
    async fn run(mut self, mut shutdown_rx: watch::Receiver<bool>) {
        let mut sweep = tokio::time::interval(PENDING_HELLO_SWEEP_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                _ = sweep.tick() => self.expire_pending_hellos(),
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
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                // If a connect is pending and we don't know the dialed peer
                // yet, this established connection is the dial's outcome.
                if let Some(pending) = self.pending_connect.as_mut() {
                    if pending.peer.is_none() {
                        pending.peer = Some(peer_id);
                    }
                }
                self.send_hello(&peer_id);
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                // Dropping buffered entries closes their channels (rejects the
                // pending hellos of a dead connection); a pending connect on
                // this peer can no longer complete.
                self.remote_keys.remove(&peer_id);
                self.pending_hellos.remove(&peer_id);
                self.handshakes.remove(&peer_id);
                self.sessions.remove(&peer_id);
                self.fail_pending_connect(
                    Some(peer_id),
                    ConnectError::HandshakeFailed {
                        reason: format!(
                            "connection to {peer_id} closed before session established"
                        ),
                    },
                );
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                self.fail_pending_connect(
                    peer_id,
                    ConnectError::Transport(format!("dial failed: {error}")),
                );
            }
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
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
        reply: oneshot::Sender<Result<PeerSession, ConnectError>>,
    ) {
        // simplify: single-flight dials. Parallel connects would need a
        // pending-connect map keyed by address; the spike is serial.
        if self.pending_connect.is_some() {
            let _ = reply.send(Err(ConnectError::Transport(
                "a connect is already in progress (spike: single-flight dials)".into(),
            )));
            return;
        }
        if let Err(e) = self.swarm.dial(addr.clone()) {
            let _ = reply.send(Err(ConnectError::Transport(format!(
                "dial {addr} failed: {e}"
            ))));
            return;
        }
        self.pending_connect = Some(PendingConnect { peer: None, reply });
    }

    fn handle_outbound_invoke(
        &mut self,
        peer: PeerId,
        request: ConnectInvokeRequest,
        reply: oneshot::Sender<Result<InvokeSuccess, InvokeError>>,
    ) {
        let request_id = self
            .swarm
            .behaviour_mut()
            .invoke
            .send_request(&peer, request.clone());
        self.pending_invokes
            .insert(request_id, PendingInvoke { request, reply });
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
                        .send(map_invoke_response(&pending.request, response));
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
        let response = match self.sessions.get(&peer) {
            Some(_session) => match &self.config.invoke_handler {
                None => self.error_response(
                    &request,
                    "op_unsupported",
                    "no invoke handler configured on this node".into(),
                ),
                Some(handler) => match handler(&request.op, request.payload.clone()) {
                    Ok(payload) => ConnectInvokeResponse::Variant0 {
                        session_id: request.session_id.to_string(),
                        sequence: request.sequence,
                        request_id: request.request_id.to_string(),
                        payload,
                        extensions: Default::default(),
                    },
                    Err(error) => ConnectInvokeResponse::Variant1 {
                        session_id: request.session_id.to_string(),
                        sequence: request.sequence,
                        request_id: request.request_id.to_string(),
                        error,
                        extensions: Default::default(),
                    },
                },
            },
            _ => self.error_response(
                &request,
                "session_not_found",
                format!("no session with {peer} for this request"),
            ),
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
        if self.sessions.contains_key(peer) {
            return;
        }
        let Some(remote_manifest) = handshake.remote_manifest.clone() else {
            return;
        };
        let Ok(session_id) = generate_request_id() else {
            self.fail_pending_connect(
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
        if let Some(pending) = self.pending_connect.take() {
            let _ = pending.reply.send(Ok(PeerSession::new(handle)));
        }
    }

    /// Fail the in-flight `connect`, if any, that matches `peer` (or any
    /// pending connect when the peer is unknown — the never-connected dial
    /// failure case).
    fn fail_pending_connect(&mut self, peer: Option<PeerId>, err: ConnectError) {
        let matches = match peer {
            Some(peer) => self
                .pending_connect
                .as_ref()
                .is_some_and(|pending| pending.peer == Some(peer)),
            None => self.pending_connect.is_some(),
        };
        if matches {
            if let Some(pending) = self.pending_connect.take() {
                let _ = pending.reply.send(Err(err));
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
        let Some(public_key) = self.remote_keys.get(&peer).cloned() else {
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
                self.fail_pending_connect(Some(*peer), reason);
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
    handshake_timeout: Duration,
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
        let handshake_timeout = config.effective_handshake_timeout();
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
            pending_invokes: HashMap::new(),
        };
        let requested_listeners = event_loop.config.listen_addrs.len();
        let task = tokio::spawn(event_loop.run(shutdown_rx));

        // Settle resolved listener addresses. A listener may report more than
        // one address; we stop early when every configured listener reported
        // at least one, or when the settle window expires (still succeeding
        // with what arrived).
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
                handshake_timeout,
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
    /// hello) or `HandshakeFailed` (when they reject ours).
    ///
    /// Spike note: dials are single-flight — a second `connect` while one is
    /// in progress fails immediately.
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
        tokio::time::timeout(self.inner.handshake_timeout, reply_rx)
            .await
            .map_err(|_| ConnectError::Timeout(format!("session established with {addr}")))?
            .map_err(|_| ConnectError::Transport("event loop stopped while connecting".into()))?
    }

    /// Gracefully stop the event loop and release the listener.
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
                Ok(_) => Ok(()),
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
}
