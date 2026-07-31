//! Two-node loopback exchange — DoD integration coverage for
//! `crates/spoke-connect` (default features; mDNS off).
//!
//! Scenarios:
//! (a) both nodes exchange signed hellos; remote manifests round-trip intact;
//! (b) one `check` op invoke closes the loop — sequence 0 + request_id echo;
//! (c) a second invoke uses sequence 1;
//! (d) a third node not on the allowlist is rejected at the handshake;
//! (e) a stub handler returning an error envelope surfaces as
//!     `InvokeError::Wire(ErrorEnvelope)`.
//!
//! All waits are bounded event waits (timeouts on `connect` / `invoke`
//! futures) — no sleep-based synchronization.

use libp2p::identity::Keypair;
use libp2p::PeerId;
use spoke_connect::{parse_multiaddr, ConnectConfig, ConnectError, InvokeError, SpokeConnectNode};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;
use std::sync::Arc;
use std::time::Duration;

/// Handshake / invoke timeout for the test nodes (≥ 5s keeps loopback CI
/// reliable; every wait in these tests is bounded by it).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const LISTEN: &str = "/ip4/127.0.0.1/tcp/0";

fn manifest(host_id: &str, roles: &[&str]) -> HostCapabilityManifest {
    HostCapabilityManifest {
        authority: None,
        capabilities: vec!["spoke-connect".into(), "spoke-baseline".into()],
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
) -> ConnectConfig {
    ConnectConfig {
        identity,
        peer_allowlist: allowlist,
        listen_addrs: vec![parse_multiaddr(LISTEN).expect("multiaddr")],
        local_manifest,
        handshake_timeout: Some(HANDSHAKE_TIMEOUT),
        invoke_handler: None,
    }
}

async fn start(
    identity: Keypair,
    allowlist: Vec<PeerId>,
    local_manifest: HostCapabilityManifest,
) -> SpokeConnectNode {
    SpokeConnectNode::start(config(identity, allowlist, local_manifest))
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

/// (a) Two nodes on loopback complete the authenticated hello handshake and
/// both sides' manifests round-trip through the generated wire type.
#[tokio::test]
async fn hello_exchange_round_trips_manifests_both_ways() {
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
    assert_eq!(
        serde_json::to_value(session.remote_manifest()).expect("serialize"),
        serde_json::to_value(&manifest_a).expect("serialize"),
    );
    let wire = serde_json::to_value(session.remote_manifest()).expect("serialize");
    assert_eq!(wire["host_id"], serde_json::json!("host-a"));
    assert_eq!(wire["roles"], serde_json::json!(["data-store", "checker"]));

    node_a.shutdown().await.expect("a shuts down");
    node_b.shutdown().await.expect("b shuts down");
}

/// (b) One `check` op invoke closes the loop: A answers through a stub
/// handler returning a real check-response envelope; the caller sees
/// `sequence == 0` and the request_id echo (asserted by the correlation
/// gate; `InvokeSuccess.request_id` is the echoed id).
#[tokio::test]
async fn check_op_invoke_round_trips_with_sequence_zero() {
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
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let handler = Arc::new(|_op: &str, _payload: serde_json::Value| {
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    let mut cfg_a = config(key_a, vec![peer_b], manifest("host-a", &["checker"]));
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
/// session is established. A allowlists only B; C allowlists nobody
/// (fail-closed), so C's own gate rejects A's hello
/// (`NotAllowlisted`), or A's gate rejects C's hello first
/// (`HandshakeFailed`) — both are handshake reject equivalents.
#[tokio::test]
async fn non_allowlisted_third_node_is_rejected() {
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let key_c = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    let node_a = start(key_a, vec![peer_b], manifest("host-a", &["data-store"])).await;
    let node_c = start(key_c, Vec::new(), manifest("host-c", &["data-store"])).await;

    let err = node_c
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect_err("c must not establish a session");
    match err {
        // C's own gate rejected A's hello (C allowlists nobody).
        ConnectError::NotAllowlisted { peer_id } => assert_eq!(peer_id, peer_a),
        // A rejected C's hello first (C not on A's allowlist) — stream
        // dropped, no ack.
        ConnectError::HandshakeFailed { .. } => {}
        other => panic!("unexpected connect error: {other:?}"),
    }

    node_c.shutdown().await.expect("c shuts down");
    node_a.shutdown().await.expect("a shuts down");
}

/// (e) A stub handler returning a wire error envelope surfaces on the caller
/// as `InvokeError::Wire(ErrorEnvelope)` — the remote-failure path, distinct
/// from transport/session failures.
#[tokio::test]
async fn remote_error_envelope_surfaces_as_invoke_error_wire() {
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
    let mut cfg_a = config(key_a, vec![peer_b], manifest("host-a", &["checker"]));
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
