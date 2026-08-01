//! Two-node loopback exchange — DoD integration coverage for
//! `crates/spoke-connect` (default features).
//!
//! Scenarios:
//! (a) both nodes exchange signed hellos; remote manifests round-trip intact
//!     in both directions (role reversal);
//! (b) one `check` op invoke closes the loop — sequence 0 + request_id echo;
//! (c) a second invoke uses sequence 1;
//! (d) a third node not on the allowlist is rejected at the handshake and
//!     observes no session; the allowlisted pair keeps working;
//! (e) a stub handler returning an error envelope surfaces as
//!     `InvokeError::Wire(ErrorEnvelope)`;
//! (f) a dial that stalls at the noise handshake fails deterministically at
//!     the handshake deadline, and a concurrent inbound session cannot
//!     consume the pending dial; a retry to a live peer succeeds;
//! (g) an unreachable address fails the pending dial and a retry succeeds;
//! (h) a duplicate dial to a sessioned peer completes with the existing
//!     session and the surplus connection is closed;
//! (i) when the peer shuts down, the session is observed closed and a
//!     reconnecting peer (same identity) establishes a fresh session;
//! (j) a panicking handler is contained (internal_error wire envelope) and
//!     the node stays alive;
//! (k) the inbound op dispatch gate denies an op whose required capability
//!     is absent from the session's negotiated capabilities (`op_unsupported`
//!     envelope, handler never called) while a negotiated op still flows;
//! (l) the product op→capability map (`ConnectConfig::
//!     op_capability_requirements`) authorizes product-defined ops through
//!     the dispatch gate: a mapped requirement that IS negotiated lets the
//!     handler run; one that is NOT negotiated is denied (`op_unsupported`,
//!     handler never called);
//! (m) the capability-token challenge/response flow (require flag true on
//!     both nodes): both sides complete the challenge with provider-minted
//!     tokens from a trusted issuer, the dialer's session reports
//!     `capability_token_ok`, and invokes flow without per-invoke `auth`;
//! (n) a receiver with `require_capability_token` true rejects invokes from
//!     a session that never completed the challenge when no `auth` is
//!     attached (`auth_failed` wire code); per-invoke `auth` with a valid
//!     token passes and an expired token is rejected;
//! (o) with `require_capability_token` false, no-`auth` behavior is
//!     unchanged, while an attached `auth` proof is still validated on every
//!     invoke (untrusted issuer / tampered proof → `auth_failed`);
//! (p) the token grant gates dispatch by capability membership: an op whose
//!     required capability is negotiated but absent from the token grant is
//!     denied with `op_unsupported` (handler never called); a token granting
//!     the required capability lets it through.
//!
//! All waits are bounded event waits (timeouts on `connect` / `invoke`
//! futures) — no sleep-based synchronization.

use ed25519_dalek::SigningKey;
use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId};
use spoke_connect::core::{
    derive_peer_id_from_ed25519_pubkey, issue_capability_token, CapabilityClaims,
};
use spoke_connect::{parse_multiaddr, ConnectConfig, ConnectError, InvokeError, SpokeConnectNode};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Serializes the network scenarios: every node binds a SO_REUSEPORT loopback
/// listener, and macOS's SO_REUSEPORT port allocation can collide across
/// concurrently running tests in one process (repeat dials fail with
/// EADDRINUSE / connect timeouts). Running the scenarios one at a time keeps
/// them deterministic on all platforms; each test still uses only bounded
/// event waits — no sleeps.
static NETWORK_TEST_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn network_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    NETWORK_TEST_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// Handshake / invoke timeout for the test nodes (≥ 5s keeps loopback CI
/// reliable; every wait in these tests is bounded by it).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Short handshake timeout for stall/reject tests (generous enough to stay
/// green under parallel test load on loopback; keeps the deterministic
/// deadline sweeps fast).
const SHORT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const LISTEN: &str = "/ip4/127.0.0.1/tcp/0";

fn manifest(host_id: &str, roles: &[&str]) -> HostCapabilityManifest {
    manifest_with_capabilities(host_id, roles, &["spoke-connect", "spoke-baseline"])
}

fn manifest_with_capabilities(
    host_id: &str,
    roles: &[&str],
    capabilities: &[&str],
) -> HostCapabilityManifest {
    HostCapabilityManifest {
        authority: None,
        capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
        extensions: Default::default(),
        host_id: host_id.parse().expect("host id parses"),
        namespaces: Vec::new(),
        roles: roles.iter().map(|r| (*r).to_string()).collect(),
        schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
    }
}

fn config(
    identity: Keypair,
    allowlist: Vec<PeerId>,
    local_manifest: HostCapabilityManifest,
    timeout: Duration,
) -> ConnectConfig {
    ConnectConfig {
        identity,
        peer_allowlist: allowlist,
        listen_addrs: vec![parse_multiaddr(LISTEN).expect("multiaddr")],
        local_manifest,
        handshake_timeout: Some(timeout),
        invoke_handler: None,
        op_capability_requirements: HashMap::new(),
        trusted_issuers: Vec::new(),
        require_capability_token: false,
        capability_token_provider: None,
        #[cfg(feature = "mdns")]
        mdns_autodial: true,
    }
}

async fn start(
    identity: Keypair,
    allowlist: Vec<PeerId>,
    local_manifest: HostCapabilityManifest,
) -> SpokeConnectNode {
    SpokeConnectNode::start(config(
        identity,
        allowlist,
        local_manifest,
        HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("node starts")
}

/// A realistic `CheckRequest` envelope (opaque invoke payload).
fn check_request() -> serde_json::Value {
    serde_json::json!({
        "scope": { "scope_id": "spike-scope-1", "entry_ids": ["entry-1"] },
        "checker_kinds": ["baseline"],
        "extensions": {}
    })
}

/// A realistic `CheckResponse` success envelope (findings branch).
fn check_response() -> serde_json::Value {
    serde_json::json!({
        "findings": [{
            "schema_version": 1,
            "finding_id": "finding-spike-001",
            "severity": "info",
            "status": "open",
            "title": "spike check ok",
            "description": "baseline checker ran clean",
            "extensions": {}
        }],
        "extensions": {}
    })
}

/// Unix time now + `offset` seconds, for token expiry fixtures.
fn now_plus(offset: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("post-epoch clock")
        .as_secs()
        + offset
}

/// The `peer_id` derived from an Ed25519 issuer secret (the issuer's
/// identity string, used in `trusted_issuers` and `claims.iss`).
fn issuer_peer_id(issuer_secret: &[u8; 32]) -> String {
    derive_peer_id_from_ed25519_pubkey(
        &SigningKey::from_bytes(issuer_secret)
            .verifying_key()
            .to_bytes(),
    )
}

/// Mint a capability-token proof from `issuer_secret` for `subject` to
/// present to `audience`, serialized as the wire `proof` object (`{ v,
/// claims, sig }`).
fn token_proof(
    issuer_secret: &[u8; 32],
    subject: &str,
    audience: &str,
    capabilities: &[&str],
    exp: u64,
) -> serde_json::Value {
    let proof = issue_capability_token(
        issuer_secret,
        CapabilityClaims {
            iss: issuer_peer_id(issuer_secret),
            sub: subject.to_string(),
            aud: audience.to_string(),
            capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            exp,
            iat: None,
            jti: None,
        },
    )
    .expect("issuer key derives iss");
    serde_json::to_value(&proof).expect("proof serializes")
}

/// A challenge-response token provider for `subject`: mints a token from
/// `issuer_secret` for the challenger as the audience (so the returned proof
/// carries `sub` = `subject` and `aud` = the challenger's peer id).
fn provider(
    issuer_secret: [u8; 32],
    subject: PeerId,
    capabilities: Vec<String>,
    exp: u64,
) -> Arc<spoke_connect::CapabilityTokenProvider> {
    Arc::new(move |audience: &str| {
        let proof = issue_capability_token(
            &issuer_secret,
            CapabilityClaims {
                iss: issuer_peer_id(&issuer_secret),
                sub: subject.to_string(),
                aud: audience.to_string(),
                capabilities: capabilities.clone(),
                exp,
                iat: None,
                jti: None,
            },
        )
        .map_err(|e| e.to_string())?;
        serde_json::to_value(&proof).map_err(|e| e.to_string())
    })
}

/// A TCP listener that accepts and holds connections without ever answering:
/// a dial to it completes TCP but stalls at the noise handshake, so neither
/// identify nor hello can ever arrive (the pending dial is only resolvable
/// by the event loop's deadline sweep).
fn stall_listener() -> Multiaddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stall listener");
    let port = listener.local_addr().expect("stall port").port();
    let held = Arc::new(std::sync::Mutex::new(Vec::new()));
    let held_thread = held.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => held_thread.lock().expect("held lock").push(s),
                Err(_) => break,
            }
        }
    });
    let _ = held;
    format!("/ip4/127.0.0.1/tcp/{port}")
        .parse::<Multiaddr>()
        .expect("stall multiaddr")
}

/// Wait until `session` reports its connection as closed (bounded event
/// wait; each iteration is a real network round trip, no sleeps).
async fn wait_until_closed(session: &spoke_connect::PeerSession, bound: Duration) {
    let deadline = std::time::Instant::now() + bound;
    loop {
        if session.invoke("check", check_request()).await.is_err() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session did not close within {bound:?}"
        );
        tokio::task::yield_now().await;
    }
}

/// (a) Two nodes on loopback complete the authenticated hello handshake and
/// both sides' manifests round-trip through the generated wire type — in
/// both directions: B dials A (B observes A's manifest) and A dials B (A
/// observes B's manifest, roles + capabilities included).
#[tokio::test]
async fn hello_exchange_round_trips_manifests_both_ways() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();
    let manifest_a = manifest("host-a", &["data-store", "checker"]);
    let manifest_b = manifest("host-b", &["input-source", "assembler"]);

    let node_a = start(key_a, vec![peer_b], manifest_a.clone()).await;
    let node_b = start(key_b, vec![peer_a], manifest_b.clone()).await;

    // B dials A (plan test strategy); handshake succeeds both directions.
    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    assert_eq!(session.remote_peer_id(), peer_a);
    assert!(!session.session_id().is_empty());
    assert_eq!(session.next_sequence(), 0);

    // Remote manifest arrives intact (round-trip through generated types).
    let wire = serde_json::to_value(session.remote_manifest()).expect("serialize");
    assert_eq!(wire, serde_json::to_value(&manifest_a).expect("serialize"));
    assert_eq!(wire["host_id"], serde_json::json!("host-a"));
    assert_eq!(wire["roles"], serde_json::json!(["data-store", "checker"]));
    assert_eq!(
        wire["capabilities"],
        serde_json::json!(["spoke-connect", "spoke-baseline"])
    );

    // Role reversal: A dials B and observes B's manifest through A's own
    // dialer session — the responder-side manifest is visible on both sides.
    let session_a = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("a dials b");
    assert_eq!(session_a.remote_peer_id(), peer_b);
    let wire_b = serde_json::to_value(session_a.remote_manifest()).expect("serialize");
    assert_eq!(
        wire_b,
        serde_json::to_value(&manifest_b).expect("serialize")
    );
    assert_eq!(wire_b["host_id"], serde_json::json!("host-b"));
    assert_eq!(
        wire_b["roles"],
        serde_json::json!(["input-source", "assembler"])
    );
    assert_eq!(
        wire_b["capabilities"],
        serde_json::json!(["spoke-connect", "spoke-baseline"])
    );
    // The returned session routes invokes (B has no handler: the wire
    // op_unsupported error proves the session round-trip works).
    match session_a.invoke("check", check_request()).await {
        Err(InvokeError::Wire(envelope)) => assert_eq!(envelope.code, "op_unsupported"),
        other => panic!("expected op_unsupported wire error, got {other:?}"),
    }

    node_a.shutdown().await.expect("a shuts down");
    node_b.shutdown().await.expect("b shuts down");
}

/// (b) One `check` op invoke closes the loop: A answers through a stub
/// handler returning a real check-response envelope; the caller sees
/// `sequence == 0` and the request_id echo (asserted by the correlation
/// gate; `InvokeSuccess.request_id` is the echoed id).
#[tokio::test]
async fn check_op_invoke_round_trips_with_sequence_zero() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let response = check_response();
    let handler = {
        let response = response.clone();
        Arc::new(move |op: &str, _payload: serde_json::Value| {
            assert_eq!(op, "check", "handler sees the requested op");
            Ok(response.clone())
        })
    };
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["data-store", "checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(
        key_b,
        vec![peer_a],
        manifest("host-b", &["input-source", "assembler"]),
    )
    .await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    let success = session
        .invoke("check", check_request())
        .await
        .expect("invoke succeeds");

    assert_eq!(success.sequence, 0, "first invoke uses sequence 0");
    assert!(!success.request_id.is_empty(), "request_id is generated");
    assert_eq!(
        success.payload, response,
        "stub handler payload arrives intact"
    );
    assert_eq!(session.next_sequence(), 1);

    node_a.shutdown().await.expect("a shuts down");
    node_b.shutdown().await.expect("b shuts down");
}

/// (c) A second invoke on the same session uses sequence 1; request ids are
/// unique per invoke.
#[tokio::test]
async fn second_invoke_uses_sequence_one() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    let first = session
        .invoke("check", check_request())
        .await
        .expect("first invoke");
    let second = session
        .invoke("check", check_request())
        .await
        .expect("second invoke");

    assert_eq!(first.sequence, 0);
    assert_eq!(second.sequence, 1, "second invoke increments the sequence");
    assert_ne!(
        first.request_id, second.request_id,
        "fresh request id per invoke"
    );
    assert_eq!(session.next_sequence(), 2);

    node_a.shutdown().await.expect("a shuts down");
    node_b.shutdown().await.expect("b shuts down");
}

/// (d) A third node not on the allowlist is rejected at the handshake — no
/// session is established, and the allowlisted pair keeps working afterwards.
/// The early allowlist gate (at `ConnectionEstablished`) makes the rejection
/// deterministic from the dialer's perspective: C's own gate rejects A with
/// `NotAllowlisted` before any hello traffic flows.
#[tokio::test]
async fn non_allowlisted_third_node_is_rejected() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let key_c = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();

    let node_a = start(
        key_a,
        vec![key_b.public().to_peer_id()],
        manifest("host-a", &["data-store"]),
    )
    .await;
    let node_c = start(key_c, Vec::new(), manifest("host-c", &["data-store"])).await;

    let err = node_c
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect_err("c must not establish a session");
    assert!(
        matches!(err, ConnectError::NotAllowlisted { peer_id } if peer_id == peer_a),
        "expected NotAllowlisted for peer_a, got {err:?}"
    );

    // The allowlisted pair is unaffected: B dials A and gets a live session.
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;
    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("allowlisted b connects after rejection");
    assert_eq!(session.remote_peer_id(), peer_a);

    node_c.shutdown().await.expect("c shuts down");
    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (e) A stub handler returning a wire error envelope surfaces on the caller
/// as `InvokeError::Wire(ErrorEnvelope)` — the remote-failure path, distinct
/// from transport/session failures.
#[tokio::test]
async fn remote_error_envelope_surfaces_as_invoke_error_wire() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let envelope = ErrorEnvelope {
        code: "check_failed".into(),
        details: Default::default(),
        extensions: Default::default(),
        message: "spike check failed".into(),
    };
    let handler = {
        let envelope = envelope.clone();
        Arc::new(move |_op: &str, _payload: serde_json::Value| Err(envelope.clone()))
    };
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    let err = session
        .invoke("check", check_request())
        .await
        .expect_err("remote failure is an invoke error");
    match err {
        InvokeError::Wire(remote) => {
            assert_eq!(remote.code, "check_failed");
            assert_eq!(remote.message, "spike check failed");
        }
        other => panic!("expected InvokeError::Wire, got {other:?}"),
    }
    // The failed invoke still consumed its sequence.
    assert_eq!(session.next_sequence(), 1);

    node_a.shutdown().await.expect("a shuts down");
    node_b.shutdown().await.expect("b shuts down");
}

/// (f) A pending dial is bound to its own connection:
/// 1. A dials an address that stalls at the noise handshake — the dial can
///    only be resolved by the event loop's deadline sweep (identify never
///    arrives, no failure event fires).
/// 2. While that dial is pending, C completes an inbound handshake with A.
///    A's responder session for C must NOT consume A's pending dial.
/// 3. After the stall timeout, A's connect fails and the entry is cleared;
///    a retry against a live peer succeeds with that peer's session.
#[tokio::test]
async fn concurrent_inbound_session_does_not_consume_pending_dial() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let key_c = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();
    let peer_c = key_c.public().to_peer_id();
    let manifest_a = manifest("host-a", &["data-store"]);
    let manifest_b = manifest("host-b", &["input-source"]);
    let manifest_c = manifest("host-c", &["assembler"]);

    let node_a = SpokeConnectNode::start(config(
        key_a,
        vec![peer_b, peer_c],
        manifest_a.clone(),
        SHORT_HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("a starts");
    let node_b = SpokeConnectNode::start(config(
        key_b,
        vec![peer_a],
        manifest_b.clone(),
        SHORT_HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("b starts");
    let node_c = SpokeConnectNode::start(config(
        key_c,
        vec![peer_a],
        manifest_c.clone(),
        SHORT_HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("c starts");

    // A's dial to the stall address stays pending while C connects to A
    // (polled concurrently via join!, no sleeps).
    let stall_addr = stall_listener();
    let (c_result, stall_result) = tokio::join!(
        node_c.connect(node_a.listen_addrs()[0].clone()),
        node_a.connect(stall_addr),
    );

    let err = stall_result.expect_err("stalled dial must fail deterministically");
    assert!(
        matches!(
            err,
            ConnectError::Timeout(_)
                | ConnectError::Transport(_)
                | ConnectError::HandshakeFailed { .. }
        ),
        "unexpected stalled-dial error: {err:?}"
    );

    // C's inbound session completed during the pending dial.
    let c_session = c_result.expect("c connects");
    assert_eq!(c_session.remote_peer_id(), peer_a);

    // The pending entry was cleared: a retry against a live peer succeeds and
    // returns that peer's session (not C's).
    let session_b = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("retry succeeds");
    assert_eq!(session_b.remote_peer_id(), peer_b);
    assert_eq!(
        serde_json::to_value(session_b.remote_manifest()).expect("serialize"),
        serde_json::to_value(&manifest_b).expect("serialize"),
    );

    node_c.shutdown().await.expect("c shuts down");
    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (g) An unreachable known-peer multiaddr fails the pending dial fast (dial
/// error) and clears the entry: a retry against a live peer succeeds.
#[tokio::test]
async fn unreachable_peer_dial_fails_and_retry_succeeds() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let node_a = SpokeConnectNode::start(config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["data-store"]),
        SHORT_HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("a starts");
    let node_b = SpokeConnectNode::start(config(
        key_b,
        vec![peer_a],
        manifest("host-b", &["input-source"]),
        SHORT_HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("b starts");

    // A loopback port with nothing listening.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = closed.local_addr().expect("probe port").port();
    drop(closed);
    let closed_addr = format!("/ip4/127.0.0.1/tcp/{port}")
        .parse::<Multiaddr>()
        .expect("closed multiaddr");

    // The dial never establishes (connection refused); the event loop
    // resolves it deterministically at the handshake deadline.
    let err = node_a
        .connect(closed_addr)
        .await
        .expect_err("unreachable dial fails");
    assert!(
        matches!(
            err,
            ConnectError::Timeout(_)
                | ConnectError::Transport(_)
                | ConnectError::HandshakeFailed { .. }
        ),
        "unexpected unreachable-dial error: {err:?}"
    );

    let session = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("retry succeeds");
    assert_eq!(session.remote_peer_id(), peer_b);

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (h) A duplicate dial to an already-sessioned peer completes immediately
/// with a clone of the existing session; the surplus connection is closed
/// without killing the session.
#[tokio::test]
async fn duplicate_dial_completes_with_existing_session() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;

    let addr = node_a.listen_addrs()[0].clone();
    let session1 = node_b.connect(addr.clone()).await.expect("first dial");
    let session2 = node_b.connect(addr.clone()).await.expect("duplicate dial");

    // Same underlying session: duplicate dial completes with the existing
    // handle (one sequence counter).
    assert_eq!(session1.session_id(), session2.session_id());

    // The surplus connection was closed without killing the session: invokes
    // keep working on both handles (one shared sequence counter).
    let first = session1
        .invoke("check", check_request())
        .await
        .expect("invoke 1");
    let second = session2
        .invoke("check", check_request())
        .await
        .expect("invoke 2");
    assert_eq!(first.sequence, 0);
    assert_eq!(second.sequence, 1);

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (i) Session lifecycle: when the peer's node shuts down, the connection
/// closes, the session is marked closed, and a reconnecting peer with the
/// same identity establishes a fresh session.
#[tokio::test]
async fn reconnect_after_shutdown_establishes_fresh_session() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();
    let manifest_a = manifest("host-a", &["input-source"]);
    let manifest_b = manifest("host-b", &["checker"]);

    // The responder (B) carries the invoke handler; A dials B.
    let handler = Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    let mut cfg_b = config(
        key_b.clone(),
        vec![peer_a],
        manifest_b.clone(),
        HANDSHAKE_TIMEOUT,
    );
    cfg_b.invoke_handler = Some(handler);
    let node_a = start(key_a, vec![peer_b], manifest_a).await;
    let node_b = SpokeConnectNode::start(cfg_b).await.expect("b starts");

    let session1 = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("a dials b");
    session1
        .invoke("check", check_request())
        .await
        .expect("invoke while connected");

    // B shuts down: A observes the connection close and closes the session.
    node_b.shutdown().await.expect("b shuts down");
    wait_until_closed(&session1, HANDSHAKE_TIMEOUT).await;

    // A reconnecting node with the same identity establishes a fresh session.
    let mut cfg_b2 = config(key_b, vec![peer_a], manifest_b, HANDSHAKE_TIMEOUT);
    cfg_b2.invoke_handler = Some(Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    }));
    let node_b2 = SpokeConnectNode::start(cfg_b2).await.expect("b2 starts");
    let session2 = node_a
        .connect(node_b2.listen_addrs()[0].clone())
        .await
        .expect("reconnect succeeds");
    assert_ne!(
        session1.session_id(),
        session2.session_id(),
        "reconnect must create a fresh session, not resurrect the closed one"
    );
    session2
        .invoke("check", check_request())
        .await
        .expect("invoke after reconnect");

    node_b2.shutdown().await.expect("b2 shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (j) A panicking handler is contained: the invoke is answered with an
/// `internal_error` wire envelope and the node keeps serving.
#[tokio::test]
async fn panicking_handler_is_contained_and_node_survives() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(
        |_op: &str, _payload: serde_json::Value| -> Result<serde_json::Value, ErrorEnvelope> {
            panic!("spike handler panic");
        },
    );
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    for _ in 0..2 {
        let err = session
            .invoke("check", check_request())
            .await
            .expect_err("panic surfaces as a wire error");
        match err {
            InvokeError::Wire(envelope) => assert_eq!(envelope.code, "internal_error"),
            other => panic!("expected internal_error wire error, got {other:?}"),
        }
    }

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (k) The inbound op dispatch gate (normative MUST, §Op dispatch gate):
/// an op whose required capability is absent from the session's negotiated
/// capabilities is answered with an `op_unsupported` wire envelope and the
/// configured handler is never called; ops whose required capability IS
/// negotiated keep flowing to the handler.
///
/// A advertises `l2-computable`; B does not — the negotiated intersection
/// is {spoke-connect, spoke-baseline}, so `project` (requires
/// `l2-computable`) must be denied while `check` (requires
/// `spoke-baseline`) still succeeds.
#[tokio::test]
async fn inbound_dispatch_gate_denies_unnegotiated_capability_ops() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler = {
        let calls = calls.clone();
        Arc::new(move |op: &str, _payload: serde_json::Value| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(op, "check", "only negotiated ops reach the handler");
            Ok(serde_json::json!({ "findings": [], "extensions": {} }))
        })
    };
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest_with_capabilities(
            "host-a",
            &["checker"],
            &["spoke-connect", "spoke-baseline", "l2-computable"],
        ),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(
        key_b,
        vec![peer_a],
        manifest_with_capabilities(
            "host-b",
            &["input-source"],
            &["spoke-connect", "spoke-baseline"],
        ),
    )
    .await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    // The negotiated intersection is stored on the session (local manifest
    // order: B's capabilities kept where A also declared them).
    let expected: Vec<String> = ["spoke-connect", "spoke-baseline"]
        .iter()
        .map(|c| (*c).to_string())
        .collect();
    assert_eq!(session.negotiated_capabilities(), &expected);

    // `project` requires `l2-computable`, which B never advertised: denied
    // with the op_unsupported envelope and no handler side effect.
    let err = session
        .invoke("project", serde_json::json!({ "extensions": {} }))
        .await
        .expect_err("unnegotiated capability op must be denied");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "op_unsupported"),
        other => panic!("expected op_unsupported wire error, got {other:?}"),
    }
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the handler must not run for a capability-denied op"
    );

    // `check` only needs `spoke-baseline` (⊆ intersection): still flows.
    session
        .invoke("check", check_request())
        .await
        .expect("negotiated op still succeeds");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the handler runs exactly once, for the negotiated op"
    );

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (l-i) A product-defined op (`merge`, outside the core-op table) with a
/// MAPPED requirement that IS part of the negotiated intersection reaches the
/// configured handler: A maps `merge` → `spoke-baseline`, B advertises
/// `spoke-baseline`, so the invoke succeeds.
#[tokio::test]
async fn product_op_with_mapped_negotiated_capability_is_dispatchable() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(|op: &str, _payload: serde_json::Value| {
        assert_eq!(op, "merge", "handler sees the product op");
        Ok(serde_json::json!({ "merged": true, "extensions": {} }))
    });
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["data-store"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    cfg_a
        .op_capability_requirements
        .insert("merge".into(), "spoke-baseline".into());
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    // `spoke-baseline` is in both manifests, hence in the negotiated
    // intersection: the mapped product op is dispatchable.
    let success = session
        .invoke("merge", serde_json::json!({ "extensions": {} }))
        .await
        .expect("product op with negotiated capability succeeds");
    assert_eq!(
        success.payload,
        serde_json::json!({ "merged": true, "extensions": {} }),
        "the handler's payload arrives intact"
    );

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (l-ii) A product-defined op whose MAPPED requirement is absent from the
/// negotiated intersection is denied with the `op_unsupported` wire envelope
/// and the configured handler is never called — the map is not a bypass.
#[tokio::test]
async fn product_op_with_unnegotiated_mapped_capability_is_denied() {
    let _network_guard = network_test_guard().await;
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler = {
        let calls = calls.clone();
        Arc::new(move |_op: &str, _payload: serde_json::Value| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(serde_json::json!({ "extensions": {} }))
        })
    };
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["data-store"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    // `l2-computable` is advertised by neither host: even though the op is
    // mapped (and a handler is configured), the gate must deny it.
    cfg_a
        .op_capability_requirements
        .insert("merge".into(), "l2-computable".into());
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = start(key_b, vec![peer_a], manifest("host-b", &["input-source"])).await;

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a");

    let err = session
        .invoke("merge", serde_json::json!({ "extensions": {} }))
        .await
        .expect_err("unnegotiated mapped capability must deny the op");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "op_unsupported"),
        other => panic!("expected op_unsupported wire error, got {other:?}"),
    }
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the handler must not run for a capability-denied product op"
    );

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (m) The capability-token challenge/response flow with
/// `require_capability_token` true on both nodes: after both hellos, each
/// side challenges the other; provider-minted tokens from a trusted issuer
/// authorize both sessions, and invokes flow without per-invoke `auth`.
#[tokio::test]
async fn capability_token_challenge_authorizes_invokes() {
    let _network_guard = network_test_guard().await;
    let issuer_secret = [1u8; 32];
    let issuer_peer = issuer_peer_id(&issuer_secret);
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    let mut cfg_a = config(
        key_a,
        vec![peer_b],
        manifest_with_capabilities("host-a", &["checker"], &["spoke-connect", "spoke-baseline"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_a.invoke_handler = Some(handler);
    cfg_a.trusted_issuers = vec![issuer_peer.clone()];
    cfg_a.require_capability_token = true;
    cfg_a.capability_token_provider = Some(provider(
        issuer_secret,
        peer_a,
        vec!["spoke-baseline".into()],
        now_plus(3600),
    ));
    let mut cfg_b = config(
        key_b,
        vec![peer_a],
        manifest_with_capabilities(
            "host-b",
            &["input-source"],
            &["spoke-connect", "spoke-baseline"],
        ),
        HANDSHAKE_TIMEOUT,
    );
    cfg_b.trusted_issuers = vec![issuer_peer];
    cfg_b.require_capability_token = true;
    cfg_b.capability_token_provider = Some(provider(
        issuer_secret,
        peer_b,
        vec!["spoke-baseline".into()],
        now_plus(3600),
    ));
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = SpokeConnectNode::start(cfg_b).await.expect("b starts");

    // B dials A: the connect completes only after the challenge exchange —
    // B's token gate is part of session establishment when the policy is
    // active, so the returned session is already token-authorized.
    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("b dials a through the token challenge");
    assert!(
        session.capability_token_ok(),
        "the dialer's session must be token-authorized after the challenge"
    );

    // A's responder side validated B's token as well: an invoke without
    // `auth` is accepted (session grant in effect).
    let success = session
        .invoke("check", check_request())
        .await
        .expect("invoke without auth on a token-authorized session");
    assert_eq!(success.sequence, 0);

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (n) A receiver with `require_capability_token` true rejects invokes from
/// a session that never completed the challenge when no `auth` is attached
/// (`auth_failed` wire code); per-invoke `auth` with a valid token passes,
/// and an expired token is rejected with `auth_failed`.
#[tokio::test]
async fn require_token_receiver_rejects_unauthenticated_invokes() {
    let _network_guard = network_test_guard().await;
    let issuer_secret = [2u8; 32];
    let issuer_peer = issuer_peer_id(&issuer_secret);
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler = {
        let calls = calls.clone();
        Arc::new(move |_op: &str, _payload: serde_json::Value| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(serde_json::json!({ "findings": [], "extensions": {} }))
        })
    };
    // A: default policy (no trusted issuers) — it dials B.
    let cfg_a = config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["input-source"]),
        HANDSHAKE_TIMEOUT,
    );
    // B: requires a token, but has no provider (it cannot answer A's
    // challenges either) — its session with A never completes the challenge.
    let mut cfg_b = config(
        key_b,
        vec![peer_a],
        manifest("host-b", &["checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_b.invoke_handler = Some(handler);
    cfg_b.trusted_issuers = vec![issuer_peer];
    cfg_b.require_capability_token = true;
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = SpokeConnectNode::start(cfg_b).await.expect("b starts");

    let session = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("a dials b (a has no token policy)");
    assert!(
        !session.capability_token_ok(),
        "a's session carries no token grant (a never challenged b)"
    );

    // No `auth` on a not-token-authorized session: rejected with auth_failed.
    let err = session
        .invoke("check", check_request())
        .await
        .expect_err("missing token must be rejected");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "auth_failed"),
        other => panic!("expected auth_failed wire error, got {other:?}"),
    }

    // Per-invoke `auth` with a valid token from the trusted issuer: accepted.
    let auth = token_proof(
        &issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline"],
        now_plus(3600),
    );
    session
        .invoke_with_auth("check", check_request(), Some(auth))
        .await
        .expect("per-invoke auth with a valid token is accepted");

    // Expired token: rejected with auth_failed.
    let expired = token_proof(
        &issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline"],
        now_plus(0).saturating_sub(60),
    );
    let err = session
        .invoke_with_auth("check", check_request(), Some(expired))
        .await
        .expect_err("expired token must be rejected");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "auth_failed"),
        other => panic!("expected auth_failed wire error, got {other:?}"),
    }

    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the handler runs exactly once, for the valid-token invoke"
    );

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (o) With `require_capability_token` false, no-`auth` behavior is
/// unchanged (noise-peerid only), while an attached `auth` proof is still
/// validated on **every** invoke: an untrusted issuer and a tampered proof
/// are both rejected with `auth_failed`.
#[tokio::test]
async fn per_invoke_auth_validated_when_require_flag_false() {
    let _network_guard = network_test_guard().await;
    let issuer_secret = [3u8; 32];
    let other_issuer_secret = [4u8; 32];
    let issuer_peer = issuer_peer_id(&issuer_secret);
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    // B trusts the issuer but does NOT require a token: the challenge is not
    // offered and no-`auth` invokes keep the noise-peerid-only behavior.
    let mut cfg_b = config(
        key_b,
        vec![peer_a],
        manifest("host-b", &["checker"]),
        HANDSHAKE_TIMEOUT,
    );
    cfg_b.invoke_handler = Some(handler);
    cfg_b.trusted_issuers = vec![issuer_peer];
    let node_a = SpokeConnectNode::start(config(
        key_a,
        vec![peer_b],
        manifest("host-a", &["input-source"]),
        HANDSHAKE_TIMEOUT,
    ))
    .await
    .expect("a starts");
    let node_b = SpokeConnectNode::start(cfg_b).await.expect("b starts");

    let session = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("a dials b");

    // No auth, require flag false: unchanged behavior.
    session
        .invoke("check", check_request())
        .await
        .expect("no-auth invoke succeeds when the require flag is false");

    // Attached valid auth: validated and accepted.
    let auth = token_proof(
        &issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline"],
        now_plus(3600),
    );
    session
        .invoke_with_auth("check", check_request(), Some(auth))
        .await
        .expect("valid per-invoke auth is accepted");

    // Auth from an untrusted issuer: rejected with auth_failed.
    let untrusted = token_proof(
        &other_issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline"],
        now_plus(3600),
    );
    let err = session
        .invoke_with_auth("check", check_request(), Some(untrusted))
        .await
        .expect_err("untrusted issuer must be rejected");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "auth_failed"),
        other => panic!("expected auth_failed wire error, got {other:?}"),
    }

    // Tampered proof (claims mutated after issuance → signature no longer
    // verifies): rejected with auth_failed.
    let mut tampered = token_proof(
        &issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline"],
        now_plus(3600),
    );
    tampered["claims"]["capabilities"] = serde_json::json!(["spoke-baseline", "l2-computable"]);
    let err = session
        .invoke_with_auth("check", check_request(), Some(tampered))
        .await
        .expect_err("tampered proof must be rejected");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "auth_failed"),
        other => panic!("expected auth_failed wire error, got {other:?}"),
    }

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (p) The token grant gates dispatch by **capability membership**: an op
/// whose required capability is part of the negotiated set but absent from
/// the token grant is denied with `op_unsupported` (handler never called);
/// a token granting the required capability lets the same op through.
#[tokio::test]
async fn token_grant_capability_membership_gates_dispatch() {
    let _network_guard = network_test_guard().await;
    let issuer_secret = [5u8; 32];
    let issuer_peer = issuer_peer_id(&issuer_secret);
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();
    // Both hosts advertise l2-computable: `project` IS in the negotiated
    // intersection, so any denial below comes from the token grant, not the
    // negotiated-set gate.
    let caps = ["spoke-connect", "spoke-baseline", "l2-computable"];

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler = {
        let calls = calls.clone();
        Arc::new(move |op: &str, _payload: serde_json::Value| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(op, "project", "only token-granted ops reach the handler");
            Ok(serde_json::json!({ "findings": [], "extensions": {} }))
        })
    };
    let cfg_a = config(
        key_a,
        vec![peer_b],
        manifest_with_capabilities("host-a", &["input-source"], &caps),
        HANDSHAKE_TIMEOUT,
    );
    let mut cfg_b = config(
        key_b,
        vec![peer_a],
        manifest_with_capabilities("host-b", &["checker"], &caps),
        HANDSHAKE_TIMEOUT,
    );
    cfg_b.invoke_handler = Some(handler);
    cfg_b.trusted_issuers = vec![issuer_peer];
    cfg_b.require_capability_token = true;
    let node_a = SpokeConnectNode::start(cfg_a).await.expect("a starts");
    let node_b = SpokeConnectNode::start(cfg_b).await.expect("b starts");

    let session = node_a
        .connect(node_b.listen_addrs()[0].clone())
        .await
        .expect("a dials b");

    // Token grants only spoke-baseline: `project` (requires l2-computable)
    // is denied with op_unsupported even though it IS negotiated.
    let baseline_only = token_proof(
        &issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline"],
        now_plus(3600),
    );
    let err = session
        .invoke_with_auth(
            "project",
            serde_json::json!({ "extensions": {} }),
            Some(baseline_only),
        )
        .await
        .expect_err("token without the op's capability must deny");
    match err {
        InvokeError::Wire(envelope) => assert_eq!(envelope.code, "op_unsupported"),
        other => panic!("expected op_unsupported wire error, got {other:?}"),
    }
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the handler must not run for a capability-denied op"
    );

    // A token granting the required capability (extra capabilities ignored):
    // the same op flows to the handler.
    let with_l2 = token_proof(
        &issuer_secret,
        &peer_a.to_string(),
        &peer_b.to_string(),
        &["spoke-baseline", "l2-computable", "unused-extra"],
        now_plus(3600),
    );
    session
        .invoke_with_auth(
            "project",
            serde_json::json!({ "extensions": {} }),
            Some(with_l2),
        )
        .await
        .expect("token granting the required capability lets the op through");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the handler runs exactly once, for the token-granted op"
    );

    node_b.shutdown().await.expect("b shuts down");
    node_a.shutdown().await.expect("a shuts down");
}
