//! Node lifecycle: transport composition, hello exchange, accept gate,
//! sessions, and op invocation.
//!
//! Composition (locked): **noise** (authenticated transport) + **yamux**
//! (multiplexing) + **request-response** (hello exchange and op invocation) +
//! **identify** (peer metadata — carries the remote public key used to verify
//! hello signatures). Discovery is **explicit peering**: nodes are configured
//! with static listen addresses and dial each other directly. The non-default
//! `mdns` cargo feature adds same-LAN peer discovery via libp2p's mDNS
//! behaviour — address discovery only, never a trust grant: discovered peers
//! still pass the allowlist + signed-hello gates.
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
//! connection can complete or fail it. An explicit `connect` that replaces
//! an in-flight auto-dial keeps the replaced dial's identity (connection id,
//! dial address) so late events of the replaced dial — a
//! `ConnectionEstablished` still in flight, its `ConnectionClosed`, a hello
//! failure — never bind or fail the replacement. The event loop owns the
//! dial deadline and clears the entry deterministically (timeout, caller
//! cancellation, or loop exit all resolve the pending reply).
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
//! invokes pass the op dispatch gate first: an op whose required capability
//! is absent from the session's `negotiated_capabilities` (the intersection
//! of both manifests, computed at session establishment) is answered with an
//! `op_unsupported` wire envelope and the handler is never called
//! (normative rule, §Op dispatch gate). Required capabilities come from the
//! core-op table first; product-defined ops resolve theirs through the
//! configurable `op_capability_requirements` map. Gate-passing invokes are
//! dispatched
//! to the configured `invoke_handler` (spike-scoped dispatcher hook;
//! adapter-owned in products). When a peer's last connection closes, live
//! session handles are marked closed and their pending invokes fail fast.

use crate::config::ConnectConfig;
use crate::core::{
    token_authorizes_op, verify_capability_token, CapabilityTokenProof, CoreError, CoreInvokeError,
    InboundSequence,
};
use crate::error::{ConnectError, InvokeError};
use crate::gate::{gate_hello, is_allowlisted, NonceStore};
use crate::hello::{generate_nonce, sign_hello};
use crate::protocol::{
    HelloAck, AUTH_PROTOCOL, HELLO_PROTOCOL, INVOKE_PROTOCOL, MAX_SEQUENCE,
    METHOD_CAPABILITY_TOKEN, TOKEN_CHALLENGE_MIN_LENGTH,
};
use crate::runtime::{
    generate_request_id, map_invoke_response, InvokeCorrelation, InvokeSuccess, LoopCommand,
    SessionHandle,
};
use crate::session::PeerSession;
use futures::StreamExt;
use libp2p::identity::{Keypair, PublicKey};
#[cfg(feature = "mdns")]
use libp2p::mdns;
use libp2p::request_response::{self, InboundRequestId, OutboundRequestId, ResponseChannel};
use libp2p::swarm::dial_opts::DialOpts;
use libp2p::swarm::{ConnectionId, NetworkBehaviour, Swarm, SwarmEvent};
use libp2p::SwarmBuilder;
use libp2p::{identify, noise, tcp, yamux, Multiaddr, PeerId, StreamProtocol};
use spoke_schemas::connect::connect_auth_challenge::ConnectAuthChallenge;
use spoke_schemas::connect::connect_auth_response::ConnectAuthResponse;
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::{ConnectInvokeResponse, ErrorEnvelope};
use spoke_schemas::connect::ConnectHello;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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

/// Hard cap on recorded mDNS dial candidates. mDNS is unauthenticated, so a
/// forged-discovery flood could otherwise grow the store without bound; new
/// `Discovered` entries beyond the cap are dropped (and never dialed).
#[cfg(feature = "mdns")]
const MAX_MDNS_CANDIDATES: usize = 256;

/// Composed libp2p behaviour for a connect node.
#[derive(NetworkBehaviour)]
struct ConnectBehaviour {
    identify: identify::Behaviour,
    hello: request_response::json::Behaviour<ConnectHello, HelloAck>,
    invoke: request_response::json::Behaviour<ConnectInvokeRequest, ConnectInvokeResponse>,
    auth: request_response::json::Behaviour<ConnectAuthChallenge, ConnectAuthResponse>,
    /// Same-LAN discovery (non-default `mdns` feature): emits `Discovered`
    /// peer addresses; never grants trust.
    #[cfg(feature = "mdns")]
    mdns: mdns::tokio::Behaviour,
}

impl ConnectBehaviour {
    fn new(keypair: &Keypair, config: &ConnectConfig) -> Result<Self, ConnectError> {
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
        let auth =
            request_response::json::Behaviour::<ConnectAuthChallenge, ConnectAuthResponse>::new(
                [(
                    StreamProtocol::new(AUTH_PROTOCOL),
                    request_response::ProtocolSupport::Full,
                )],
                request_response::Config::default().with_request_timeout(timeout),
            );
        // 0.56 constructor form (verified at impl): mdns 0.48's
        // `Behaviour::new(Config, PeerId)` returns `io::Result` — the
        // interface watcher creation is the only fallible step; multicast
        // traffic only starts once the behaviour is polled.
        #[cfg(feature = "mdns")]
        let mdns =
            mdns::tokio::Behaviour::new(mdns::Config::default(), keypair.public().to_peer_id())
                .map_err(|e| ConnectError::Transport(format!("mdns setup failed: {e}")))?;
        Ok(Self {
            identify,
            hello,
            invoke,
            auth,
            #[cfg(feature = "mdns")]
            mdns,
        })
    }
}

/// Compose the libp2p transport and behaviour stack for a connect node.
///
/// Shared by [`SpokeConnectNode::start`] and the mdns unit tests (which
/// build a loop without running it — no network I/O happens at this stage).
fn build_swarm(
    identity: &Keypair,
    config: &ConnectConfig,
) -> Result<Swarm<ConnectBehaviour>, ConnectError> {
    Ok(SwarmBuilder::with_existing_identity(identity.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ConnectError::Transport(format!("transport setup failed: {e}")))?
        .with_behaviour(|keypair| {
            ConnectBehaviour::new(keypair, config)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
        .map_err(|e| ConnectError::Transport(format!("behaviour setup failed: {e}")))?
        .with_swarm_config(|cfg| cfg)
        .with_connection_timeout(config.effective_handshake_timeout())
        .build())
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

/// An outstanding capability-token challenge sent to a peer.
///
/// The challenge nonce is bound to the `challenge_id` server-side (anti-replay
/// for the challenge slot; protocol_version 1 does not require the client to
/// embed challenge bytes inside token claims). One outstanding challenge per
/// peer in the spike; the entry is removed when the peer answers, when the
/// outbound challenge fails (timeout, dropped stream, or connection error),
/// or when the peer's last connection closes.
#[derive(Clone)]
struct PendingChallenge {
    challenge_id: String,
    /// The opaque random nonce sent in the challenge (not consulted by
    /// token validation — reserved bookkeeping for a future binding design).
    #[allow(dead_code)]
    nonce: String,
}

/// A dial that an explicit `connect` replaced mid-flight (a preempted
/// auto-dial). Its identity is kept on the replacement's pending entry so
/// late transport events of the replaced dial — a `ConnectionEstablished`
/// still in flight when the preemption happened, the `ConnectionClosed`
/// that follows the close, an outbound hello failure — are attributed to
/// the replaced dial and never bind or fail the replacement.
struct SupersededDial {
    /// The replaced dial's connection, when its `ConnectionEstablished`
    /// had already been processed before the preemption (the close is
    /// issued at preemption; the recorded id guards against any late
    /// duplicate).
    connection: Option<ConnectionId>,
    /// The address the replaced dial targeted. Recognizes the replaced
    /// dial's late `ConnectionEstablished` when its connection id was not
    /// yet known at preemption time.
    addr: Multiaddr,
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
    /// Whether this dial was started by mDNS auto-dial (no live caller —
    /// the reply receiver is already dropped). An explicit `connect` replaces
    /// such a dial instead of being rejected with "already in progress".
    autodial: bool,
    /// The outbound hello sent for this dial, once the dial's connection
    /// bound its peer. A hello response or failure carrying a different
    /// request id belongs to an earlier dial and never resolves this one.
    hello_request: Option<OutboundRequestId>,
    /// The auto-dial this explicit connect replaced, when there was one:
    /// late events of the replaced dial must not bind or fail this dial.
    superseded: Option<SupersededDial>,
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
    /// monotonicity; starts at 0 per session — normative ordering rule). The
    /// rule lives in [`InboundSequence`] (pure core); this map is the
    /// transport's per-peer storage, maintained in lockstep with `sessions`:
    /// created in `maybe_complete_session`, removed when the peer's last
    /// connection closes.
    inbound_sequences: HashMap<PeerId, InboundSequence>,
    /// The dial address each live session was established through (dialer
    /// side only). Used to complete duplicate `connect` calls against the
    /// same address without opening a surplus connection.
    peer_listen_addrs: HashMap<PeerId, Multiaddr>,
    /// The connection each session was established through (dialer side
    /// only). The duplicate-dial completion path closes the pending dial's
    /// connection as surplus **unless** it is the session's own connection —
    /// which happens when the capability-token gate delayed the pending
    /// connect's completion past session creation.
    session_connections: HashMap<PeerId, ConnectionId>,
    /// In-flight outbound invokes keyed by request-response id.
    pending_invokes: HashMap<OutboundRequestId, PendingInvoke>,
    /// Outstanding capability-token challenges sent to each peer (one per
    /// peer in the spike), keyed by the challenged peer.
    pending_challenges: HashMap<PeerId, PendingChallenge>,
    /// Dial candidates recorded from mDNS `Discovered` events (same-LAN
    /// discovery only; drained by `take_mdns_discoveries`). Each entry
    /// carries the auto-dial attempted flag (false = not yet dialed, or
    /// reset by a re-emission): candidates skipped while the single-flight
    /// slot is busy stay un-attempted and are dialed when the slot frees.
    /// mDNS records addresses — it never grants trust: auto-dials still pass
    /// the `ConnectionEstablished` allowlist gate.
    #[cfg(feature = "mdns")]
    mdns_candidates: Vec<(PeerId, Multiaddr, bool)>,
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
                // A late connection of the auto-dial that an explicit connect
                // replaced is closed on arrival: it must not bind the pending
                // slot (its `ConnectionEstablished` can race the preemption)
                // nor start a hello exchange for a dial nobody is waiting for.
                let dialer_address = match &endpoint {
                    libp2p::core::connection::ConnectedPoint::Dialer { address, .. } => {
                        Some(address)
                    }
                    _ => None,
                };
                let from_replaced_dial = match dialer_address {
                    None => false,
                    Some(address) => self.pending_connect.as_ref().is_some_and(|pending| {
                        pending
                            .superseded
                            .as_ref()
                            .is_some_and(|sup| match sup.connection {
                                // The replaced dial's connection id is known:
                                // attribute by id (dial addresses may
                                // legitimately coincide).
                                Some(known) => connection_id == known,
                                // Unknown at preemption: attribute by dial
                                // address, but only when it differs from the
                                // current dial's address — identical addresses
                                // make the two dials interchangeable, so binding
                                // is benign and closing would break the explicit
                                // dial.
                                None => *address == sup.addr && *address != pending.addr,
                            })
                    }),
                };
                if from_replaced_dial {
                    let _ = self.swarm.close_connection(connection_id);
                    return;
                }
                // Bind the pending dial to its own connection: only an
                // outbound connection that has not yet been attributed can be
                // the dial's outcome. Concurrent inbound connections never
                // claim a pending dial.
                if let Some(pending) = self.pending_connect.as_mut() {
                    if pending.connection.is_none() && dialer_address.is_some() {
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
                self.pending_challenges.remove(&peer_id);
                self.session_connections.remove(&peer_id);
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
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Auth(event)) => {
                self.handle_auth_event(event);
            }
            #[cfg(feature = "mdns")]
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Mdns(event)) => {
                self.handle_mdns_event(event);
            }
            // Listener errors, dial failures, address churn, bandwidth and
            // other identify events are not part of the hello/invoke path.
            _ => {}
        }
    }

    fn handle_command(&mut self, cmd: LoopCommand) {
        match cmd {
            LoopCommand::Connect { addr, reply } => self.handle_connect(addr, reply, false),
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
        autodial: bool,
    ) {
        // simplify: single-flight dials. Parallel connects would need a
        // pending-connect map keyed by address; the spike is serial.
        //
        // An explicit connect replaces an in-flight auto-dial (the auto-dial
        // has no live caller — its reply receiver is already dropped): the
        // replaced dial's identity is kept so its late transport events (a
        // `ConnectionEstablished` still in flight, the `ConnectionClosed`
        // that follows the close, a hello failure) never bind or fail the
        // replacement; its connection is closed, its candidate is reset so
        // the slot-free drain retries it, and the explicit dial proceeds. A
        // second explicit connect during an explicit dial is still rejected.
        let mut superseded: Option<SupersededDial> = None;
        if let Some(pending) = self.pending_connect.take() {
            if pending.autodial && !autodial {
                superseded = Some(SupersededDial {
                    connection: pending.connection,
                    addr: pending.addr.clone(),
                });
                if let Some(connection) = pending.connection {
                    let _ = self.swarm.close_connection(connection);
                }
                drop(pending.reply);
                #[cfg(feature = "mdns")]
                self.reset_mdns_candidate(&pending.addr);
            } else {
                self.pending_connect = Some(pending);
                let _ = reply.send(Err(ConnectError::Transport(
                    "a connect is already in progress (spike: single-flight dials)".into(),
                )));
                return;
            }
        }
        // Already-connected fast path: connecting to the recorded listen
        // address of a live session completes with that session (duplicate
        // dial) instead of opening a surplus connection. This is the
        // sanctioned "already connected" duplicate-dial semantic. When the
        // token policy is active, the fast path only completes for sessions
        // whose challenge already succeeded; otherwise the dial proceeds and
        // waits for the token exchange through the normal completion path.
        if let Some(handle) = self
            .peer_listen_addrs
            .iter()
            .filter(|(_, recorded)| **recorded == addr)
            .filter(|(peer, _)| self.token_gate_satisfied(peer))
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
            autodial,
            hello_request: None,
            superseded,
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
                        request_id,
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
                    // ack means the peer rejected our hello. Only the pending
                    // dial's own hello can fail the pending connect — a stale
                    // ack of a replaced or earlier dial's hello is ignored.
                    if !self.hello_belongs_to_pending_dial(&peer, &request_id) {
                        return;
                    }
                    self.fail_pending_connect(
                        None,
                        Some(peer),
                        ConnectError::HandshakeFailed {
                            reason: format!("hello rejected by peer {peer}"),
                        },
                    );
                }
            }
            request_response::Event::OutboundFailure {
                peer,
                connection_id: _connection_id,
                request_id,
                error,
                ..
            } => {
                // Our outbound hello failed: the peer dropped the stream
                // (rejected our hello), timed out, or the connection died. A
                // failure of a replaced or earlier dial's hello never fails
                // the current pending connect.
                if !self.hello_belongs_to_pending_dial(&peer, &request_id) {
                    return;
                }
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

    /// The capability-token auth exchange: inbound challenges are answered
    /// from the configured token provider; outbound challenge responses are
    /// validated and mark the session token-authorized on success.
    fn handle_auth_event(
        &mut self,
        event: request_response::Event<ConnectAuthChallenge, ConnectAuthResponse>,
    ) {
        match event {
            request_response::Event::Message {
                peer,
                message:
                    request_response::Message::Request {
                        request_id: _request_id,
                        request,
                        channel,
                    },
                ..
            } => self.handle_inbound_challenge(peer, request, channel),
            request_response::Event::Message {
                peer,
                message:
                    request_response::Message::Response {
                        request_id: _request_id,
                        response,
                    },
                ..
            } => self.handle_challenge_response(peer, response),
            request_response::Event::OutboundFailure { peer, .. } => {
                // The peer did not answer our challenge (unknown method, no
                // token provider, or a dropped stream). The session stays
                // unauthorized for invokes — fail closed, per the spec's
                // "kept established but restricted" reference behavior. The
                // challenge slot is one-shot per peer, so the failure
                // consumes it exactly like the response path does.
                self.pending_challenges.remove(&peer);
            }
            request_response::Event::InboundFailure { .. }
            | request_response::Event::ResponseSent { .. } => {}
        }
    }

    /// Record mDNS discovery events as dial candidates.
    ///
    /// `Discovered` entries are appended to the internal candidate list
    /// (deduplicated, bounded by [`MAX_MDNS_CANDIDATES`]); a re-emitted
    /// `(peer, addr)` pair resets its attempted flag so the slot-free drain
    /// gives the peer another auto-dial chance. When
    /// [`ConnectConfig::mdns_autodial`] is set, the first eligible
    /// allowlisted candidate is dialed through the **same** single-flight
    /// pending-connect machinery as an explicit `connect(addr)`: there is no
    /// separate "trusted because discovered" branch (AD-P3-4). The
    /// allowlist pre-filter here only spares the single-flight slot — the
    /// trust anchor stays the `ConnectionEstablished` allowlist and
    /// signed-hello (`noise-peerid`) gates. Candidates that cannot dial yet
    /// (slot busy) stay un-attempted and are dialed when the slot frees. The
    /// auto-dial carries a dropped reply sender — the caller-less dial still
    /// binds the pending entry, passes the `ConnectionEstablished` allowlist
    /// gate, and creates the session like any user-initiated connect;
    /// completion or failure simply resolves to nothing. An explicit
    /// `connect` replaces an in-flight auto-dial instead of being rejected.
    ///
    /// `Expired` entries are removed from the candidate list (TTL
    /// bookkeeping mirroring the behaviour's own expiry).
    #[cfg(feature = "mdns")]
    fn handle_mdns_event(&mut self, event: mdns::Event) {
        match event {
            mdns::Event::Discovered(list) => {
                for (peer, addr) in list {
                    self.record_mdns_candidate(peer, addr);
                }
                // Attempt the first eligible auto-dial now; candidates that
                // cannot dial (slot busy) stay un-attempted and are picked
                // up by `maybe_autodial_next` when the slot frees.
                self.maybe_autodial_next();
            }
            mdns::Event::Expired(list) => {
                self.mdns_candidates.retain(|(peer, addr, _)| {
                    !list.iter().any(|(p, a)| *p == *peer && *a == *addr)
                });
            }
        }
    }

    /// Record (or refresh) one mDNS dial candidate.
    ///
    /// New `(peer, addr)` pairs are appended (deduplicated, bounded by
    /// [`MAX_MDNS_CANDIDATES`]); a re-emitted pair resets its attempted flag
    /// so a later drain can retry it. Recording never dials — eligibility is
    /// decided by [`EventLoop::maybe_autodial_next`].
    #[cfg(feature = "mdns")]
    fn record_mdns_candidate(&mut self, peer: PeerId, addr: Multiaddr) {
        let existing =
            self.mdns_candidates
                .iter()
                .position(|(candidate_peer, candidate_addr, _)| {
                    *candidate_peer == peer && *candidate_addr == addr
                });
        match existing {
            Some(index) => {
                // TTL re-emission: reset the attempted flag — the peer gets
                // another auto-dial chance (even if a previous dial failed).
                self.mdns_candidates[index].2 = false;
            }
            None => {
                if self.mdns_candidates.len() >= MAX_MDNS_CANDIDATES {
                    // mDNS is unauthenticated: drop new candidates beyond
                    // the cap instead of unbounded growth; dials stay
                    // fail-closed (nothing is dialed here either).
                    return;
                }
                self.mdns_candidates.push((peer, addr, false));
            }
        }
    }

    /// Reset the attempted flag of the candidate recorded for `addr`, if
    /// any. Used when an explicit `connect` replaces an in-flight auto-dial,
    /// so the replaced candidate is retried once the slot frees.
    #[cfg(feature = "mdns")]
    fn reset_mdns_candidate(&mut self, addr: &Multiaddr) {
        if let Some(entry) = self
            .mdns_candidates
            .iter_mut()
            .find(|(_, candidate_addr, _)| candidate_addr == addr)
        {
            entry.2 = false;
        }
    }

    /// Dial the next eligible auto-dial candidate, if any.
    ///
    /// Runs when the single-flight slot is free (after a pending connect
    /// resolves — success, failure, or timeout — and at the end of a
    /// `Discovered` burst). Eligibility: `mdns_autodial` enabled, and a
    /// recorded candidate that is allowlisted, not yet attempted, and
    /// without a live session. The dial goes through the same
    /// [`EventLoop::handle_connect`] machinery as an explicit connect;
    /// best-effort — a synchronous dial failure simply moves the loop to the
    /// next candidate (each iteration marks one candidate attempted, so the
    /// loop is bounded by the candidate cap).
    #[cfg(feature = "mdns")]
    fn maybe_autodial_next(&mut self) {
        if !self.config.mdns_autodial || self.pending_connect.is_some() {
            return;
        }
        while self.pending_connect.is_none() {
            let Some(index) = self
                .mdns_candidates
                .iter()
                .position(|(peer, _addr, attempted)| {
                    !*attempted
                        && is_allowlisted(&self.config.peer_allowlist, peer)
                        && !self.sessions.contains_key(peer)
                })
            else {
                return;
            };
            let (_, addr, _) = self.mdns_candidates[index].clone();
            self.mdns_candidates[index].2 = true;
            let (reply, receiver) = oneshot::channel::<Result<Arc<SessionHandle>, ConnectError>>();
            drop(receiver);
            self.handle_connect(addr, reply, true);
            // Loop while the dial did not claim the slot (already-connected
            // fast path or a synchronous dial failure): the next candidate
            // gets its chance.
        }
    }

    /// Drain the recorded mDNS dial candidates.
    ///
    /// Unit-test hook (no live multicast required): feeding a fabricated
    /// `Discovered` event to [`EventLoop::handle_event`] exercises the
    /// `Discovered` → candidate mapping deterministically.
    #[cfg(feature = "mdns")]
    #[cfg_attr(not(test), allow(dead_code))] // test hook; no lib callers yet
    fn take_mdns_discoveries(&mut self) -> Vec<(PeerId, Multiaddr)> {
        std::mem::take(&mut self.mdns_candidates)
            .into_iter()
            .map(|(peer, addr, _attempted)| (peer, addr))
            .collect()
    }

    /// Answer a received `capability-token` challenge with this node's proof.
    ///
    /// An unknown method or a challenge below the nonce floor cannot be
    /// answered: the channel is dropped and the challenger observes an
    /// outbound failure (its session stays unauthorized for invokes).
    fn handle_inbound_challenge(
        &mut self,
        peer: PeerId,
        challenge: ConnectAuthChallenge,
        channel: ResponseChannel<ConnectAuthResponse>,
    ) {
        if challenge.method.as_str() != METHOD_CAPABILITY_TOKEN {
            drop(channel);
            return;
        }
        if challenge.challenge.as_str().len() < TOKEN_CHALLENGE_MIN_LENGTH {
            drop(channel);
            return;
        }
        let Some(provider) = &self.config.capability_token_provider else {
            drop(channel);
            return;
        };
        match provider(&peer.to_string()) {
            Ok(proof) => {
                let response = ConnectAuthResponse {
                    // Echo the challenge's correlation id and method.
                    challenge_id: challenge
                        .challenge_id
                        .as_str()
                        .parse()
                        .expect("echoed challenge id parses"),
                    method: METHOD_CAPABILITY_TOKEN.parse().expect("method name parses"),
                    proof,
                    extensions: Default::default(),
                };
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .auth
                    .send_response(channel, response);
            }
            // The provider could not produce a proof (e.g. no token for this
            // audience): same fail-closed path as no provider.
            Err(_) => drop(channel),
        }
    }

    /// Validate a challenge response and, on success, mark the session
    /// token-authorized (`capability_token_ok`).
    ///
    /// Checks, in order: the response must be for this node's outstanding
    /// challenge (challenge_id echo), the method must be `capability-token`
    /// (anything else is `auth_failed` — the session stays unauthorized),
    /// and the proof must validate against this node's trust config. A
    /// rejected or stale response consumes the challenge slot: the session
    /// remains unauthorized for invokes (fail closed).
    fn handle_challenge_response(&mut self, peer: PeerId, response: ConnectAuthResponse) {
        let Some(pending) = self.pending_challenges.get(&peer).cloned() else {
            return; // unsolicited response: ignore
        };
        if response.challenge_id.as_str() != pending.challenge_id {
            return; // stale or mismatched challenge slot: ignore
        }
        self.pending_challenges.remove(&peer);
        if response.method.as_str() != METHOD_CAPABILITY_TOKEN {
            // Unknown method on the challenge response ⇒ auth_failed: the
            // session stays unauthorized for invokes.
            return;
        }
        let Ok(grant) = self.validate_token_proof(&response.proof, &peer) else {
            // Invalid token: the session stays unauthorized for invokes.
            return;
        };
        let Some(session) = self.sessions.get(&peer) else {
            return;
        };
        session.mark_token_ok(grant);
        // The token gate may have been blocking the pending connect: retry
        // completion now that the session is token-authorized.
        self.maybe_complete_session(&peer);
    }

    /// Validate a capability-token proof against this node's trust
    /// configuration and the authenticated session peer.
    ///
    /// The opaque wire `proof` is deserialized into the core token type
    /// (unknown claims / wrapper keys and malformed shapes reject here) and
    /// validated with the pure core rule. Returns the token's granted
    /// capabilities for the invoke dispatch gate.
    fn validate_token_proof(
        &self,
        proof: &serde_json::Value,
        session_peer: &PeerId,
    ) -> Result<Vec<String>, CoreError> {
        let proof: CapabilityTokenProof = serde_json::from_value(proof.clone())
            .map_err(|e| CoreError::TokenInvalid(format!("malformed proof: {e}")))?;
        verify_capability_token(
            &proof,
            &self.config.trusted_issuers,
            &self.identity.public().to_peer_id().to_string(),
            &session_peer.to_string(),
            unix_now_seconds(),
        )
    }

    /// Send a fresh `capability-token` challenge to `peer` and bind it in the
    /// pending map (anti-replay for the challenge slot).
    fn send_token_challenge(&mut self, peer: &PeerId) {
        let Ok(challenge_id) = generate_request_id() else {
            return;
        };
        let Ok(nonce) = generate_nonce() else { return };
        // The generated UUID and base64url nonce always satisfy the wire
        // minLength constraints (nonce = 22 chars ≥ 16).
        let challenge = ConnectAuthChallenge {
            challenge_id: challenge_id.parse().expect("generated challenge id parses"),
            method: METHOD_CAPABILITY_TOKEN.parse().expect("method name parses"),
            challenge: nonce.parse().expect("generated nonce parses"),
            extensions: Default::default(),
        };
        self.pending_challenges.insert(
            *peer,
            PendingChallenge {
                challenge_id,
                nonce,
            },
        );
        self.swarm
            .behaviour_mut()
            .auth
            .send_request(peer, challenge);
    }

    /// Whether the capability-token gate is satisfied for `peer`'s session:
    /// always when the token policy is inactive; otherwise the session must
    /// have completed the challenge with a valid token.
    fn token_gate_satisfied(&self, peer: &PeerId) -> bool {
        if !self.config.token_policy_active() {
            return true;
        }
        self.sessions
            .get(peer)
            .is_some_and(|session| session.token_ok())
    }

    /// The capability-token gate for an inbound invoke (normative dispatch
    /// order step 2).
    ///
    /// Returns the **effective token grant** (`claims.capabilities`) when a
    /// valid token is in effect for this invoke, or the wire `auth_failed`
    /// message when the token gate rejects:
    /// - `auth` present → the proof is validated on **every** invoke (same
    ///   rules as the challenge), even when `require_capability_token` is
    ///   false;
    /// - `auth` absent and the token policy active → the session must have
    ///   completed the challenge (`capability_token_ok`), otherwise the
    ///   invoke is rejected;
    /// - `auth` absent and the policy inactive → `None` (no token gate; the
    ///   noise-peerid-only dispatch gate applies).
    fn evaluate_invoke_token_gate(
        &self,
        peer: &PeerId,
        request: &ConnectInvokeRequest,
    ) -> Result<Option<Vec<String>>, String> {
        if let Some(auth) = &request.auth {
            return self
                .validate_token_proof(auth, peer)
                .map(Some)
                .map_err(|e| format!("invalid capability token: {e}"));
        }
        let session = self.sessions.get(peer).expect("session verified above");
        if self.config.token_policy_active() && !session.token_ok() {
            return Err(
                "capability token required but this session is not token-authorized".into(),
            );
        }
        Ok(session.granted_capabilities())
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
                    // session, starting at 0 — the pure rule is
                    // `InboundSequence::advance` (core). A replayed or
                    // out-of-order sequence is answered with a wire
                    // `invalid_sequence` envelope and is never dispatched —
                    // no duplicate or reordered handler side effects.
                    let gate = {
                        let inbound = self
                            .inbound_sequences
                            .get_mut(&peer)
                            .expect("session and its inbound sequence state are created and removed together");
                        inbound.advance(request.sequence)
                    };
                    match gate {
                        Ok(_next) => {
                            // The sequence is consumed once accepted, whatever
                            // the response outcome (mirrors the outbound
                            // direction: a failed invoke still consumes its
                            // sequence).
                            // Capability-token gate (normative §Method —
                            // capability-token, dispatch order step 2): when
                            // the request carries `auth`, the proof is
                            // validated on **every** invoke (same rules as
                            // the challenge); when the token policy is
                            // active, an invoke from a session that has not
                            // completed the challenge and carries no `auth`
                            // is rejected with an `auth_failed` wire
                            // envelope — before any dispatch.
                            match self.evaluate_invoke_token_gate(&peer, &request) {
                                Err(message) => {
                                    self.error_response(&request, "auth_failed", message)
                                }
                                Ok(token_grant) => {
                                    match &self.config.invoke_handler {
                                        None => self.error_response(
                                            &request,
                                            "op_unsupported",
                                            "no invoke handler configured on this node".into(),
                                        ),
                                        Some(handler) => {
                                            // Op dispatch gate (normative MUST,
                                            // `.mstar/specs/spoke-connect.md` §Op
                                            // dispatch gate): a host that performs
                                            // op dispatch must not run an op whose
                                            // required capability is absent from the
                                            // session's `negotiated_capabilities`.
                                            // Denied ops are answered with an
                                            // `op_unsupported` wire envelope and the
                                            // handler is never invoked — no side
                                            // effects. The core table (pure
                                            // `dispatch_allowed`, fail-closed for
                                            // unknown ops) is consulted first; ops
                                            // outside the core table fall back to
                                            // the product-configured
                                            // `op_capability_requirements` map.
                                            // When a capability token is in effect
                                            // (session grant or per-invoke `auth`),
                                            // the token grant AND the negotiated
                                            // set must both allow the op —
                                            // capability-not-granted reuses the
                                            // same `op_unsupported` deny path.
                                            let session = self
                                                .sessions
                                                .get(&peer)
                                                .expect("session verified above");
                                            let required = crate::core::required_capability(
                                                request.op.as_str(),
                                            )
                                            .or_else(|| {
                                                self.config
                                                    .op_capability_requirements
                                                    .get(request.op.as_str())
                                                    .map(String::as_str)
                                            });
                                            let negotiated_allowed = required.is_some_and(
                                                |required| {
                                                    session
                                                        .negotiated_capabilities
                                                        .iter()
                                                        .any(|granted| granted == required)
                                                },
                                            );
                                            let token_allowed = token_grant
                                                .as_deref()
                                                .is_none_or(|grant| {
                                                    token_authorizes_op(required, grant)
                                                });
                                            if !negotiated_allowed || !token_allowed {
                                                self.error_response(
                                                    &request,
                                                    "op_unsupported",
                                                    format!(
                                                        "op {} requires a capability that is not granted in this session",
                                                        request.op.as_str()
                                                    ),
                                                )
                                            } else {
                                                // The handler runs synchronously on the
                                                // event loop: it must return promptly
                                                // and must not block on I/O (see
                                                // ConnectConfig::invoke_handler).
                                                // Panics are contained so a
                                                // misbehaving adapter cannot kill the
                                                // node; the invoke is answered with an
                                                // `internal_error` wire envelope.
                                                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                                    || handler(&request.op, request.payload.clone()),
                                                ));
                                                match result {
                                                    Ok(Ok(payload)) => self.sign_invoke_response(
                                                        crate::core::InvokeResponseSignInput::Success {
                                                            session_id: request.session_id.to_string(),
                                                            sequence: request.sequence,
                                                            request_id: request.request_id.to_string(),
                                                            payload,
                                                        },
                                                    ),
                                                    Ok(Err(error)) => self.sign_invoke_response(
                                                        crate::core::InvokeResponseSignInput::Error {
                                                            session_id: request.session_id.to_string(),
                                                            sequence: request.sequence,
                                                            request_id: request.request_id.to_string(),
                                                            error,
                                                        },
                                                    ),
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
                        }
                        Err(CoreInvokeError::InboundSequenceMismatch {
                            expected,
                            actual,
                        }) => self.error_response(
                            &request,
                            "invalid_sequence",
                            format!(
                                "sequence {actual} is not the next expected inbound sequence {expected} (replay or out-of-order)"
                            ),
                        ),
                        Err(_) => unreachable!("advance only reports InboundSequenceMismatch"),
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
        self.sign_invoke_response(crate::core::InvokeResponseSignInput::Error {
            session_id: request.session_id.to_string(),
            sequence: request.sequence,
            request_id: request.request_id.to_string(),
            error: ErrorEnvelope {
                code: code.into(),
                message,
                details: Default::default(),
                extensions: Default::default(),
            },
        })
    }

    /// Sign an inbound-invoke response with this node's Ed25519 identity
    /// (`spoke-connect-invoke-response-jcs-v1` — v2 requires the
    /// `signature` on every response branch).
    ///
    /// Signing is infallible for a valid 32-byte Ed25519 seed (the only
    /// failure mode is a wrong-length secret), and a session — hence an
    /// inbound invoke — can only exist after a successful Ed25519 hello
    /// exchange, so `ed25519_seed` cannot fail here. A failure would mean
    /// the node's configured identity changed shape mid-flight; failing
    /// loudly beats emitting an unauthenticated v2 envelope.
    fn sign_invoke_response(
        &self,
        input: crate::core::InvokeResponseSignInput,
    ) -> ConnectInvokeResponse {
        let seed = crate::hello::ed25519_seed(&self.identity)
            .expect("connect identity is Ed25519 (validated at the hello exchange)");
        crate::core::authenticate_invoke_response(&seed, &input, HashMap::new())
            .expect("authenticate_invoke_response is infallible for a valid 32-byte seed")
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
                // handle — but only once the token gate is satisfied (the
                // challenge exchange is a property of the session, not of
                // any one connection). The surplus connection the dial just
                // established is closed only when it is not the session's
                // own connection: when the capability-token gate delayed the
                // pending connect past session creation, the "existing"
                // session lives on the pending dial's connection, and
                // closing it would kill the session.
                if !self.token_gate_satisfied(peer) {
                    return;
                }
                let session_connection = self.session_connections.get(peer).copied();
                if let Some(pending) = self.pending_connect.take() {
                    let surplus = pending
                        .connection
                        .is_some_and(|connection| session_connection != Some(connection));
                    if surplus {
                        if let Some(connection) = pending.connection {
                            let _ = self.swarm.close_connection(connection);
                        }
                    }
                    let _ = pending.reply.send(Ok(existing.clone()));
                    // The single-flight slot is free again: give the next
                    // un-attempted auto-dial candidate its chance.
                    #[cfg(feature = "mdns")]
                    self.maybe_autodial_next();
                }
            }
            return;
        }
        let Some(remote_manifest) = handshake.remote_manifest.clone() else {
            return;
        };
        // Negotiated capabilities = intersection of both hosts' manifests
        // (normative rule, `.mstar/specs/spoke-connect.md` §Negotiation),
        // evaluated once at session establishment. Iterating the local
        // capabilities and keeping those the remote also declared makes the
        // result deterministic in local manifest order.
        let negotiated_capabilities = self
            .config
            .local_manifest
            .capabilities
            .iter()
            .filter(|cap| {
                remote_manifest
                    .capabilities
                    .iter()
                    .any(|remote| remote == *cap)
            })
            .cloned()
            .collect();
        let Ok(session_id) = generate_request_id() else {
            self.fail_pending_connect(
                None,
                Some(*peer),
                ConnectError::Transport("CSPRNG failure while generating session id".into()),
            );
            return;
        };
        // The outbound invoke authenticator is this node's hello identity
        // seed (the peer verified it at the hello exchange; envelope-auth
        // signs every post-hello envelope with the same key). A session
        // can only exist after a successful Ed25519 hello, so the identity
        // is always Ed25519 here.
        let local_secret = crate::hello::ed25519_seed(&self.identity)
            .expect("connect identity is Ed25519 (validated at the hello exchange)");
        let handle = Arc::new(SessionHandle::new(
            session_id,
            *peer,
            remote_manifest,
            negotiated_capabilities,
            local_secret,
            self.config.effective_handshake_timeout(),
            self.cmd_tx.clone(),
        ));
        self.sessions.insert(*peer, handle.clone());
        // The receiver-side inbound expectation starts at 0 with the session
        // (lockstep with `sessions`; removed together on connection close).
        self.inbound_sequences.insert(*peer, InboundSequence::new());
        // Dialer side: record the connection the session was established
        // through (the duplicate-dial completion path uses it to tell a
        // surplus dial connection apart from the session's own connection).
        if is_pending_peer {
            if let Some(connection) = self
                .pending_connect
                .as_ref()
                .and_then(|pending| pending.connection)
            {
                self.session_connections.insert(*peer, connection);
            }
        }
        // Capability-token step-up (normative §Challenge / response and
        // invoke `auth`): when the token policy is active, the session is
        // offered the challenge right after establishment — the peer's
        // answer authorizes invokes. Until then (or if the peer cannot
        // answer), invokes from this peer are rejected with `auth_failed`.
        if self.config.token_policy_active() {
            self.send_token_challenge(peer);
        }
        if is_pending_peer {
            if !self.token_gate_satisfied(peer) {
                return;
            }
            if let Some(pending) = self.pending_connect.take() {
                self.peer_listen_addrs.insert(*peer, pending.addr.clone());
                let _ = pending.reply.send(Ok(handle));
                // The single-flight slot is free again: give the next
                // un-attempted auto-dial candidate its chance.
                #[cfg(feature = "mdns")]
                self.maybe_autodial_next();
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
                // The single-flight slot is free again: give the next
                // un-attempted auto-dial candidate its chance.
                #[cfg(feature = "mdns")]
                self.maybe_autodial_next();
            }
        }
    }

    /// Whether a hello response/failure for `peer` with `request_id` belongs
    /// to the current pending dial.
    ///
    /// Only the pending dial's own outbound hello (recorded in
    /// [`PendingConnect::hello_request`] at send time) may complete or fail
    /// it: anything else is a late event of an earlier dial — e.g. the
    /// auto-dial an explicit connect replaced, whose connection was closed
    /// at preemption but whose in-flight hello request can still report a
    /// response or failure.
    fn hello_belongs_to_pending_dial(&self, peer: &PeerId, request_id: &OutboundRequestId) -> bool {
        self.pending_connect.as_ref().is_some_and(|pending| {
            pending.peer == Some(*peer) && pending.hello_request == Some(*request_id)
        })
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
        // The single-flight slot is free again: give the next un-attempted
        // auto-dial candidate its chance.
        #[cfg(feature = "mdns")]
        self.maybe_autodial_next();
    }

    fn send_hello(&mut self, peer: &PeerId) {
        // CSPRNG or signing failures are not recoverable inside the event
        // loop; skip the send (the peer will simply not see a hello).
        let Ok(nonce) = generate_nonce() else { return };
        let Ok(hello) = sign_hello(&self.identity, &nonce, &self.config.local_manifest, None) else {
            return;
        };
        let request_id = self.swarm.behaviour_mut().hello.send_request(peer, hello);
        // Attribute the outbound hello to the pending dial it belongs to:
        // the dial's own hello is the first one sent for its peer after the
        // dial bound its connection, so a late response or failure for a
        // different hello — e.g. the auto-dial an explicit connect replaced
        // — never resolves this dial.
        if let Some(pending) = self.pending_connect.as_mut() {
            if pending.peer == Some(*peer) && pending.hello_request.is_none() {
                pending.hello_request = Some(request_id);
            }
        }
    }
}

/// Wire-range gate for an inbound invoke's `sequence`.
///
/// Required because the generated `ConnectInvokeRequest.sequence` is a bare
/// `i64` — typify does not enforce the schema's `minimum: 0` / JSON-safe
/// maximum, so this check is what keeps out-of-range values off the wire
/// path. `core::InboundSequence` would also reject them, but only with a
/// generic `InboundSequenceMismatch`; this gate lets the wire path answer
/// with the distinct `invalid_sequence` envelope per spec §Ordering
/// semantics.
fn inbound_sequence_valid(sequence: i64) -> bool {
    sequence >= 0 && (sequence as u64) <= MAX_SEQUENCE
}

/// Current Unix time in seconds (UTC), for capability-token expiry checks.
fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
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

        let swarm = build_swarm(&config.identity, &config)?;

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
            session_connections: HashMap::new(),
            pending_invokes: HashMap::new(),
            pending_challenges: HashMap::new(),
            #[cfg(feature = "mdns")]
            mdns_candidates: Vec::new(),
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
            op_capability_requirements: HashMap::new(),
            trusted_issuers: Vec::new(),
            require_capability_token: false,
            capability_token_provider: None,
            #[cfg(feature = "mdns")]
            mdns_autodial: true,
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
        // The transport composes the wire-range check with the pure
        // `InboundSequence` rule: sequential 0, 1, 2 advance the expectation
        // (the wire path accepts them; covered end-to-end by the two-node
        // integration tests).
        let mut inbound = InboundSequence::new();
        for sequence in 0..=2 {
            inbound
                .advance(sequence)
                .expect("sequential inbound sequences are accepted");
        }
        assert_eq!(inbound.next_expected(), 3);
        // A replayed sequence (already consumed) is rejected.
        assert!(matches!(
            inbound.advance(1),
            Err(CoreInvokeError::InboundSequenceMismatch { expected: 3, .. })
        ));
        // An out-of-order sequence (skipping ahead) is rejected.
        assert!(matches!(
            inbound.advance(4),
            Err(CoreInvokeError::InboundSequenceMismatch {
                expected: 3,
                actual: 4
            })
        ));
        // Negative sequences never advance (defense in depth; the range
        // check rejects them first on the wire path).
        assert!(matches!(
            inbound.advance(-1),
            Err(CoreInvokeError::InboundSequenceMismatch { .. })
        ));
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

    /// Config gating (AD-P3-3/AD-P3-5): the `mdns` feature is non-default.
    /// This module compiles only in default builds — there is no
    /// `mdns_autodial` config field and no mdns behaviour member to
    /// reference (touching them here would not compile). Tripwire
    /// limitation: if `mdns` became a default feature, this module would
    /// disappear from the default suite without failing — the CI mdns test
    /// step asserts `default = []` in `Cargo.toml`, and the feature-on
    /// `mdns_tests` module asserts the other side of the gate.
    #[cfg(not(feature = "mdns"))]
    mod no_mdns_tests {
        use super::*;

        #[test]
        fn default_build_has_no_mdns_surface() {
            assert!(
                !cfg!(feature = "mdns"),
                "mdns must stay a non-default feature"
            );
            // The default facade is unchanged: the config builds without any
            // mdns field and the node machinery is the explicit-peering
            // baseline.
            let cfg = config(Keypair::generate_ed25519(), Vec::new());
            assert!(cfg.peer_allowlist.is_empty());
        }
    }

    /// Deterministic mDNS wiring tests (AD-P3-5): no real multicast, no
    /// timing assertions. The behaviour and the event loop construct
    /// without network I/O (multicast queries only start once the behaviour
    /// is polled with an interface-up event), so fabricated `Discovered` /
    /// `Expired` events can be fed to `handle_event` directly.
    #[cfg(feature = "mdns")]
    mod mdns_tests {
        use super::*;

        fn event_loop(identity: Keypair, cfg: ConnectConfig) -> EventLoop {
            let (listen_tx, _listen_rx) = mpsc::channel(16);
            let (cmd_tx, cmd_rx) = mpsc::channel(32);
            let swarm = build_swarm(&identity, &cfg).expect("swarm builds without network I/O");
            EventLoop {
                swarm,
                identity,
                config: cfg,
                remote_keys: HashMap::new(),
                pending_hellos: HashMap::new(),
                nonces: NonceStore::new(),
                listen_tx,
                cmd_rx,
                cmd_tx,
                pending_connect: None,
                handshakes: HashMap::new(),
                sessions: HashMap::new(),
                inbound_sequences: HashMap::new(),
                peer_listen_addrs: HashMap::new(),
                session_connections: HashMap::new(),
                pending_invokes: HashMap::new(),
                pending_challenges: HashMap::new(),
                mdns_candidates: Vec::new(),
            }
        }

        fn discovered_event(peer: PeerId, addr: Multiaddr) -> SwarmEvent<ConnectBehaviourEvent> {
            SwarmEvent::Behaviour(ConnectBehaviourEvent::Mdns(mdns::Event::Discovered(vec![
                (peer, addr),
            ])))
        }

        fn established_event(peer: PeerId, addr: Multiaddr) -> SwarmEvent<ConnectBehaviourEvent> {
            established_event_id(peer, addr, ConnectionId::new_unchecked(1))
        }

        fn established_event_id(
            peer: PeerId,
            addr: Multiaddr,
            connection_id: ConnectionId,
        ) -> SwarmEvent<ConnectBehaviourEvent> {
            // A fabricated transport-level success for the auto-dial: the
            // noise-authenticated connection is up. Fully deterministic — no
            // sockets, no polling; `handle_event` processes it like any swarm
            // event.
            SwarmEvent::ConnectionEstablished {
                peer_id: peer,
                connection_id,
                endpoint: libp2p::core::connection::ConnectedPoint::Dialer {
                    address: addr,
                    role_override: libp2p::core::connection::Endpoint::Dialer,
                    port_use: libp2p::core::transport::PortUse::New,
                },
                num_established: std::num::NonZeroU32::new(1).expect("non-zero"),
                concurrent_dial_errors: None,
                established_in: Duration::ZERO,
            }
        }

        #[tokio::test]
        async fn mdns_behaviour_constructs_without_live_multicast() {
            // AD-P3-5(a): the feature-gated wiring compiles and `new` succeeds
            // without any multicast traffic (construction only creates the
            // interface watcher; queries start on poll).
            let identity = Keypair::generate_ed25519();
            let behaviour = ConnectBehaviour::new(&identity, &config(identity.clone(), Vec::new()))
                .expect("mdns behaviour construction succeeds");
            let _ = behaviour;
        }

        #[tokio::test]
        async fn mdns_discovered_event_records_dial_candidates() {
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr)]);
        }

        #[tokio::test]
        async fn mdns_repeated_discovery_is_deduplicated() {
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr)]);
        }

        #[tokio::test]
        async fn mdns_expired_event_removes_candidates() {
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            loop_.handle_event(SwarmEvent::Behaviour(ConnectBehaviourEvent::Mdns(
                mdns::Event::Expired(vec![(peer, addr.clone())]),
            )));
            assert!(
                loop_.take_mdns_discoveries().is_empty(),
                "expired candidates are dropped"
            );
        }

        #[tokio::test]
        async fn mdns_candidate_store_is_bounded() {
            // Forged-discovery flood safety (AD-P3-6): mDNS is unauthenticated,
            // so entries beyond the cap are dropped instead of growing the store
            // without bound. Deterministic — no multicast, no dials (empty
            // allowlist).
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            for i in 0..(MAX_MDNS_CANDIDATES + 32) {
                let addr: Multiaddr = format!("/ip4/192.168.1.42/tcp/{}", 40_000 + i)
                    .parse()
                    .expect("multiaddr");
                loop_.handle_event(discovered_event(peer, addr));
            }
            let candidates = loop_.take_mdns_discoveries();
            assert_eq!(candidates.len(), MAX_MDNS_CANDIDATES);
            assert_eq!(
                candidates.first().map(|(_, addr)| addr.to_string()),
                Some("/ip4/192.168.1.42/tcp/40000".to_owned()),
                "earliest discoveries are kept, later ones are dropped"
            );
        }

        #[tokio::test]
        async fn mdns_take_drains_the_candidate_store() {
            // Pure helper semantics (AD-P3-3): `take_mdns_discoveries` drains
            // the store — taking on an empty store is empty, a second take
            // after a first is empty (nothing re-appears), and discoveries
            // made after a drain are recorded again (dedupe runs against the
            // live store, not against history).
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            assert!(loop_.take_mdns_discoveries().is_empty());

            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr.clone())]);
            assert!(
                loop_.take_mdns_discoveries().is_empty(),
                "drained: a second take returns nothing"
            );

            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert_eq!(
                loop_.take_mdns_discoveries(),
                vec![(peer, addr)],
                "post-drain discoveries are recorded again"
            );
        }

        #[tokio::test]
        async fn mdns_candidate_cap_resets_after_drain() {
            // Cap interplay: `MAX_MDNS_CANDIDATES` bounds the live store, not
            // cumulative discoveries — a drain resets the accounting, so a
            // post-drain discovery is accepted again (and a fresh flood would
            // be capped anew).
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            for i in 0..MAX_MDNS_CANDIDATES {
                let addr: Multiaddr = format!("/ip4/192.168.1.42/tcp/{}", 40_000 + i)
                    .parse()
                    .expect("multiaddr");
                loop_.handle_event(discovered_event(peer, addr));
            }
            assert_eq!(loop_.take_mdns_discoveries().len(), MAX_MDNS_CANDIDATES);

            // After the drain the store is empty and accepts new candidates.
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/50000".parse().expect("multiaddr");
            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr)]);
        }

        #[tokio::test]
        async fn mdns_autodial_skips_non_allowlisted_discoveries() {
            // AD-P3-4 + AD-P3-5(b): an empty allowlist is fail-closed — the
            // discovered peer is recorded as a candidate but never dialed (no
            // pending dial is created). mDNS is discovery only, never a trust
            // grant; the allowlist gate itself is exercised end-to-end by
            // `mdns_autodial_dials_allowlisted_discoveries_through_connect_machinery`.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, Vec::new()));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert!(
                loop_.pending_connect.is_none(),
                "non-allowlisted discovery must not start a dial"
            );
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr)]);
        }

        #[tokio::test]
        async fn mdns_autodial_dials_allowlisted_discoveries_through_connect_machinery() {
            // The auto-dial reuses the same single-flight machinery as an
            // explicit `connect(addr)`: a pending dial bound to the discovered
            // address is created (no parallel trusted branch). A tokio runtime
            // is required because `swarm.dial` synchronously spawns the dial
            // task through the connection pool's executor (`tokio::spawn`);
            // no poll or socket I/O happens, so the test stays deterministic.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            let pending = loop_.pending_connect.as_ref().expect("auto-dial pending");
            assert_eq!(pending.addr, addr);
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr)]);
        }

        #[tokio::test]
        async fn mdns_autodial_allowlisted_peer_is_not_admitted_without_signed_hello() {
            // AD-P3-4: mDNS grants addresses, never trust. An allowlisted
            // discovered peer is auto-dialed through the shared connect
            // machinery, but admission still requires the signed-hello
            // exchange: even after the connection is established, no session
            // exists until `gate_hello` accepts the peer's hello. (The hello
            // gate itself is exercised end-to-end by the two-node tests.)
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));
            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert!(
                loop_.pending_connect.is_some(),
                "allowlisted discovery auto-dials"
            );

            // The transport layer "succeeds": the auto-dial's connection is
            // established. The node binds the pending dial, passes the
            // allowlist gate, and starts the hello exchange — but the peer
            // never returns a valid signed hello, so no session is admitted.
            loop_.handle_event(established_event(peer, addr));
            let pending = loop_.pending_connect.as_ref().expect("dial still pending");
            assert_eq!(
                pending.peer,
                Some(peer),
                "dial bound to the discovered peer"
            );
            assert_eq!(
                pending.connection,
                Some(ConnectionId::new_unchecked(1)),
                "dial bound through the shared connect machinery"
            );
            assert!(
                loop_.sessions.is_empty(),
                "no session before a valid signed hello exchange"
            );
        }

        #[tokio::test]
        async fn mdns_autodial_off_records_candidates_without_dialing() {
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut cfg = config(identity.clone(), vec![peer]);
            cfg.mdns_autodial = false;
            let mut loop_ = event_loop(identity, cfg);
            loop_.handle_event(discovered_event(peer, addr.clone()));
            assert!(loop_.pending_connect.is_none(), "autodial disabled");
            assert_eq!(loop_.take_mdns_discoveries(), vec![(peer, addr)]);
        }

        #[tokio::test]
        async fn mdns_explicit_connect_preempts_inflight_autodial() {
            // F-1: an explicit connect replaces an in-flight auto-dial
            // instead of being rejected with "already in progress". The
            // replaced dial's candidate is reset, so the slot-free drain
            // retries it once the explicit dial resolves.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let auto_addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let explicit_addr: Multiaddr = "/ip4/10.0.0.9/tcp/9090".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));

            loop_.handle_event(discovered_event(peer, auto_addr.clone()));
            assert!(
                loop_.pending_connect.as_ref().is_some_and(|p| p.autodial),
                "auto-dial holds the slot"
            );

            let (reply, receiver) = oneshot::channel();
            loop_.handle_connect(explicit_addr.clone(), reply, false);
            let pending = loop_
                .pending_connect
                .as_ref()
                .expect("explicit dial pending");
            assert!(!pending.autodial, "explicit connect replaced the auto-dial");
            assert_eq!(pending.addr, explicit_addr);

            // The explicit dial fails: the slot frees and the replaced
            // auto-dial candidate is retried through the drain.
            loop_.handle_event(established_event(peer, explicit_addr.clone()));
            loop_.fail_pending_connect(
                None,
                Some(peer),
                ConnectError::HandshakeFailed {
                    reason: "test failure".into(),
                },
            );
            let retried = loop_.pending_connect.as_ref().expect("candidate retried");
            assert!(retried.autodial, "retried dial is an auto-dial");
            assert_eq!(retried.addr, auto_addr);
            assert!(
                matches!(receiver.await, Ok(Err(_))),
                "the explicit connect reports its own failure"
            );
        }

        #[tokio::test]
        async fn mdns_preempted_autodial_connection_is_not_bound_to_explicit_dial() {
            // G-1 (preemption misbind): when an explicit connect replaces an
            // in-flight auto-dial BEFORE the auto-dial's
            // `ConnectionEstablished` is processed, the late event must not
            // bind the replaced dial's connection + peer to the explicit
            // pending entry — the slot belongs to the explicit dial's own
            // connection. The stale connection is closed instead.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let auto_addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let explicit_addr: Multiaddr = "/ip4/10.0.0.9/tcp/9090".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));

            loop_.handle_event(discovered_event(peer, auto_addr.clone()));
            let (reply, mut receiver) = oneshot::channel();
            loop_.handle_connect(explicit_addr.clone(), reply, false);
            let pending = loop_
                .pending_connect
                .as_ref()
                .expect("explicit dial pending");
            assert_eq!(pending.addr, explicit_addr);
            assert!(
                pending.superseded.is_some(),
                "the replaced auto-dial is recorded on the replacement"
            );

            // The replaced auto-dial's connection establishes late: it is
            // closed, not bound — the explicit dial stays intact.
            loop_.handle_event(established_event(peer, auto_addr));
            let pending = loop_
                .pending_connect
                .as_ref()
                .expect("explicit dial still pending");
            assert!(
                pending.connection.is_none(),
                "stale connection is not bound"
            );
            assert!(pending.peer.is_none(), "stale peer is not bound");
            assert!(
                matches!(
                    receiver.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ),
                "explicit connect is not resolved by the stale connection"
            );

            // The explicit dial's own connection binds normally.
            loop_.handle_event(established_event_id(
                peer,
                explicit_addr.clone(),
                ConnectionId::new_unchecked(2),
            ));
            let pending = loop_.pending_connect.as_ref().expect("explicit dial bound");
            assert_eq!(pending.connection, Some(ConnectionId::new_unchecked(2)));
            assert_eq!(pending.peer, Some(peer));
        }

        #[tokio::test]
        async fn mdns_preempted_autodial_close_does_not_fail_explicit_connect() {
            // G-2 (failure side): the replaced auto-dial's `ConnectionClosed`
            // (following the close issued at preemption) must not fail the
            // explicit pending connect — the slot survives and still binds
            // the explicit dial's own connection afterwards.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let auto_addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let explicit_addr: Multiaddr = "/ip4/10.0.0.9/tcp/9090".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));

            // The auto-dial establishes and binds to its own connection
            // first (its hello request is recorded on the replaced entry).
            loop_.handle_event(discovered_event(peer, auto_addr.clone()));
            loop_.handle_event(established_event(peer, auto_addr.clone()));
            assert_eq!(
                loop_
                    .pending_connect
                    .as_ref()
                    .expect("auto-dial bound")
                    .connection,
                Some(ConnectionId::new_unchecked(1))
            );

            // Explicit connect replaces it; the replaced connection is
            // closed and its identity recorded on the replacement.
            let (reply, mut receiver) = oneshot::channel();
            loop_.handle_connect(explicit_addr.clone(), reply, false);
            let pending = loop_
                .pending_connect
                .as_ref()
                .expect("explicit dial pending");
            assert!(
                pending
                    .superseded
                    .as_ref()
                    .is_some_and(|sup| sup.connection == Some(ConnectionId::new_unchecked(1))),
                "the replaced dial's connection id is recorded"
            );

            // The replaced connection's close event arrives late: it must
            // not fail the explicit dial, which still binds its own
            // connection afterwards.
            loop_.handle_event(SwarmEvent::ConnectionClosed {
                peer_id: peer,
                connection_id: ConnectionId::new_unchecked(1),
                endpoint: libp2p::core::connection::ConnectedPoint::Dialer {
                    address: auto_addr.clone(),
                    role_override: libp2p::core::connection::Endpoint::Dialer,
                    port_use: libp2p::core::transport::PortUse::New,
                },
                num_established: 0,
                cause: None,
            });
            let pending = loop_
                .pending_connect
                .as_ref()
                .expect("explicit dial not failed");
            assert_eq!(pending.addr, explicit_addr);
            assert!(
                matches!(
                    receiver.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ),
                "the stale close does not resolve the explicit connect"
            );

            loop_.handle_event(established_event_id(
                peer,
                explicit_addr.clone(),
                ConnectionId::new_unchecked(2),
            ));
            let pending = loop_.pending_connect.as_ref().expect("explicit dial bound");
            assert_eq!(pending.connection, Some(ConnectionId::new_unchecked(2)));
        }

        #[tokio::test]
        async fn mdns_preempted_autodial_hello_failure_does_not_fail_explicit_connect() {
            // G-3 (hello failure side): a hello failure of the replaced
            // auto-dial (its connection was closed at preemption but the
            // in-flight hello request can still report a failure) must not
            // fail the explicit pending connect — only the explicit dial's
            // own hello may fail it.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let auto_addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let explicit_addr: Multiaddr = "/ip4/10.0.0.9/tcp/9090".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));

            // The auto-dial establishes (its hello request is recorded) and
            // is then replaced by the explicit connect.
            loop_.handle_event(discovered_event(peer, auto_addr.clone()));
            loop_.handle_event(established_event(peer, auto_addr.clone()));
            let auto_hello = loop_
                .pending_connect
                .as_ref()
                .and_then(|pending| pending.hello_request)
                .expect("auto-dial hello recorded");
            let (reply, mut receiver) = oneshot::channel();
            loop_.handle_connect(explicit_addr.clone(), reply, false);

            // The explicit dial establishes and sends its own hello.
            loop_.handle_event(established_event_id(
                peer,
                explicit_addr.clone(),
                ConnectionId::new_unchecked(2),
            ));
            let explicit_hello = loop_
                .pending_connect
                .as_ref()
                .and_then(|pending| pending.hello_request)
                .expect("explicit dial hello recorded");
            assert_ne!(auto_hello, explicit_hello);

            // A late failure of the replaced dial's hello is ignored…
            loop_.handle_hello_event(request_response::Event::OutboundFailure {
                peer,
                connection_id: ConnectionId::new_unchecked(1),
                request_id: auto_hello,
                error: request_response::OutboundFailure::ConnectionClosed,
            });
            let pending = loop_
                .pending_connect
                .as_ref()
                .expect("explicit dial not failed");
            assert_eq!(pending.addr, explicit_addr);
            assert!(
                matches!(
                    receiver.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ),
                "the stale hello failure does not resolve the explicit connect"
            );

            // …while the explicit dial's own hello failure fails it.
            loop_.handle_hello_event(request_response::Event::OutboundFailure {
                peer,
                connection_id: ConnectionId::new_unchecked(2),
                request_id: explicit_hello,
                error: request_response::OutboundFailure::ConnectionClosed,
            });
            assert!(
                matches!(
                    receiver.try_recv(),
                    Ok(Err(ConnectError::HandshakeFailed { .. }))
                ),
                "the explicit connect reports its own hello failure"
            );
        }

        #[tokio::test]
        async fn mdns_autodial_candidate_is_retried_when_slot_frees() {
            // F-2: a candidate discovered while the slot is busy is not
            // permanently skipped — when the first dial resolves, the
            // slot-free drain dials the next un-attempted candidate.
            let identity = Keypair::generate_ed25519();
            let peer1 = Keypair::generate_ed25519().public().to_peer_id();
            let peer2 = Keypair::generate_ed25519().public().to_peer_id();
            let addr1: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let addr2: Multiaddr = "/ip4/192.168.1.43/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer1, peer2]));

            loop_.handle_event(discovered_event(peer1, addr1.clone()));
            assert!(
                loop_
                    .pending_connect
                    .as_ref()
                    .is_some_and(|p| p.addr == addr1),
                "first discovery auto-dials"
            );

            // Second peer discovered while the slot is busy: recorded, not
            // dialed, not lost.
            loop_.handle_event(discovered_event(peer2, addr2.clone()));
            assert!(
                loop_
                    .pending_connect
                    .as_ref()
                    .is_some_and(|p| p.addr == addr1),
                "slot still busy — peer2 waits"
            );

            // First dial fails → slot frees → peer2 is dialed.
            loop_.handle_event(established_event(peer1, addr1.clone()));
            loop_.fail_pending_connect(
                None,
                Some(peer1),
                ConnectError::HandshakeFailed {
                    reason: "test failure".into(),
                },
            );
            let pending = loop_.pending_connect.as_ref().expect("peer2 auto-dialed");
            assert!(pending.autodial);
            assert_eq!(pending.addr, addr2);
            assert_eq!(
                loop_.take_mdns_discoveries(),
                vec![(peer1, addr1), (peer2, addr2)],
                "both peers were recorded as candidates"
            );
        }

        #[tokio::test]
        async fn mdns_multi_peer_autodials_are_serialized_as_slot_frees() {
            // F-3(c): allowlisted candidates are dialed one at a time, in
            // candidate order, as the single-flight slot frees.
            let identity = Keypair::generate_ed25519();
            let peer1 = Keypair::generate_ed25519().public().to_peer_id();
            let peer2 = Keypair::generate_ed25519().public().to_peer_id();
            let peer3 = Keypair::generate_ed25519().public().to_peer_id();
            let addr1: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let addr2: Multiaddr = "/ip4/192.168.1.43/tcp/4242".parse().expect("multiaddr");
            let addr3: Multiaddr = "/ip4/192.168.1.44/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(
                identity.clone(),
                config(identity, vec![peer1, peer2, peer3]),
            );

            // One burst discovers all three allowlisted peers: only the
            // first is dialed (serialized), the rest wait for the slot.
            loop_.handle_event(SwarmEvent::Behaviour(ConnectBehaviourEvent::Mdns(
                mdns::Event::Discovered(vec![
                    (peer1, addr1.clone()),
                    (peer2, addr2.clone()),
                    (peer3, addr3.clone()),
                ]),
            )));
            assert!(
                loop_
                    .pending_connect
                    .as_ref()
                    .is_some_and(|p| p.addr == addr1),
                "first candidate dialed first"
            );

            // Resolve each dial as a failure in turn: the slot frees and the
            // next candidate is dialed serially.
            loop_.handle_event(established_event(peer1, addr1.clone()));
            loop_.fail_pending_connect(
                None,
                Some(peer1),
                ConnectError::HandshakeFailed {
                    reason: "test failure".into(),
                },
            );
            assert!(
                loop_
                    .pending_connect
                    .as_ref()
                    .is_some_and(|p| p.addr == addr2),
                "second candidate dialed after the first failed"
            );

            loop_.handle_event(established_event(peer2, addr2.clone()));
            loop_.fail_pending_connect(
                None,
                Some(peer2),
                ConnectError::HandshakeFailed {
                    reason: "test failure".into(),
                },
            );
            assert!(
                loop_
                    .pending_connect
                    .as_ref()
                    .is_some_and(|p| p.addr == addr3),
                "third candidate dialed after the second failed"
            );

            loop_.handle_event(established_event(peer3, addr3.clone()));
            loop_.fail_pending_connect(
                None,
                Some(peer3),
                ConnectError::HandshakeFailed {
                    reason: "test failure".into(),
                },
            );
            assert!(
                loop_.pending_connect.is_none(),
                "all candidates attempted — no further dials"
            );
        }

        #[tokio::test]
        async fn mdns_rediscovery_resets_attempted_for_retry() {
            // F-2: TTL re-emission resets the attempted flag — a failed
            // auto-dial candidate gets another chance on re-Discovery.
            let identity = Keypair::generate_ed25519();
            let peer = Keypair::generate_ed25519().public().to_peer_id();
            let addr: Multiaddr = "/ip4/192.168.1.42/tcp/4242".parse().expect("multiaddr");
            let mut loop_ = event_loop(identity.clone(), config(identity, vec![peer]));

            loop_.handle_event(discovered_event(peer, addr.clone()));
            loop_.handle_event(established_event(peer, addr.clone()));
            loop_.fail_pending_connect(
                None,
                Some(peer),
                ConnectError::HandshakeFailed {
                    reason: "test failure".into(),
                },
            );
            assert!(
                loop_.pending_connect.is_none(),
                "failed candidate is not re-dialed by the drain (attempted)"
            );

            // Re-emission resets the attempted flag: the drain dials again.
            loop_.handle_event(discovered_event(peer, addr.clone()));
            let pending = loop_.pending_connect.as_ref().expect("re-dialed");
            assert!(pending.autodial);
            assert_eq!(pending.addr, addr);
        }
    }
}
