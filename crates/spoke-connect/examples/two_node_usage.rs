//! Two-node usage example for `spoke-connect`.
//!
//! Node B dials node A's resolved listen address; both sides exchange their
//! signed `ConnectHello` (and with it their `HostCapabilityManifest`), then B
//! invokes the `check` op on A's stub handler. Mirrors the README usage
//! section — this file is compiled by `cargo test -p spoke-connect` so the
//! documented happy path stays runnable.
//!
//! Run: `cargo run -p spoke-connect --example two_node_usage`

use libp2p::identity::Keypair;
use spoke_connect::{parse_multiaddr, ConnectConfig, SpokeConnectNode};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

fn manifest(host_id: &str, role: &str) -> HostCapabilityManifest {
    HostCapabilityManifest {
        authority: None,
        capabilities: vec!["spoke-baseline".into()],
        extensions: Default::default(),
        host_id: host_id.parse().expect("host id parses"),
        namespaces: Vec::new(),
        roles: vec![role.into()],
        schema_version: NonZeroU64::new(1).expect("non-zero"),
    }
}

#[tokio::main]
async fn main() {
    // Two nodes, each allowlisting the other's PeerId.
    let key_a = Keypair::generate_ed25519();
    let key_b = Keypair::generate_ed25519();
    let peer_a = key_a.public().to_peer_id();
    let peer_b = key_b.public().to_peer_id();

    // Node A answers inbound invokes through the responder handler.
    let handler = Arc::new(|op: &str, _payload: serde_json::Value| {
        assert_eq!(op, "check", "handler sees the requested op");
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    });
    let config_a = ConnectConfig {
        identity: key_a,
        peer_allowlist: vec![peer_b],
        listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
        local_manifest: manifest("host-a", "checker"),
        handshake_timeout: None,
        invoke_handler: Some(handler),
        op_capability_requirements: HashMap::new(),
    };
    let node_a = SpokeConnectNode::start(config_a).await.expect("start a");

    // Node B dials A's resolved remote address.
    let config_b = ConnectConfig {
        identity: key_b,
        peer_allowlist: vec![peer_a],
        listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
        local_manifest: manifest("host-b", "input-source"),
        handshake_timeout: None,
        invoke_handler: None,
        op_capability_requirements: HashMap::new(),
    };
    let node_b = SpokeConnectNode::start(config_b).await.expect("start b");

    let session = node_b
        .connect(node_a.listen_addrs()[0].clone())
        .await
        .expect("session established");
    println!(
        "session {} with peer {} (roles {:?})",
        session.session_id(),
        session.remote_peer_id(),
        session.remote_manifest().roles
    );

    let success = session
        .invoke(
            "check",
            serde_json::json!({ "scope": { "scope_id": "spike-scope-1" } }),
        )
        .await
        .expect("invoke succeeds");
    println!("sequence {} -> {:?}", success.sequence, success.payload);

    node_a.shutdown().await.expect("shutdown a");
    node_b.shutdown().await.expect("shutdown b");
}
