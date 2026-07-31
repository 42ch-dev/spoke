//! Node lifecycle: transport composition, hello exchange, accept gate.
//!
//! Composition (locked): **noise** (authenticated transport) + **yamux**
//! (multiplexing) + **request-response** (hello exchange; invoke in Task 2) +
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

use crate::config::ConnectConfig;
use crate::error::ConnectError;
use crate::gate::{gate_hello, NonceStore};
use crate::hello::{generate_nonce, sign_hello};
use crate::protocol::{HelloAck, HELLO_PROTOCOL};
use futures::StreamExt;
use libp2p::identity::{Keypair, PublicKey};
use libp2p::request_response::{self, InboundRequestId, ResponseChannel};
use libp2p::swarm::{NetworkBehaviour, Swarm, SwarmEvent};
use libp2p::SwarmBuilder;
use libp2p::{identify, noise, tcp, yamux, Multiaddr, PeerId, StreamProtocol};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::ConnectHello;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

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
}

impl ConnectBehaviour {
    fn new(keypair: &Keypair, config: &ConnectConfig) -> Self {
        let identify = identify::Behaviour::new(identify::Config::new(
            format!("spoke-connect/{}", env!("CARGO_PKG_VERSION")),
            keypair.public(),
        ));
        let hello = request_response::json::Behaviour::<ConnectHello, HelloAck>::new(
            [(
                StreamProtocol::new(HELLO_PROTOCOL),
                request_response::ProtocolSupport::Full,
            )],
            request_response::Config::default()
                .with_request_timeout(config.effective_handshake_timeout()),
        );
        Self { identify, hello }
    }
}

/// A buffered inbound hello awaiting the remote public key.
struct PendingHello {
    request_id: InboundRequestId,
    hello: ConnectHello,
    channel: ResponseChannel<HelloAck>,
    received_at: Instant,
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
}

impl EventLoop {
    async fn run(mut self, mut shutdown_rx: watch::Receiver<bool>) {
        let mut sweep = tokio::time::interval(PENDING_HELLO_SWEEP_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                _ = sweep.tick() => self.expire_pending_hellos(),
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
            SwarmEvent::ConnectionEstablished { peer_id, .. } => self.send_hello(&peer_id),
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                // Dropping buffered entries closes their channels (rejects the
                // pending hellos of a dead connection).
                self.remote_keys.remove(&peer_id);
                self.pending_hellos.remove(&peer_id);
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
            // Listener errors, dial failures, address churn, bandwidth and
            // other identify events are not part of the hello path.
            _ => {}
        }
    }

    fn handle_hello_event(&mut self, event: request_response::Event<ConnectHello, HelloAck>) {
        // Responses to our own hellos and request failures are correlated by
        // `connect` in Task 2; nothing to do on the accept path.
        if let request_response::Event::Message {
            peer,
            message:
                request_response::Message::Request {
                    request_id,
                    request,
                    channel,
                },
            ..
        } = event
        {
            self.handle_inbound_hello(peer, request_id, request, channel);
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
            // Protocol v1 defines no hello error envelope.
            Err(_reason) => drop(respond),
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
    shutdown_tx: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
                shutdown_tx,
                task: Mutex::new(Some(task)),
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
