//! Loopback interop test — Rust `RemoteAdapter` (client) ↔ a Rust connect
//! host serving an async `ToyWorldAdapter` (server) over the in-repo
//! `loopback_transport_pair` (frozen contract §10 verification checklist;
//! parity with `packages/spoke-connect-ts/tests/remote/remote-adapter.test.ts`).
//!
//! Asserts, per the plan:
//! (a) ENCAPSULATION — the consumer surface is ONLY the async `BaselinePorts`
//!     (+ `connect_remote_adapter`): the client is driven through the port
//!     traits and orchestration entrypoints only; no verification helpers are
//!     reachable on the adapter.
//! (b) DROP-IN — `orchestrate_upsert(remote, req)` / `orchestrate_check(...)`
//!     return the same `SpokeResult` as the local `ToyWorldAdapter` for
//!     identical requests (upsert + conflict-reject + check paths).
//! (c) VERIFICATION RAN — the connect handshake actually happened (host
//!     allowlist + signature + nonce gates; remote hello host cached).
//!
//! Plus the §10 concurrency/error rows: concurrent invokes demuxed by
//! `request_id` with out-of-order responses, invoke timeout, transport close
//! mid-flight, dispatch deny mapping, HostManifestPort cache/proxy, and
//! fail-closed allowlist dials.
//!
//! This file is gated on the `remote-adapter` feature; `cargo test -p
//! spoke-connect` (default features) does not build it.

#![cfg(feature = "remote-adapter")]

#[path = "common/loopback_oracle.rs"]
mod loopback_oracle;
use loopback_oracle::*;

#[path = "common/minimal_responder.rs"]
mod minimal_responder;
use minimal_responder::*;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Serialize;
use serde_json::{json, Value};
use spoke_connect::core::{
    derive_peer_id_from_ed25519_pubkey, sign_hello_ed25519, verify_hello_ed25519,
    CapabilityClaims, CapabilityTokenProof, CoreInvokeError,
};
use spoke_connect::remote::{
    connect_multi_peer_router, connect_remote_adapter, loopback_transport_pair, reset_accepted_server_hellos_for_test,
    LoopbackTransport, LoopbackTransportPair, MultiPeerRouterOptions, RemoteAdapter,
    RemoteAdapterError, RemoteAdapterOptions, RemoteIdentity, Transport, TransportError,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_operations::{
    orchestrate_check, orchestrate_upsert, spoke_ok, spoke_reject, BaselinePorts, CheckRunInput,
    FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort, ScopeQueryPort,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::host_capability_manifest::HostCapabilityManifestExtensionsKey;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::connect::ConnectSession;
use spoke_schemas::{CheckRequest, Finding, HostCapabilityManifest, KnowledgeEntry, Scope, UpsertRequest};

/// Compare two `SpokeResult`s structurally (generated types do not derive
/// `PartialEq`).
fn results_equal<T: std::fmt::Debug>(left: &SpokeResult<T>, right: &SpokeResult<T>) -> bool {
    // Generated wire types do not derive PartialEq; Debug is the derived
    // structural equality surface.
    format!("{left:?}") == format!("{right:?}")
}

/// Extract the `details.kind` of an `INTERNAL_ERROR` reject (or `None`).
fn reject_kind(result: &SpokeResult<KnowledgeEntry>) -> Option<String> {
    match result {
        SpokeResult::Ok(_) => None,
        SpokeResult::Reject(reject) => reject
            .details
            .as_ref()
            .and_then(|details| details.get("kind"))
            .and_then(|kind| kind.as_str())
            .map(|kind| kind.to_string()),
    }
}

// ── Greptile #1 fixture transports (replayed server hello) ────────────────

/// Delegating transport that records every inbound envelope (the view an
/// active transport attacker has after one legitimate dial).
struct RecordingTransport {
    inner: Arc<dyn Transport>,
    captured: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait]
#[async_trait]
impl Transport for RecordingTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let bytes = self.inner.recv().await?;
        self.captured.lock().expect("captured lock").push(bytes.clone());
        Ok(bytes)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

/// Wire-level injector transport wrapper (mirror of the TS
/// `tamperOutboundRequests` / `tamperInboundResponses` helpers): mutates
/// envelopes in one direction on the wire — the view an active transport
/// attacker has of the peers' signed envelopes. A `None` mutation returns
/// `None` to pass the envelope through unchanged (the handshake hello /
/// session snapshot traverse untouched so the dial still establishes).
struct TamperTransport {
    inner: Arc<dyn Transport>,
    /// Outbound (client → host) mutation; `None` = pass through.
    outbound: Option<Arc<dyn Fn(Value) -> Option<Value> + Send + Sync>>,
    /// Inbound (host → client) mutation; `None` = pass through.
    inbound: Option<Arc<dyn Fn(Value) -> Option<Value> + Send + Sync>>,
}

#[async_trait]
impl Transport for TamperTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        let Some(mutate) = self.outbound.as_ref() else {
            return self.inner.send(envelope).await;
        };
        let doc: Value = serde_json::from_slice(envelope).map_err(|_| TransportError::Closed)?;
        match mutate(doc) {
            Some(mutated) => {
                let bytes = serde_json::to_vec(&mutated).map_err(|_| TransportError::Closed)?;
                self.inner.send(&bytes).await
            }
            None => self.inner.send(envelope).await,
        }
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let bytes = self.inner.recv().await?;
        let Some(mutate) = self.inbound.as_ref() else {
            return Ok(bytes);
        };
        let doc: Value = serde_json::from_slice(&bytes).map_err(|_| TransportError::Closed)?;
        match mutate(doc) {
            Some(mutated) => {
                Ok(serde_json::to_vec(&mutated).map_err(|_| TransportError::Closed)?)
            }
            None => Ok(bytes),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

/// Scripted transport that answers like an attacker replaying captured
/// envelopes: server hello, then session snapshot, then "connection closed".
struct ReplayTransport {
    hello: Vec<u8>,
    session: Vec<u8>,
    index: AtomicUsize,
}

#[async_trait]
#[async_trait]
impl Transport for ReplayTransport {
    async fn send(&self, _envelope: &[u8]) -> Result<(), TransportError> {
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let index = self.index.fetch_add(1, Ordering::SeqCst);
        match index {
            0 => Ok(self.hello.clone()),
            1 => Ok(self.session.clone()),
            _ => Err(TransportError::Closed),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

/// Scripted transport that answers like a mixed-version OLD responder: it
/// serves a 4-field signed hello (no `peer_nonce`) and nothing else — the
/// new initiator's fail-closed dial must reject at the hello, before the
/// session snapshot is even requested.
struct LegacyHelloTransport {
    hello: Vec<u8>,
}

#[async_trait]
impl Transport for LegacyHelloTransport {
    async fn send(&self, _envelope: &[u8]) -> Result<(), TransportError> {
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        Ok(self.hello.clone())
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn encapsulates_verification_and_is_drop_in_baseline_ports() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let local_adapter = ToyWorldAdapter::with_committed_fixtures(); // drop-in parity target
    let (client, host) = dial(host_adapter, DialOptions::default()).await;

    // (a) Encapsulation: the consumer surface is ONLY the async
    //     BaselinePorts — the trait-object coercion proves the type surface
    //     compiles, and the test drives the adapter through ports +
    //     orchestration entrypoints only (no hello/nonce/sequence helpers).
    let ports: &dyn BaselinePorts = client.as_ref();
    let _: &dyn KnowledgeEntryPort = ports;

    // (c) Verification ran: both hellos authenticated, remote hello host
    //     cached (the REMOTE "test-host", not the client's "test-client").
    assert_eq!(host.stats().hellos_verified, 1);
    assert_eq!(client.state().as_str(), "Established");
    assert_eq!(client.session_id().as_deref(), Some(host.session_id()));
    assert_eq!(
        client.remote_peer_id().as_deref(),
        Some(derive_peer_id_from_ed25519_pubkey(&pubkey_host()).as_str())
    );
    assert_eq!(
        client
            .remote_manifest()
            .expect("remote manifest")
            .host_id
            .as_str(),
        "test-host"
    );

    // (b) Drop-in parity — upsert path: identical requests produce identical
    //     SpokeResults on the local adapter and the remote one.
    let candidate = fresh_entry("kb_remote_cartographer", "Remote Cartographer");
    let request = upsert_request(&[candidate]);
    let local_upsert = orchestrate_upsert(&local_adapter, request.clone()).await;
    let remote_upsert = orchestrate_upsert(client.as_ref(), request).await;
    assert!(results_equal(&remote_upsert, &local_upsert));
    assert!(remote_upsert.is_ok());
    // The remote write actually landed in the host-side store.
    assert!(host
        .inner
        .adapter
        .get_knowledge_entry("kb_remote_cartographer")
        .await
        .is_ok());

    // Drop-in parity — reject path: a conflicting second upsert rejects
    // identically on both sides (error branch → SpokeResult reject).
    let request = upsert_request(&[fresh_entry("kb_remote_cartographer", "Remote Cartographer")]);
    let local_conflict = orchestrate_upsert(&local_adapter, request.clone()).await;
    let remote_conflict = orchestrate_upsert(client.as_ref(), request).await;
    assert!(remote_conflict.is_reject());
    assert!(results_equal(&remote_conflict, &local_conflict));

    // Drop-in parity — check path (listKnowledgeEntries + listTimelineEvents +
    // listRules + putFindings over the wire).
    let checker = |_: CheckRunInput| spoke_ok(Vec::<Finding>::new());
    let local_check =
        orchestrate_check(&local_adapter, check_request("toy-scope-001"), checker).await;
    let remote_check =
        orchestrate_check(client.as_ref(), check_request("toy-scope-001"), checker).await;
    assert!(results_equal(&remote_check, &local_check));
    assert!(remote_check.is_ok());

    client.close();
    host.close();
}

#[tokio::test]
async fn demuxes_concurrent_invokes_with_out_of_order_responses() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    // Deterministic out-of-order fixture: sequence-0 responses are delayed
    // 30ms, so the sequence-1 response arrives first.
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_delay: Some(Box::new(
                |request| {
                    if request.sequence == 0 {
                        30
                    } else {
                        0
                    }
                },
            )),
            ..Default::default()
        },
    )
    .await;

    // Both invokes must be in flight concurrently (the delay fixture only
    // works when seq-0 is already parked when seq-1 arrives).
    let (first, second) = tokio::join!(
        client.get_knowledge_entry("kb_tw_mira"), // sequence 0 — delayed
        client.get_knowledge_entry("kb_tw_harbor"), // sequence 1 — fast
    );
    match (&first, &second) {
        (SpokeResult::Ok(mira), SpokeResult::Ok(harbor)) => {
            assert_eq!(mira.entry_id.as_str(), "kb_tw_mira");
            assert_eq!(harbor.entry_id.as_str(), "kb_tw_harbor");
        }
        _ => panic!("both concurrent gets must succeed"),
    }
    // The delayed response landed second: demux delivered to the right
    // waiter despite arrival order.
    let stats = host.stats();
    assert_eq!(stats.response_order, vec![1, 0]);
    assert_eq!(stats.invokes_dispatched, 2);

    client.close();
    host.close();
}

#[tokio::test]
async fn maps_invoke_timeout_to_internal_error_kind_timeout_without_closing_session() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let delay_ms = Arc::new(AtomicU64::new(100));
    let delay_ms_clone = Arc::clone(&delay_ms);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            invoke_timeout_ms: Some(20),
            host_delay: Some(Box::new(move |_| delay_ms_clone.load(Ordering::Relaxed))),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(result.is_reject());
    assert_eq!(reject_kind(&result).as_deref(), Some("timeout"));

    // Timeout fails only the waiter — the session stays usable.
    assert_eq!(client.state().as_str(), "Established");
    delay_ms.store(0, Ordering::Relaxed);
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_pending_invokes_with_session_closed_when_transport_closes_mid_flight() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_delay: Some(Box::new(|_| 100)),
            ..Default::default()
        },
    )
    .await;

    // The request is registered + sent synchronously, then the host drops
    // the connection while the response is still delayed.
    let pending = client.get_knowledge_entry("kb_tw_mira");
    host.close();
    let result = pending.await;
    assert!(result.is_reject());
    assert_eq!(reject_kind(&result).as_deref(), Some("session_closed"));
    assert_eq!(client.state().as_str(), "Closed");

    // Subsequent port calls also fail closed with session_closed.
    let after = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(after.is_reject());
    assert_eq!(reject_kind(&after).as_deref(), Some("session_closed"));

    client.close();
    host.close();
}

#[tokio::test]
async fn maps_host_dispatch_denials_to_capability_port_missing() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    // A client manifest without spoke-baseline ⇒ the negotiated set is empty
    // ⇒ the host's dispatch gate denies every port.* op.
    let no_baseline = manifest("test-client", &["l2-computable"]);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            client_manifest: Some(no_baseline),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(|code| code.as_str()),
                Some("op_unsupported")
            );
        }
        SpokeResult::Ok(_) => panic!("dispatch denial must reject"),
    }
    let stats = host.stats();
    assert_eq!(stats.dispatch_denials, 1);
    assert_eq!(stats.invokes_dispatched, 0);

    client.close();
    host.close();
}

#[tokio::test]
async fn host_manifest_port_returns_remote_hello_host_from_cache_and_proxies_peer_list() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(host_adapter, DialOptions::default()).await;

    // getHostCapabilityManifest = remote hello host cache — NO invoke.
    let self_manifest = client.get_host_capability_manifest().await;
    match &self_manifest {
        SpokeResult::Ok(manifest) => assert_eq!(manifest.host_id.as_str(), "test-host"),
        SpokeResult::Reject(_) => panic!("cache manifest must succeed"),
    }
    assert_eq!(host.stats().invokes_dispatched, 0);

    // listPeerHostCapabilityManifests = remote proxy (product-seeded peers).
    let peers = client.list_peer_host_capability_manifests().await;
    match &peers {
        SpokeResult::Ok(manifests) => {
            let host_ids: Vec<&str> = manifests.iter().map(|m| m.host_id.as_str()).collect();
            assert_eq!(host_ids, vec!["host_tw_peer"]);
        }
        SpokeResult::Reject(_) => panic!("peer list must succeed"),
    }
    assert_eq!(host.stats().invokes_dispatched, 1);

    client.close();
    host.close();
}

#[tokio::test]
async fn attaches_configured_capability_token_as_auth_on_outbound_invokes() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    let token = spoke_connect::core::issue_capability_token(
        &seed_client(),
        CapabilityClaims {
            iss: derive_peer_id_from_ed25519_pubkey(&pubkey_client()),
            sub: derive_peer_id_from_ed25519_pubkey(&pubkey_client()),
            aud: derive_peer_id_from_ed25519_pubkey(&pubkey_host()),
            capabilities: vec!["spoke-baseline".to_string()],
            exp: now + 3600,
            iat: None,
            jti: None,
        },
        now,
    )
    .expect("token issues");
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            capability_token: Some(token),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(result.is_ok());
    // The host observed the auth field on the wire (attach path ran).
    assert!(host.stats().auth_seen);

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_dial_fail_closed_when_remote_peer_is_not_on_allowlist() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let pair = loopback_transport_pair();
    let foreign_peer = derive_peer_id_from_ed25519_pubkey(&[0x70u8; 32]);
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![foreign_peer], // wrong peer — fail-closed before any hello
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("dial must fail closed"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RemoteAdapterError::Config(ref message) if message.contains("not on the allowlist"))
    );

    host.close();
}

#[tokio::test]
async fn fails_dial_when_host_rejects_client_hello() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let host_peer_id = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let other_peer_id = derive_peer_id_from_ed25519_pubkey(&[0x20u8; 32]);
    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![other_peer_id], // the real client is NOT allowed
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![host_peer_id],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("dial must fail when the host rejects the client hello"),
        Err(error) => error,
    };
    // The host closes the transport on a hello-gate failure, so the client
    // observes a handshake failure (never a half-open adapter).
    assert!(
        matches!(error, RemoteAdapterError::Handshake(_)),
        "unexpected dial error: {error:?}"
    );
    host.close();
}

#[tokio::test]
async fn maps_correlation_mismatch_to_internal_error_kind_correlation_mismatch() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let mangled = Arc::new(AtomicBool::new(true));
    let mangled_clone = Arc::clone(&mangled);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Same request_id (so the demux still finds the pending waiter)
            // but a wrong sequence echo — a correlation failure (§6 echo
            // rules). Fires once; the retry below uses the real response.
            host_response_override: Some(Box::new(move |request| {
                if !mangled_clone.swap(false, Ordering::SeqCst) {
                    return None;
                }
                Some(json!({
                    "session_id": request.session_id,
                    "sequence": request.sequence + 1,
                    "request_id": request.request_id,
                    "payload": {},
                    "extensions": {},
                }))
            })),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(result.is_reject());
    assert_eq!(
        reject_kind(&result).as_deref(),
        Some("correlation_mismatch")
    );
    // Mismatch fails only the waiter — the session stays usable.
    assert_eq!(client.state().as_str(), "Established");
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());

    client.close();
    host.close();
}

#[tokio::test]
async fn rejects_a_replayed_server_hello_before_any_session_is_accepted() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();

    // Dial 1 through a recording transport — the view an active transport
    // attacker has after one legitimate dial (server hello + session
    // snapshot captured at the wire).
    let pair = loopback_transport_pair();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(RecordingTransport {
            inner: Arc::new(pair.client),
            captured: Arc::clone(&captured),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    .expect("first dial");
    assert_eq!(client.state().as_str(), "Established");
    client.close();
    host.close();
    let captured = captured.lock().expect("captured lock").clone();
    assert!(captured.len() >= 2, "captured hello + session snapshot");
    let replay_hello = captured[0].clone();
    let replay_session = captured[1].clone();

    // Dial 2: replay the captured envelopes through a scripted transport
    // with NO real host on the other end. Without receiver-side nonce
    // single-use this dial would succeed — the signature is genuinely the
    // allowlisted peer's — and the attacker could fabricate a session and
    // answer invokes; the fix rejects the replay before any
    // `ConnectSession` snapshot is accepted.
    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(ReplayTransport {
            hello: replay_hello,
            session: replay_session,
            index: AtomicUsize::new(0),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("replayed server hello must be rejected"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RemoteAdapterError::Handshake(ref message) if message.contains("replay") || message.contains("dial binding")),
        "unexpected dial error: {error:?}"
    );
}

#[tokio::test]
async fn replayed_server_hello_is_rejected_across_restart_by_dial_binding() {
    // Greptile P1 scenario, now defeated: a captured responder hello is
    // replayed on a FRESH dial after a "restart" — the process-wide
    // accepted-hello store is reset (in-memory state lost) and the initiator
    // nonce is new. The responder's signed `peer_nonce` (dial 1's initiator
    // nonce) does not match the new initiator nonce, so the dial-binding
    // assert rejects the replay even though the signature is genuinely the
    // allowlisted peer's and the nonce store has forgotten the pair.
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();

    // Dial 1 through a recording transport — capture the responder hello.
    let pair = loopback_transport_pair();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(RecordingTransport {
            inner: Arc::new(pair.client),
            captured: Arc::clone(&captured),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    .expect("first dial");
    assert_eq!(client.state().as_str(), "Established");
    client.close();
    host.close();
    let captured = captured.lock().expect("captured lock").clone();
    assert!(captured.len() >= 2, "captured hello + session snapshot");
    let replay_hello = captured[0].clone();
    let replay_session = captured[1].clone();

    // "Restart": the in-memory accepted-hello store resets.
    reset_accepted_server_hellos_for_test();

    // Dial 2: fresh initiator nonce, replay the captured responder hello
    // through a scripted transport with NO real host. The receiver-side
    // nonce gate cannot help (store reset) — only the dial-binding assert
    // (signed `peer_nonce` != new initiator nonce) rejects the replay.
    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(ReplayTransport {
            hello: replay_hello,
            session: replay_session,
            index: AtomicUsize::new(0),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("replayed server hello must be rejected after a restart"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RemoteAdapterError::Handshake(ref message) if message.contains("dial binding")),
        "unexpected dial error: {error:?}"
    );
}

#[tokio::test]
async fn responder_hello_without_peer_nonce_fails_the_dial_closed() {
    // Mixed-version downgrade: an OLD responder (pre-dial-binding) signs the
    // 4-field initiator object — no `peer_nonce` on the wire, with a
    // genuinely valid signature from the allowlisted host key. The NEW
    // initiator dial expects a responder (it supplies its own nonce), so the
    // missing `peer_nonce` must fail the dial closed — not silently skip
    // the binding assert (fail-open downgrade).
    let legacy_hello = sign_hello_ed25519(
        &seed_host(),
        &host_nonce(),
        &connect_manifest(&manifest("test-host", &["spoke-baseline"])),
        None,
    )
    .expect("sign legacy 4-field hello");
    let legacy_bytes = serde_json::to_vec(&legacy_hello).expect("legacy hello serializes");

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(LegacyHelloTransport {
            hello: legacy_bytes,
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("responder hello without peer_nonce must fail the dial"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, RemoteAdapterError::Handshake(message) if message.contains("dial binding")),
        "unexpected dial error: {error:?}"
    );
}

#[tokio::test]
async fn mixed_version_responder_hello_surfaces_protocol_version_mismatch() {
    // A mixed-version responder hello (protocol_version != core) must
    // surface the DEDICATED dial kind — `RemoteAdapterError::ProtocolVersionMismatch`
    // — not be folded into `Handshake`. The version gate runs before
    // signature verification (verify_hello_ed25519 step 1), so the dial
    // rejects a wrong-version hello even though the (v1-signed) signature no
    // longer matches the mutated object — you do not waste crypto on a
    // wrong-version peer.
    let mut hello = sign_hello_ed25519(
        &seed_host(),
        &host_nonce(),
        &connect_manifest(&manifest("test-host", &["spoke-baseline"])),
        Some("initiator-nonce-12345678"),
    )
    .expect("sign responder hello");
    hello.protocol_version = std::num::NonZeroU64::new(2).expect("non-zero");
    let bytes = serde_json::to_vec(&hello).expect("responder hello serializes");

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(LegacyHelloTransport { hello: bytes }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("mixed-version hello must fail the dial"),
        Err(error) => error,
    };
    assert!(
        matches!(
            &error,
            RemoteAdapterError::ProtocolVersionMismatch(message)
                if message.contains("unsupported protocol_version 2 (expected 1)")
        ),
        "unexpected dial error: {error:?}"
    );
}

// ── Envelope-auth enforcement (contract §7/§8) ─────────────────────────────

/// Extract the `details.kind` of an `INTERNAL_ERROR` reject (or `None`).
fn reject_kind_of(result: &SpokeResult<KnowledgeEntry>) -> Option<&str> {
    match result {
        SpokeResult::Reject(reject) => reject
            .details
            .as_ref()
            .and_then(|details| details.get("kind"))
            .and_then(Value::as_str),
        SpokeResult::Ok(_) => None,
    }
}

#[tokio::test]
async fn host_rejects_a_wire_tampered_invoke_request_with_auth_failed_and_no_advance_or_dispatch() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let tampered = Arc::new(AtomicBool::new(false));
    let tampered_flag = Arc::clone(&tampered);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // One-shot wire-level injector: only the FIRST outbound invoke
            // request's payload is mutated AFTER the signature was computed
            // — the host's envelope-auth verify must reject it.
            client_transport: Some(Box::new(move |client_end| {
                let tampered_flag = Arc::clone(&tampered_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: Some(Arc::new(move |doc| {
                        if doc.get("op").is_some() && !tampered_flag.swap(true, Ordering::SeqCst) {
                            let mut doc = doc;
                            doc["payload"]["tampered"] = json!(true);
                            return Some(doc);
                        }
                        None
                    })),
                    inbound: None,
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The host answered `auth_failed` (wire code) with the locked
    // `details.kind`; the client maps it to `INTERNAL_ERROR` verbatim.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_invalid"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // Auth-before-advance: the forged envelope consumed nothing. The
    // host's inbound counter is still at 0 (the next expected sequence was
    // NOT consumed) and the host's counters prove the forged envelope was
    // neither dispatched nor counted as a sequence rejection. (The client's
    // own outbound counter has moved on — protocol v1 defines no retry —
    // so the session is deliberately not reused here, mirroring the TS
    // twin.)
    let stats = host.stats();
    assert_eq!(host.inbound_next_expected(), 0);
    assert_eq!(stats.auth_rejections, 1);
    assert_eq!(stats.sequence_rejections, 0);
    assert_eq!(stats.invokes_dispatched, 0);

    client.close();
    host.close();
}

#[tokio::test]
async fn host_rejects_an_invoke_request_with_a_stripped_signature_as_auth_failed() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let stripped = Arc::new(AtomicBool::new(false));
    let stripped_flag = Arc::clone(&stripped);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Wire-level injector: strip the signature from the FIRST
            // outbound invoke request — the host must answer `auth_failed`
            // with `envelope_auth_missing` (v2 requires the signature on
            // every post-hello envelope; a missing authenticator is never
            // dispatched).
            client_transport: Some(Box::new(move |client_end| {
                let stripped_flag = Arc::clone(&stripped_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: Some(Arc::new(move |doc| {
                        if doc.get("op").is_some() && !stripped_flag.swap(true, Ordering::SeqCst) {
                            let mut doc = doc;
                            if let Some(object) = doc.as_object_mut() {
                                object.remove("signature");
                            }
                            return Some(doc);
                        }
                        None
                    })),
                    inbound: None,
                })
            })),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_missing"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // No advance, no dispatch — the host's inbound counter is untouched
    // (next expected still 0) and the stripped envelope was never
    // dispatched nor counted as a sequence rejection.
    let stats = host.stats();
    assert_eq!(host.inbound_next_expected(), 0);
    assert_eq!(stats.auth_rejections, 1);
    assert_eq!(stats.sequence_rejections, 0);
    assert_eq!(stats.invokes_dispatched, 0);

    client.close();
    host.close();
}

#[tokio::test]
async fn rejects_a_wire_tampered_invoke_response_with_envelope_auth_invalid() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let tampered = Arc::new(AtomicBool::new(false));
    let tampered_flag = Arc::clone(&tampered);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // One-shot wire-level injector: only the FIRST response's
            // payload is mutated after the host signed it — the client's
            // envelope-auth verify must reject it (fail-closed, only this
            // waiter).
            client_transport: Some(Box::new(move |client_end| {
                let tampered_flag = Arc::clone(&tampered_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: None,
                    inbound: Some(Arc::new(move |doc| {
                        if doc.get("request_id").is_some()
                            && doc.get("payload").is_some()
                            && !tampered_flag.swap(true, Ordering::SeqCst)
                        {
                            let mut doc = doc;
                            doc["payload"]["tampered"] = json!(true);
                            return Some(doc);
                        }
                        None
                    })),
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The client's `verify_invoke_response_auth` rejects the tampered
    // response with the locked `details.kind` via `INTERNAL_ERROR`.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_invalid"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // A forged response fails only this waiter — no session-state mutation:
    // the next invoke round-trips (the host dispatched the first request
    // normally; only the response was tampered with).
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 2);
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

#[tokio::test]
async fn rejects_an_invoke_response_with_a_stripped_signature_as_envelope_auth_missing() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let stripped = Arc::new(AtomicBool::new(false));
    let stripped_flag = Arc::clone(&stripped);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Wire-level injector: delete the `signature` field of the FIRST
            // response — the client must fail closed with
            // `envelope_auth_missing` (v2 requires the signature on every
            // response branch).
            client_transport: Some(Box::new(move |client_end| {
                let stripped_flag = Arc::clone(&stripped_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: None,
                    inbound: Some(Arc::new(move |doc| {
                        if doc.get("request_id").is_some()
                            && !stripped_flag.swap(true, Ordering::SeqCst)
                        {
                            let mut doc = doc;
                            if let Some(object) = doc.as_object_mut() {
                                object.remove("signature");
                            }
                            return Some(doc);
                        }
                        None
                    })),
                })
            })),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_missing"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_the_dial_when_the_session_snapshot_signature_is_stripped() {
    // Wire-level injector on the client end: strip the `signature` from the
    // session snapshot — the dial must fail closed at the snapshot verify
    // (`verify_session_auth` runs before typed checks / before establish;
    // contract §7 — a v2 snapshot without a valid authenticator never
    // establishes a session).
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let stripped = Arc::new(AtomicBool::new(false));
    let stripped_flag = Arc::clone(&stripped);
    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(TamperTransport {
            inner: Arc::new(pair.client),
            outbound: None,
            inbound: Some(Arc::new(move |doc| {
                // Session shape only — the signed server hello (no
                // `session_id`) passes through so the dial reaches the
                // snapshot step.
                if doc.get("session_id").is_some()
                    && doc.get("initiator_peer_id").is_some()
                    && doc.get("request_id").is_none()
                    && !stripped_flag.swap(true, Ordering::SeqCst)
                {
                    let mut doc = doc;
                    if let Some(object) = doc.as_object_mut() {
                        object.remove("signature");
                    }
                    return Some(doc);
                }
                None
            })),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("dial must fail closed on a stripped session snapshot signature"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, RemoteAdapterError::Handshake(message) if message.contains("missing a signature")),
        "unexpected dial error: {error:?}"
    );

    host.close();
}

#[tokio::test]
async fn rejects_a_host_signed_response_with_wrong_session_id_fail_closed() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let fired = Arc::new(AtomicBool::new(false));
    let fired_flag = Arc::clone(&fired);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // The host signs its own response envelope — but over a
            // `session_id` that is not bound to the established session
            // (the override is signed by the host after the request passed
            // every inbound gate). The signature verifies against the
            // host's hello key; the response still fails closed.
            //
            // NOTE: the surfaced kind is `correlation_mismatch`, not
            // `envelope_auth_session_unbound`: `session_id` is one of the
            // three correlation echo fields, and the locked check order
            // (mirrored in TS) runs the correlation echo check BEFORE
            // envelope-auth verify — so a wrong `session_id` is caught by
            // the correlation gate first. The session-binding kind is
            // covered end-to-end by
            // `fails_the_dial_when_the_session_snapshot_carries_unbound_peer_ids`
            // and at the core level by `verify_invoke_response_auth` unit
            // tests. Fires once; the retry below uses the real response.
            host_response_override: Some(Box::new(move |request| {
                if !fired_flag.swap(true, Ordering::SeqCst) {
                    return Some(json!({
                        "session_id": "not-the-established-session",
                        "sequence": request.sequence,
                        "request_id": request.request_id,
                        "payload": {},
                        "extensions": {},
                    }));
                }
                None
            })),
            ..Default::default()
        },
    )
    .await;

    // The wrong-session response fails only this waiter — correlation
    // mismatch, fail-closed, no session-state mutation.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("correlation_mismatch"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // The session stays established and the next invoke round-trips
    // normally.
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_the_dial_when_the_session_snapshot_carries_unbound_peer_ids() {
    // The host signs a `ConnectSession` snapshot whose responder peer id is
    // not one of the authenticated hellos' peer ids. The client's
    // `verify_session_auth` verifies the host's signature over the wire
    // form and then fires the step-6 session-binding assert
    // (`envelope_auth_session_unbound`) — the dial fails closed, no
    // adapter instance is created.
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let pair = loopback_transport_pair();
    let foreign_peer = derive_peer_id_from_ed25519_pubkey(&[0x70u8; 32]);
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: Some((
            derive_peer_id_from_ed25519_pubkey(&pubkey_client()),
            foreign_peer.clone(),
        )),
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("unbound-peer-id snapshot must fail the dial"),
        Err(error) => error,
    };
    // `verify_session_auth` step 6 fired `EnvelopeAuthError::SessionUnbound`
    // (host-signed snapshot, binding mismatch) — the message names the
    // unbound peer id and the authenticated hellos.
    assert!(
        matches!(&error, RemoteAdapterError::Handshake(message) if message.contains("session peer ids")),
        "unexpected dial error: {error:?}"
    );

    host.close();
}

/// Wire-level transport wrapper that duplicates the FIRST outbound invoke
/// request (an envelope carrying `op`) — the host then serves two
/// same-sequence envelopes concurrently (one task per envelope, §10
/// concurrency rows), racing peek → verify → advance.
struct DuplicateOnceTransport {
    inner: Arc<dyn Transport>,
    duplicated: AtomicBool,
}

#[async_trait]
impl Transport for DuplicateOnceTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        let doc: Value = serde_json::from_slice(envelope).map_err(|_| TransportError::Closed)?;
        if doc.get("op").is_some() && !self.duplicated.swap(true, Ordering::SeqCst) {
            // Deliver the request twice: both copies hit the host's
            // serve loop and run `handle_invoke` concurrently.
            self.inner.send(envelope).await?;
            return self.inner.send(envelope).await;
        }
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.inner.recv().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

#[tokio::test]
async fn concurrent_same_sequence_duplicate_is_rejected_non_fatally() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Duplicate the first invoke request on the wire: the host
            // races two same-sequence envelopes through
            // peek → verify → advance. Exactly one wins; the loser must be
            // answered `inbound_sequence_mismatch` (non-fatal, counter
            // increments) — the fixture must NOT panic on the lost race.
            client_transport: Some(Box::new(|client_end| {
                Arc::new(DuplicateOnceTransport {
                    inner: client_end,
                    duplicated: AtomicBool::new(false),
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The invoke settles exactly once (ok or reject — response order of the
    // payload vs. the mismatch error is not deterministic), the host
    // dispatched exactly one handler, and the duplicate was rejected as a
    // sequence mismatch — never a panic, never a poisoned session.
    let _result = client.get_knowledge_entry("kb_tw_mira").await;
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 1);
    assert_eq!(stats.sequence_rejections, 1);
    assert_eq!(stats.auth_rejections, 0);
    assert_eq!(client.state().as_str(), "Established");

    // The session remains usable: the next invoke round-trips normally.
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(host.stats().invokes_dispatched, 2);
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

// ── Fix-wave 1: outbound send serialization + post-verify decode grace ────

/// Wire-level transport wrapper that yields inside `send` before delegating
/// — widens the interleave window at the adapter's `transport.send().await`
/// yield point. Without outbound send serialization, a later-allocated
/// request can reach the wire first and the host's strict inbound sequence
/// gate answers `inbound_sequence_mismatch` (contract §5.3 / §10).
struct YieldingSendTransport {
    inner: Arc<dyn Transport>,
}

#[async_trait]
impl Transport for YieldingSendTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        // Yield before pushing: forces a scheduling point inside the
        // adapter's send so concurrent invokes genuinely race for the wire.
        tokio::task::yield_now().await;
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.inner.recv().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_invokes_reach_the_wire_in_allocation_order() {
    const CONCURRENT_INVOKES: i64 = 32;

    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    // Replicates `dial()` with a server-end recording wrapper so the wire
    // order of the client's outbound invoke requests is observable.
    let pair = loopback_transport_pair();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(RecordingTransport {
            inner: Arc::new(pair.server),
            captured: Arc::clone(&captured),
        }),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![peer_id_client.clone()],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        // Yield inside every send so concurrent invokes race for the wire
        // position (without serialization the host would see a
        // later-allocated sequence first and reject it).
        transport: Arc::new(YieldingSendTransport {
            inner: Arc::new(pair.client),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_host.clone()],
        invoke_timeout_ms: Some(5000),
        capability_token: None,
    })
    .await
    .expect("dial");

    // Fire all invokes concurrently; each allocates its outbound sequence
    // in call order. Serialized sends must put them on the wire 0..N-1.
    let mut handles = Vec::new();
    for _ in 0..CONCURRENT_INVOKES {
        let client = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            client.get_knowledge_entry("kb_tw_mira").await
        }));
    }
    for handle in handles {
        let result = handle.await.expect("invoke task must not panic");
        assert!(result.is_ok(), "concurrent invoke must succeed: {result:?}");
    }

    // The host accepted and dispatched every request in sequence — no
    // out-of-order rejection, no auth rejection.
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, CONCURRENT_INVOKES as usize);
    assert_eq!(stats.sequence_rejections, 0);
    assert_eq!(stats.auth_rejections, 0);

    // Wire witness: the request envelopes the host received carry exactly
    // the monotonic allocation order 0..N-1.
    let captured = captured.lock().expect("captured lock").clone();
    let sequences: Vec<i64> = captured
        .iter()
        .filter_map(|bytes| {
            let doc: Value = serde_json::from_slice(bytes).ok()?;
            doc.get("op").is_some().then(|| doc.get("sequence")?.as_i64())
        })
        .flatten()
        .collect();
    assert_eq!(
        sequences,
        (0..CONCURRENT_INVOKES).collect::<Vec<i64>>(),
        "wire request sequences must be monotonic (allocation order)"
    );

    client.close();
    host.close();
}

#[tokio::test]
async fn host_answers_invalid_request_non_fatally_when_verified_extensions_are_malformed() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let mutated = Arc::new(AtomicBool::new(false));
    let mutated_flag = Arc::clone(&mutated);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // One-shot wire-level injector: on the FIRST invoke request,
            // replace the (unsigned) `extensions` bag with a value whose
            // key violates the schema pattern `^[a-z][a-z0-9_-]*$`. The
            // signature still verifies (extensions are not in the signed
            // object, contract §3), so the request passes envelope-auth
            // verify and then fails typed deserialization.
            client_transport: Some(Box::new(move |client_end| {
                let mutated_flag = Arc::clone(&mutated_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: Some(Arc::new(move |doc| {
                        if doc.get("op").is_some() && !mutated_flag.swap(true, Ordering::SeqCst)
                        {
                            let mut doc = doc;
                            if let Some(object) = doc.as_object_mut() {
                                object.insert(
                                    "extensions".into(),
                                    json!({ "Bad_Key": { "n": 1 } }),
                                );
                            }
                            Some(doc)
                        } else {
                            None
                        }
                    })),
                    inbound: None,
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The host must NOT panic on the post-verify decode failure — it
    // answers a non-fatal `invalid_request` error envelope instead.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    match &result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("invalid_request")
            );
        }
        SpokeResult::Ok(_) => panic!("malformed unsigned extensions must be rejected"),
    }
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 0);
    assert_eq!(stats.auth_rejections, 0);
    assert_eq!(stats.sequence_rejections, 0);
    // The verified envelope's sequence position was consumed (advance
    // happened before typed deserialization).
    assert_eq!(host.inbound_next_expected(), 1);
    assert_eq!(client.state().as_str(), "Established");

    // The session stays usable: the next invoke round-trips normally.
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(host.stats().invokes_dispatched, 1);
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

// ── Multi-peer router loopback proof (contract §9) ────────────────────────

/// Dial a client against a fresh loopback host with a CUSTOM host identity
/// seed + host manifest — the multi-peer proof needs distinct remote peer
/// ids (tie-break) and disjoint capability manifests (routing), which the
/// fixed `dial` fixture cannot express. The client side keeps the standard
/// fixture identity; per-peer session state stays independent (contract §1).
async fn dial_peer(
    host_seed: [u8; 32],
    host_manifest: HostCapabilityManifest,
) -> (Arc<RemoteAdapter>, LoopbackHost) {
    let host_pubkey = SigningKey::from_bytes(&host_seed)
        .verifying_key()
        .to_bytes();
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&host_pubkey);
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed,
        host_manifest,
        allowlist: vec![peer_id_client.clone()],
        adapter: Arc::new(ToyWorldAdapter::default()),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: host_pubkey,
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial");
    (client, host)
}

#[tokio::test]
async fn multi_peer_router_routes_upsert_and_check_to_the_capable_peer() {
    // Two peers with DISJOINT capabilities: baseline vs l2-computable.
    let (baseline_adapter, baseline_host) =
        dial_peer([0xa1; 32], manifest("host-baseline", &["spoke-baseline"])).await;
    let (computable_adapter, computable_host) =
        dial_peer([0xb2; 32], manifest("host-computable", &["l2-computable"])).await;

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    let baseline_id = router
        .register_peer(baseline_adapter.clone())
        .expect("register baseline");
    let computable_id = router
        .register_peer(computable_adapter.clone())
        .expect("register computable");
    // Registry holds both; selection (not registration order) routes.
    assert_eq!(router.list_peers(), vec![baseline_id, computable_id]);

    // orchestrateUpsert → port.knowledge.get + port.knowledge.put — both
    // baseline ops → the spoke-baseline peer; the l2-computable peer is
    // never touched.
    let request = upsert_request(&[fresh_entry("kb_mpr_upsert", "Multi-Peer Upsert")]);
    let upsert = orchestrate_upsert(&router, request).await;
    assert!(
        upsert.is_ok(),
        "upsert must route to the baseline peer: {upsert:?}"
    );
    assert_eq!(baseline_host.stats().invokes_dispatched, 2);
    assert_eq!(computable_host.stats().invokes_dispatched, 0);
    // The remote write actually landed on the selected peer's host store.
    assert!(baseline_host
        .inner
        .adapter
        .get_knowledge_entry("kb_mpr_upsert")
        .await
        .is_ok());

    // orchestrateCheck → listKnowledgeEntries + listTimelineEvents +
    // putFindings (listRules is skipped: the fixture request carries no
    // rule_refs) — all baseline ops → the same peer.
    let checker = |_: CheckRunInput| spoke_ok(Vec::<Finding>::new());
    let check = orchestrate_check(&router, check_request("toy-scope-001"), checker).await;
    assert!(
        check.is_ok(),
        "check must route to the baseline peer: {check:?}"
    );
    assert_eq!(baseline_host.stats().invokes_dispatched, 5);
    assert_eq!(computable_host.stats().invokes_dispatched, 0);

    baseline_adapter.close();
    computable_adapter.close();
    baseline_host.close();
    computable_host.close();
}

#[tokio::test]
async fn multi_peer_router_rejects_no_capable_peer_when_no_peer_has_the_capability() {
    // Only an l2-computable peer registered — every baseline op on the
    // router's six-family surface has no capable peer.
    let (computable_adapter, computable_host) =
        dial_peer([0xb2; 32], manifest("host-computable", &["l2-computable"])).await;

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    router
        .register_peer(computable_adapter.clone())
        .expect("register computable");

    let request = upsert_request(&[fresh_entry("kb_mpr_nomatch", "No Match")]);
    let result = orchestrate_upsert(&router, request).await;
    match &result {
        SpokeResult::Reject(reject) => {
            // §5 locked reject: CAPABILITY_PORT_MISSING + details.kind /
            // wire_code = no_capable_peer — terminal, stable.
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str),
                Some("no_capable_peer")
            );
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("no_capable_peer")
            );
        }
        SpokeResult::Ok(_) => panic!("no capable peer must reject"),
    }
    // Terminal: no delegate ran (no wrong-peer fallback).
    assert_eq!(computable_host.stats().invokes_dispatched, 0);

    computable_adapter.close();
    computable_host.close();
}

#[tokio::test]
async fn multi_peer_router_breaks_ties_on_the_lowest_peer_id_across_two_baseline_peers() {
    // Three peers: alpha (baseline-only), beta (baseline + l2-computable),
    // gamma (l2-computable-only). A baseline op has TWO candidates (alpha +
    // beta); the locked §4 tie-break picks the lowest peer_id in UTF-8 byte
    // order — the host ids derive from the seeds, so the expected recipient
    // is computed at runtime rather than hunted for.
    let (alpha_adapter, alpha_host) =
        dial_peer([0xc3; 32], manifest("host-alpha", &["spoke-baseline"])).await;
    let (beta_adapter, beta_host) = dial_peer(
        [0xd4; 32],
        manifest("host-beta", &["spoke-baseline", "l2-computable"]),
    )
    .await;
    let (gamma_adapter, gamma_host) =
        dial_peer([0xe5; 32], manifest("host-gamma", &["l2-computable"])).await;

    let alpha_id = alpha_adapter.remote_peer_id().expect("alpha peer id");
    let beta_id = beta_adapter.remote_peer_id().expect("beta peer id");

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    router
        .register_peer(alpha_adapter.clone())
        .expect("register alpha");
    router
        .register_peer(beta_adapter.clone())
        .expect("register beta");
    router
        .register_peer(gamma_adapter.clone())
        .expect("register gamma");

    let request = upsert_request(&[fresh_entry("kb_mpr_tiebreak", "Tie-Break")]);
    let upsert = orchestrate_upsert(&router, request).await;
    assert!(upsert.is_ok(), "tie-break upsert must route: {upsert:?}");

    let (expected_host, other_host) = if alpha_id < beta_id {
        (&alpha_host, &beta_host)
    } else {
        (&beta_host, &alpha_host)
    };
    // Both baseline port calls (get + put) select the same lowest-id peer.
    assert_eq!(expected_host.stats().invokes_dispatched, 2);
    assert_eq!(other_host.stats().invokes_dispatched, 0);
    // The l2-computable-only peer is never a candidate for baseline ops.
    assert_eq!(gamma_host.stats().invokes_dispatched, 0);

    alpha_adapter.close();
    beta_adapter.close();
    gamma_adapter.close();
    alpha_host.close();
    beta_host.close();
    gamma_host.close();
}

#[tokio::test]
async fn multi_peer_router_composes_host_manifests_over_loopback_peers() {
    // §6 over real wires: two loopback peers with overlapping capabilities
    // and distinct roles/namespaces. The composed view must union + dedup
    // the real signed-hello manifests, carry the ROUTER's own host_id
    // (never a peer's), omit authority, and list contributing peer ids
    // sorted; the per-peer array must return each peer's OWN cached hello
    // manifest sorted by peer_id — per-peer data, never the union.
    let peer_a_manifest: HostCapabilityManifest = serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": "host-a",
        "capabilities": ["spoke-baseline", "l2-computable"],
        "roles": ["data-store", "checker"],
        "namespaces": ["alpha", "beta"],
        "extensions": {},
    }))
    .expect("valid host-a manifest");
    let peer_b_manifest: HostCapabilityManifest = serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": "host-b",
        "capabilities": ["spoke-baseline", "l2-computable"],
        "roles": ["data-store", "assembler"],
        "namespaces": ["beta", "gamma"],
        "extensions": {},
    }))
    .expect("valid host-b manifest");

    let (peer_a_adapter, peer_a_host) = dial_peer([0xd1; 32], peer_a_manifest).await;
    let (peer_b_adapter, peer_b_host) = dial_peer([0xe2; 32], peer_b_manifest).await;
    let peer_a_id = peer_a_adapter.remote_peer_id().expect("peer-a id");
    let peer_b_id = peer_b_adapter.remote_peer_id().expect("peer-b id");

    let router = connect_multi_peer_router(MultiPeerRouterOptions {
        host_id: Some("test-router".to_string()),
    });
    router
        .register_peer(peer_a_adapter.clone())
        .expect("register a");
    router
        .register_peer(peer_b_adapter.clone())
        .expect("register b");

    // Composed view (§6).
    let composed = router.get_host_capability_manifest().await;
    match composed {
        SpokeResult::Ok(composed) => {
            assert_eq!(composed.host_id.as_str(), "test-router");
            let mut capabilities = composed.capabilities.clone();
            capabilities.sort();
            assert_eq!(capabilities, vec!["l2-computable", "spoke-baseline"]);
            let mut roles = composed.roles.clone();
            roles.sort();
            assert_eq!(roles, vec!["assembler", "checker", "data-store"]);
            let mut namespaces: Vec<&str> =
                composed.namespaces.iter().map(|ns| ns.as_str()).collect();
            namespaces.sort();
            assert_eq!(namespaces, vec!["alpha", "beta", "gamma"]);
            assert!(composed.authority.is_none());
            let router_ext = composed
                .extensions
                .get(&HostCapabilityManifestExtensionsKey::try_from("router").expect("key"))
                .expect("router extensions");
            let peers = router_ext
                .get("peers")
                .and_then(Value::as_array)
                .expect("peers array");
            let mut peer_ids: Vec<&str> = peers
                .iter()
                .map(|value| value.as_str().expect("peer id string"))
                .collect();
            peer_ids.sort();
            let mut expected_ids = vec![peer_a_id.as_str(), peer_b_id.as_str()];
            expected_ids.sort();
            assert_eq!(peer_ids, expected_ids);
        }
        SpokeResult::Reject(reject) => panic!("composed view must succeed: {reject:?}"),
    }

    // Per-peer array: each peer's own hello manifest, sorted by peer_id.
    let per_peer = router.list_peer_host_capability_manifests().await;
    match per_peer {
        SpokeResult::Ok(manifests) => {
            assert_eq!(manifests.len(), 2);
            let mut host_ids: Vec<&str> = manifests
                .iter()
                .map(|manifest| manifest.host_id.as_str())
                .collect();
            host_ids.sort();
            assert_eq!(host_ids, vec!["host-a", "host-b"]);
            let host_a = manifests
                .iter()
                .find(|manifest| manifest.host_id.as_str() == "host-a")
                .expect("host-a entry");
            let host_a_roles: Vec<&str> =
                host_a.roles.iter().map(|role| role.as_str()).collect();
            assert_eq!(host_a_roles, vec!["data-store", "checker"]);
            let host_a_namespaces: Vec<&str> = host_a
                .namespaces
                .iter()
                .map(|ns| ns.as_str())
                .collect();
            assert_eq!(host_a_namespaces, vec!["alpha", "beta"]);
            let host_b = manifests
                .iter()
                .find(|manifest| manifest.host_id.as_str() == "host-b")
                .expect("host-b entry");
            let host_b_roles: Vec<&str> =
                host_b.roles.iter().map(|role| role.as_str()).collect();
            assert_eq!(host_b_roles, vec!["data-store", "assembler"]);
            let host_b_namespaces: Vec<&str> = host_b
                .namespaces
                .iter()
                .map(|ns| ns.as_str())
                .collect();
            assert_eq!(host_b_namespaces, vec!["beta", "gamma"]);
        }
        SpokeResult::Reject(reject) => panic!("per-peer list must succeed: {reject:?}"),
    }

    peer_a_adapter.close();
    peer_b_adapter.close();
    peer_a_host.close();
    peer_b_host.close();
}

// ══════════════════════════════════════════════════════════════════════════
// Tool serving (reverse invokes) + connectResponder — mirrored loopback
// scenarios from TS T1 (`reverse-invoke.test.ts`) + T2
// (`responder.test.ts`). The dialer is the real `connect_remote_adapter`;
// the peer is either the minimal test responder double
// (`tests/common/minimal_responder_impl.rs`, adapter-side scenarios) or the
// production `connect_responder` (responder-side scenarios).
// ══════════════════════════════════════════════════════════════════════════

use spoke_connect::remote::{
    connect_responder, ConnectResponder, ConnectResponderOptions, RemoteAdapterState, ToolHandler,
};

/// Fixture seed: base+i, all values within byte range for base ≤ 0xe0.
fn seed(base: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = base.wrapping_add(i as u8);
    }
    bytes
}

/// Tool descriptors (frozen §2: `op === capability_id`, namespaces owned).
fn add_descriptor() -> Value {
    json!({
        "schema_version": 1,
        "capability_id": "tools.math.add",
        "op": "tools.math.add",
        "description": "Add two integers",
        "input": { "type": "object" },
        "output": { "type": "object" },
    })
}

fn echo_descriptor() -> Value {
    json!({
        "schema_version": 1,
        "capability_id": "tools.echo.echo",
        "op": "tools.echo.echo",
        "description": "Echo the arguments",
        "input": { "type": "object" },
        "output": { "type": "object" },
    })
}

fn boom_descriptor() -> Value {
    json!({
        "schema_version": 1,
        "capability_id": "tools.echo.boom",
        "op": "tools.echo.boom",
        "description": "Explodes",
        "input": { "type": "object" },
        "output": { "type": "object" },
    })
}

/// Tool-carrying manifest: namespaces own the tool namespaces; every tool
/// capability ∈ capabilities[].
fn tool_manifest(host_id: &str) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": [
            "spoke-baseline",
            "tools.math.add",
            "tools.echo.echo",
            "tools.echo.boom",
        ],
        "namespaces": ["math", "echo", "toy_world"],
        "extensions": {},
        "tools": [add_descriptor(), echo_descriptor(), boom_descriptor()],
    }))
    .expect("valid tool manifest")
}

/// Baseline-only manifest (no tools): the negotiated set lacks the tool
/// capability, so the serving dispatch gate denies.
fn no_tools_manifest(host_id: &str) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": ["spoke-baseline"],
        "namespaces": ["toy_world"],
        "extensions": {},
    }))
    .expect("valid manifest")
}

/// The add handler used by most fixtures: records the arguments object and
/// returns `{ "sum": a + b }`.
fn add_handler(calls: Arc<Mutex<Vec<Value>>>) -> ToolHandler {
    Arc::new(move |args: Value| {
        let calls = Arc::clone(&calls);
        Box::pin(async move {
            calls.lock().expect("calls lock").push(args.clone());
            let a = args.get("a").and_then(Value::as_i64).unwrap_or(0);
            let b = args.get("b").and_then(Value::as_i64).unwrap_or(0);
            spoke_ok(json!({ "sum": a + b }))
        })
    })
}

/// Poll an async state transition (loopback close propagation).
async fn until_state<F: Fn() -> RemoteAdapterState>(
    get: F,
    expected: RemoteAdapterState,
    what: &str,
) {
    for _ in 0..200 {
        if get() == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} did not reach {expected:?}");
}

// ── Adapter-side fixtures (minimal responder double as the peer) ──────────

/// Wire-level injector on the responder end: mutates the FIRST outbound
/// reverse request's payload after signing (the view an active transport
/// attacker has on the client end). The handshake hellos pass through
/// unchanged.
struct TamperFirstRequestTransport {
    inner: Arc<dyn Transport>,
    tampered: AtomicBool,
}

#[async_trait]
impl Transport for TamperFirstRequestTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        let doc: Value = serde_json::from_slice(envelope)
            .map_err(|error| TransportError::Io(error.to_string()))?;
        if doc.get("op").is_some() && !self.tampered.swap(true, Ordering::SeqCst) {
            let mut doc = doc;
            doc["payload"]["tampered"] = json!(true);
            let bytes = serde_json::to_vec(&doc)
                .map_err(|error| TransportError::Io(error.to_string()))?;
            return self.inner.send(&bytes).await;
        }
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.inner.recv().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

/// Response-override hook (malformed-response fixtures).
type ResponseOverride = Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>;

/// Dial the real `connect_remote_adapter` (client) against the minimal
/// responder double (server) over a loopback pair.
async fn dial_with_tools(
    client_manifest: Option<HostCapabilityManifest>,
    responder_transport: Option<TransportWrap>,
    response_override: Option<ResponseOverride>,
) -> (Arc<RemoteAdapter>, Arc<MinimalResponder>) {
    let pair = loopback_transport_pair();
    let server_end: Arc<dyn Transport> = match responder_transport {
        Some(wrap) => wrap(Arc::new(pair.server)),
        None => Arc::new(pair.server),
    };
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let responder = start_minimal_responder(MinimalResponderOptions {
        transport: server_end,
        seed: seed_host(),
        client_pubkey: pubkey_client(),
        allowlist: vec![peer_id_client.clone()],
        manifest: tool_manifest("test-responder"),
        invoke_timeout_ms: None,
        response_override,
    })
    .await;
    let peer_id_responder = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: client_manifest.unwrap_or_else(|| tool_manifest("test-client")),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_responder.clone()],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial");
    (client, responder)
}

// ── RemoteAdapter tool serving (mirror of TS reverse-invoke.test.ts) ──────

#[tokio::test]
async fn serves_a_reverse_invoke_issued_by_the_responder() {
    let calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (client, responder) = dial_with_tools(None, None, None).await;
    client.register_tool_handler("tools.math.add", add_handler(Arc::clone(&calls)));
    let result = responder
        .issue_invoke("tools.math.add", json!({ "a": 2, "b": 3 }), None)
        .await;
    match result {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 5 })),
        SpokeResult::Reject(reject) => panic!("reverse invoke must succeed: {reject:?}"),
    }
    // The dialer-side registered handler is what ran (not a responder side
    // effect), with the request's arguments object passed through.
    assert_eq!(
        calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 2, "b": 3 })]
    );
    assert_eq!(
        responder.stats.responses_verified.load(Ordering::SeqCst),
        1
    );
    // The session stays Established on both ends.
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn does_not_demux_a_reverse_request_as_a_response_while_a_forward_waiter_is_pending() {
    let calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (client, responder) = dial_with_tools(None, None, None).await;
    client.register_tool_handler("tools.math.add", add_handler(Arc::clone(&calls)));
    // The responder serves the dialer's forward invoke; the dialer serves
    // the responder's reverse invoke. Under the pre-fix discriminator the
    // reverse request (op-bearing, response-shaped) would be swallowed by
    // the request_id demux and the responder would time out.
    responder.register_tool_handler("tools.echo.echo", Arc::new(|args: Value| {
        Box::pin(async move { spoke_ok(args) })
    }));
    let (forward, reverse) = tokio::join!(
        client.invoke_tool("tools.echo.echo", json!({ "v": 1 })),
        responder.issue_invoke("tools.math.add", json!({ "a": 10, "b": 32 }), None),
    );
    match reverse {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("reverse invoke must succeed: {reject:?}"),
    }
    match forward {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "v": 1 })),
        SpokeResult::Reject(reject) => panic!("forward invoke must succeed: {reject:?}"),
    }
    assert_eq!(
        calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 10, "b": 32 })]
    );
    client.close();
    responder.close();
}

#[tokio::test]
async fn denies_a_reverse_invoke_with_op_unsupported_when_no_handler_is_registered() {
    let (client, responder) = dial_with_tools(None, None, None).await;
    // The tool IS negotiated (both manifests list it) but no handler is
    // registered — gate passes, serving fails closed.
    let result = responder
        .issue_invoke("tools.math.add", json!({ "a": 1, "b": 2 }), None)
        .await;
    match result {
        SpokeResult::Ok(_) => panic!("no-handler reverse invoke must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("no handler registered"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
    }
    // The session stays usable: a registered handler serves the next
    // reverse invoke.
    client.register_tool_handler(
        "tools.math.add",
        add_handler(Arc::new(Mutex::new(Vec::new()))),
    );
    let retry = responder
        .issue_invoke("tools.math.add", json!({ "a": 4, "b": 5 }), None)
        .await;
    match retry {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 9 })),
        SpokeResult::Reject(reject) => panic!("retry must succeed: {reject:?}"),
    }
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn denies_a_reverse_invoke_with_op_unsupported_when_the_tool_is_not_negotiated() {
    // The client manifest carries no tools: the negotiated set lacks the
    // tool capability, so the client's dispatch gate denies the invoke
    // (frozen deny matrix: gate fail → op_unsupported).
    let (client, responder) = dial_with_tools(Some(no_tools_manifest("test-client")), None, None).await;
    let result = responder
        .issue_invoke("tools.math.add", json!({ "a": 1, "b": 2 }), None)
        .await;
    match result {
        SpokeResult::Ok(_) => panic!("not-negotiated reverse invoke must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("not authorized"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
    }
    assert_eq!(
        responder.stats.responses_verified.load(Ordering::SeqCst),
        1
    );
    client.close();
    responder.close();
}

#[tokio::test]
async fn rejects_a_tampered_reverse_request_with_auth_failed_and_does_not_advance_the_inbound_counter()
{
    let calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (client, responder) = dial_with_tools(
        None,
        Some(Box::new(|server_end| {
            Arc::new(TamperFirstRequestTransport {
                inner: server_end,
                tampered: AtomicBool::new(false),
            })
        })),
        None,
    )
    .await;
    client.register_tool_handler("tools.math.add", add_handler(Arc::clone(&calls)));

    // Tampered request (sequence 0): envelope-auth verify fails BEFORE
    // advance — the error branch is auth_failed with the locked
    // details.kind, and no handler side effect runs.
    let tampered_result = responder
        .issue_invoke("tools.math.add", json!({ "a": 1, "b": 2 }), Some(0))
        .await;
    match tampered_result {
        SpokeResult::Ok(_) => panic!("tampered reverse invoke must reject"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str),
                Some("envelope_auth_invalid")
            );
        }
    }
    assert!(calls.lock().expect("calls lock").is_empty());

    // Auth-before-advance: the client's inbound counter is UNCHANGED (still
    // expects 0), so re-issuing with the same sequence succeeds and the
    // handler runs — the session stayed usable and no state was mutated by
    // the forged envelope.
    let retry = responder
        .issue_invoke("tools.math.add", json!({ "a": 20, "b": 22 }), Some(0))
        .await;
    match retry {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("retry must succeed: {reject:?}"),
    }
    assert_eq!(
        calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 20, "b": 22 })]
    );
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn rejects_a_sequence_gap_reverse_invoke_with_invalid_sequence_and_does_not_advance_the_counter(
) {
    let calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (client, responder) = dial_with_tools(None, None, None).await;
    client.register_tool_handler("tools.math.add", add_handler(Arc::clone(&calls)));

    // The responder jumps to sequence 5; the client expects 0. The peek
    // fails — error branch invalid_sequence, counter unchanged, no handler
    // side effect.
    let gap = responder
        .issue_invoke("tools.math.add", json!({ "a": 1, "b": 1 }), Some(5))
        .await;
    match gap {
        SpokeResult::Ok(_) => panic!("sequence-gap reverse invoke must reject"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("invalid_sequence")
            );
        }
    }
    assert!(calls.lock().expect("calls lock").is_empty());

    // The inbound counter is still at 0: the next expected sequence
    // succeeds.
    let retry = responder
        .issue_invoke("tools.math.add", json!({ "a": 40, "b": 2 }), Some(0))
        .await;
    match retry {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("retry must succeed: {reject:?}"),
    }
    assert_eq!(
        calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 40, "b": 2 })]
    );
    client.close();
    responder.close();
}

#[tokio::test]
async fn answers_the_error_branch_when_a_handler_panics_without_loop_damage() {
    let (client, responder) = dial_with_tools(None, None, None).await;
    client.register_tool_handler("tools.echo.boom", Arc::new(|_args: Value| {
        Box::pin(async move {
            panic!("provider exploded");
        })
    }));
    client.register_tool_handler(
        "tools.math.add",
        add_handler(Arc::new(Mutex::new(Vec::new()))),
    );

    // A panicking handler answers the error branch (mapped via the
    // INTERNAL_ERROR error envelope) instead of crashing the loop.
    let thrown = responder
        .issue_invoke("tools.echo.boom", json!({}), None)
        .await;
    match thrown {
        SpokeResult::Ok(_) => panic!("panicking handler must answer the error branch"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject.message, "provider exploded");
        }
    }
    // Loop damage check: the receive loop survived the panic — a different
    // reverse invoke for a healthy handler still succeeds.
    let healthy = responder
        .issue_invoke("tools.math.add", json!({ "a": 40, "b": 2 }), None)
        .await;
    match healthy {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("healthy handler must succeed: {reject:?}"),
    }
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn answers_the_error_branch_with_the_handlers_spoke_reject_code() {
    let (client, responder) = dial_with_tools(None, None, None).await;
    client.register_tool_handler("tools.echo.echo", Arc::new(|_args: Value| {
        Box::pin(async move {
            SpokeResult::<Value>::Reject(SpokeReject {
                code: SpokeRejectCode::RevisionConflict,
                message: "the tool's backing store has a newer revision".to_owned(),
                details: None,
            })
        })
    }));
    let result = responder
        .issue_invoke("tools.echo.echo", json!({ "v": 1 }), None)
        .await;
    match result {
        SpokeResult::Ok(_) => panic!("handler reject must surface"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::RevisionConflict);
            assert_eq!(reject.message, "the tool's backing store has a newer revision");
        }
    }
    client.close();
    responder.close();
}

#[tokio::test]
async fn forwards_a_tool_invoke_via_invoke_tool_to_the_responders_handler() {
    let responder_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (client, responder) = dial_with_tools(None, None, None).await;
    responder.register_tool_handler("tools.math.add", add_handler(Arc::clone(&responder_calls)));
    let result = client
        .invoke_tool("tools.math.add", json!({ "a": 21, "b": 21 }))
        .await;
    match result {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("forward invoke must succeed: {reject:?}"),
    }
    assert_eq!(
        responder_calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 21, "b": 21 })]
    );
    assert_eq!(responder.stats.handlers_run.load(Ordering::SeqCst), 1);
    assert_eq!(responder.stats.sequence_rejections.load(Ordering::SeqCst), 0);
    assert_eq!(responder.stats.auth_rejections.load(Ordering::SeqCst), 0);
    client.close();
    responder.close();
}

#[tokio::test]
async fn maps_a_forward_invoke_tool_deny_to_capability_port_missing_with_wire_code_preserved() {
    let (client, responder) = dial_with_tools(None, None, None).await;
    // tools.echo.boom is negotiated but the responder serves no handler for
    // it — fail-closed deny → op_unsupported → D7 mapping.
    let result = client.invoke_tool("tools.echo.boom", json!({})).await;
    match result {
        SpokeResult::Ok(_) => panic!("forward deny must surface"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("no handler registered"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
    }
    client.close();
    responder.close();
}

#[tokio::test]
async fn rejects_a_forward_invoke_tool_whose_success_payload_lacks_a_result_key() {
    // The responder answers a success envelope whose payload is not
    // `{ result: <opaque JSON> }` — the frozen tool success-payload gate
    // must reject instead of surfacing spokeOk(garbage).
    let (client, responder) = dial_with_tools(
        None,
        None,
        Some(Box::new(|request: &ConnectInvokeRequest| {
            Some(json!({
                "session_id": request.session_id,
                "sequence": request.sequence,
                "request_id": request.request_id,
                "payload": { "garbage": true },
                "extensions": {},
            }))
        })),
    )
    .await;
    let result = client
        .invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 }))
        .await;
    match result {
        SpokeResult::Ok(_) => panic!("malformed success payload must reject"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert!(reject.message.contains("payload decode failed"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str),
                Some("transport")
            );
        }
    }
    // The malformed payload fails only this waiter — the session stays
    // usable.
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn invoke_tool_fails_fast_with_invalid_input_on_a_non_tool_capability_id() {
    let (client, responder) = dial_with_tools(None, None, None).await;
    let result = client.invoke_tool("spoke-baseline", json!({})).await;
    match result {
        SpokeResult::Ok(_) => panic!("non-tool capability id must reject"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            assert!(reject.message.contains("tools.\" prefix"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("capability_id"))
                    .and_then(Value::as_str),
                Some("spoke-baseline")
            );
        }
    }
    // No wire traffic: the grammar gate is local.
    assert_eq!(
        responder.stats.reverse_invokes_issued.load(Ordering::SeqCst),
        0
    );
    client.close();
    responder.close();
}

// ── connect_responder (mirror of TS responder.test.ts) ────────────────────

/// Transport wrapper hook (wire-level injector fixtures).
type TransportWrap = Box<dyn Fn(Arc<dyn Transport>) -> Arc<dyn Transport> + Send + Sync>;

/// Dial options for the responder loopback pair.
#[derive(Default)]
struct ResponderDialOptions {
    client_manifest: Option<HostCapabilityManifest>,
    responder_manifest: Option<HostCapabilityManifest>,
    ports: Option<Arc<dyn BaselinePorts + Send + Sync>>,
    responder_timeout_ms: Option<u64>,
    responder_transport: Option<TransportWrap>,
}

/// Loopback pair: `connect_responder` (server end) + real
/// `connect_remote_adapter` (client end). The responder's handshake runs in
/// the background; the client's dial is the synchronization point.
async fn dial_with_responder(
    options: ResponderDialOptions,
) -> (Arc<ConnectResponder>, Arc<RemoteAdapter>, LoopbackTransportPair) {
    let pair = loopback_transport_pair();
    let server_end: Arc<dyn Transport> = match options.responder_transport {
        Some(wrap) => wrap(Arc::new(pair.server.clone())),
        None => Arc::new(pair.server.clone()),
    };
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let peer_id_responder = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let responder = connect_responder(ConnectResponderOptions {
        transport: server_end,
        identity: RemoteIdentity {
            seed: seed_host(),
        },
        manifest: options
            .responder_manifest
            .unwrap_or_else(|| tool_manifest("test-responder")),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client.clone(), pubkey_client())]),
        ports: options.ports,
        invoke_timeout_ms: options.responder_timeout_ms,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client.clone()),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: options
            .client_manifest
            .unwrap_or_else(|| tool_manifest("test-client")),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_responder.clone()],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial");
    (responder, client, pair)
}

#[tokio::test]
async fn responder_establishes_a_session_with_a_real_dial_and_discovery_after_auth() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let peer_id_responder = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    assert_eq!(responder.session_id(), client.session_id());
    // Session peer binding: the responder's remote peer is the dialer.
    assert_eq!(responder.remote_peer_id(), Some(peer_id_client));
    assert_eq!(client.remote_peer_id(), Some(peer_id_responder));
    // Discovery after auth: the authenticated hello `host` is the source —
    // the responder sees the dialer's tools[] only once the signed-hello
    // handshake completed.
    let remote_manifest = responder.remote_manifest().expect("remote manifest");
    let tool_ids: Vec<&str> = remote_manifest
        .tools
        .iter()
        .map(|tool| tool.capability_id.as_str())
        .collect();
    assert_eq!(
        tool_ids,
        vec!["tools.math.add", "tools.echo.echo", "tools.echo.boom"]
    );
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_rejects_a_non_allowlisted_peer_during_the_handshake() {
    let pair = loopback_transport_pair();
    let seed_stranger = seed(0x70);
    let pubkey_stranger = ed25519_dalek::SigningKey::from_bytes(&seed_stranger)
        .verifying_key()
        .to_bytes();
    let peer_id_stranger = derive_peer_id_from_ed25519_pubkey(&pubkey_stranger);
    // The server-side allowlist names the STRANGER; the dialing client uses
    // the regular fixture identity, which is not on it.
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server),
        identity: RemoteIdentity {
            seed: seed_host(),
        },
        manifest: tool_manifest("test-responder"),
        allowlist: vec![peer_id_stranger.clone()],
        peer_keys: HashMap::from([(peer_id_stranger.clone(), pubkey_stranger)]),
        ports: Some(Arc::new(ToyWorldAdapter::with_committed_fixtures())),
        invoke_timeout_ms: None,
    })
    .await;
    // The server-side allowlist rejects the hello and closes the transport,
    // failing the dial fast.
    let dial_result = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: tool_manifest("test-client"),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await;
    assert!(dial_result.is_err(), "dial must fail closed");
    until_state(
        || responder.state(),
        RemoteAdapterState::Closed,
        "responder",
    )
    .await;
    responder.close();
}

#[tokio::test]
async fn responder_round_trips_port_ops_into_the_injected_baseline_ports() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        ports: Some(Arc::new(ToyWorldAdapter::with_committed_fixtures())),
        ..ResponderDialOptions::default()
    })
    .await;
    // port.knowledge.get — seeded fixture round-trip.
    let mira = client.get_knowledge_entry("kb_tw_mira").await;
    match mira {
        SpokeResult::Ok(entry) => assert_eq!(entry.entry_id.as_str(), "kb_tw_mira"),
        SpokeResult::Reject(reject) => panic!("port.knowledge.get must succeed: {reject:?}"),
    }
    // port.knowledge.put — create (expected_base_revision null), then a
    // compare-and-swap update over the wire (the toy-world store treats an
    // absent revision as 0, so base 0 accepts the update).
    let compass: KnowledgeEntry = serde_json::from_value(json!({
        "schema_version": 1,
        "entry_id": "test-harbor/item/compass",
        "entry_type": "item",
        "canonical_name": "Compass",
        "status": "provisional",
        "body": { "summary": "A brass compass." },
        "extensions": {},
    }))
    .expect("valid KnowledgeEntry");
    let created = client.put_knowledge_entry(compass.clone(), None).await;
    match created {
        SpokeResult::Ok(entry) => assert_eq!(entry.entry_id.as_str(), compass.entry_id.as_str()),
        SpokeResult::Reject(reject) => panic!("port.knowledge.put create must succeed: {reject:?}"),
    }
    let mut updated_compass = compass.clone();
    updated_compass.status = serde_json::from_value(json!("confirmed")).expect("status");
    let updated = client.put_knowledge_entry(updated_compass, Some(0)).await;
    match updated {
        SpokeResult::Ok(entry) => assert_eq!(entry.status.as_str(), "confirmed"),
        SpokeResult::Reject(reject) => panic!("CAS update must succeed: {reject:?}"),
    }
    // Negative OCC over the wire: re-creating an existing entry rejects
    // REVISION_CONFLICT through the responder's error branch.
    let conflicted = client.put_knowledge_entry(compass.clone(), None).await;
    match conflicted {
        SpokeResult::Ok(_) => panic!("OCC conflict must reject"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::RevisionConflict)
        }
    }
    // port.scope.list_knowledge_entries — includes the created entry.
    let scope: Scope = serde_json::from_value(json!({ "scope_id": "toy-scope-001" }))
        .expect("valid Scope");
    let listed = client.list_knowledge_entries(&scope).await;
    match listed {
        SpokeResult::Ok(entries) => assert!(
            entries.iter().any(|entry| entry.entry_id.as_str() == compass.entry_id.as_str())
        ),
        SpokeResult::Reject(reject) => panic!("list must succeed: {reject:?}"),
    }
    // port.host.list_peer_manifests — the adapter's product-seeded peers.
    let peers = client.list_peer_host_capability_manifests().await;
    match peers {
        SpokeResult::Ok(manifests) => {
            let host_ids: Vec<&str> = manifests
                .iter()
                .map(|manifest| manifest.host_id.as_str())
                .collect();
            assert_eq!(host_ids, vec!["host_tw_peer"]);
        }
        SpokeResult::Reject(reject) => panic!("peer manifests must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_denies_port_invokes_with_dispatch_deny_when_ports_are_absent() {
    // No `ports` injected: the capability gate passes (spoke-baseline is
    // negotiated) but there is no BaselinePorts to serve — the responder
    // answers the dispatch-deny branch, mapped by the D7 invoker row.
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    match result {
        SpokeResult::Ok(_) => panic!("absent-ports port invoke must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("no BaselinePorts configured"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_issues_a_reverse_invoke_served_by_the_dialers_registered_handler() {
    let calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    client.register_tool_handler("tools.math.add", add_handler(Arc::clone(&calls)));
    let result = responder
        .invoke_tool("tools.math.add", json!({ "a": 2, "b": 3 }))
        .await;
    match result {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 5 })),
        SpokeResult::Reject(reject) => panic!("reverse invoke must succeed: {reject:?}"),
    }
    // The dialer-side registered handler is what ran, with the request's
    // arguments object passed through.
    assert_eq!(
        calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 2, "b": 3 })]
    );
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_maps_a_deny_no_handler_to_capability_port_missing_with_wire_code() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    // The tool IS negotiated but the dialer serves no handler for it —
    // fail-closed serving → op_unsupported → D7 mapping.
    let result = responder
        .invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 }))
        .await;
    match result {
        SpokeResult::Ok(_) => panic!("no-handler deny must surface"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("no handler registered"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_maps_a_deny_not_negotiated_to_capability_port_missing() {
    // The client manifest carries no tools: the negotiated set lacks the
    // tool capability, so the dialer's dispatch gate denies the reverse
    // invoke (frozen deny matrix: gate fail → op_unsupported).
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        client_manifest: Some(no_tools_manifest("test-client")),
        ..ResponderDialOptions::default()
    })
    .await;
    let result = responder
        .invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 }))
        .await;
    match result {
        SpokeResult::Ok(_) => panic!("not-negotiated deny must surface"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("not authorized"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
    }
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_invoke_tool_fails_fast_with_invalid_input_on_a_non_tool_capability_id() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    let result = responder.invoke_tool("spoke-baseline", json!({})).await;
    match result {
        SpokeResult::Ok(_) => panic!("non-tool capability id must reject"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            assert!(reject.message.contains("tools.\" prefix"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("capability_id"))
                    .and_then(Value::as_str),
                Some("spoke-baseline")
            );
        }
    }
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_times_out_the_waiter_without_closing_the_session() {
    // The dialer's handler never resolves: the request DID hit the wire
    // (outbound sequence transmitted), so the waiter times out but the
    // session stays usable on both ends — no poison-close.
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        responder_timeout_ms: Some(100),
        ..ResponderDialOptions::default()
    })
    .await;
    client.register_tool_handler("tools.echo.boom", Arc::new(|_args: Value| {
        Box::pin(async move { futures::future::pending::<SpokeResult<Value>>().await })
    }));
    let timed_out = responder
        .invoke_tool("tools.echo.boom", json!({}))
        .await;
    match timed_out {
        SpokeResult::Ok(_) => panic!("never-settling handler must time out"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert!(reject.message.contains("timed out after 100ms"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str),
                Some("timeout")
            );
        }
    }
    // The session stays usable: a follow-up reverse invoke for a resolving
    // handler succeeds.
    client.register_tool_handler(
        "tools.math.add",
        add_handler(Arc::new(Mutex::new(Vec::new()))),
    );
    let retry = responder
        .invoke_tool("tools.math.add", json!({ "a": 40, "b": 2 }))
        .await;
    match retry {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("retry must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

/// Wire-level injector on the RESPONDER end: the FIRST outbound reverse
/// invoke's send is delayed on the wire (200ms — far outside the 100ms
/// invoke timeout), so the second reverse invoke times out while queued
/// behind it in the send tail.
struct DelayedFirstSendTransport {
    inner: Arc<dyn Transport>,
    sent_sequences: Arc<Mutex<Vec<i64>>>,
    first_delayed: AtomicBool,
}

#[async_trait]
impl Transport for DelayedFirstSendTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        let doc: Value = serde_json::from_slice(envelope)
            .map_err(|error| TransportError::Io(error.to_string()))?;
        if doc.get("op").is_some() {
            if !self.first_delayed.swap(true, Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let sequence = doc.get("sequence").and_then(Value::as_i64).unwrap_or(-1);
            self.sent_sequences.lock().expect("sent lock").push(sequence);
        }
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.inner.recv().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

#[tokio::test]
async fn responder_closes_the_session_when_a_timed_out_queued_reverse_invoke_send_is_skipped() {
    // Mirror of the TS poison-close test: the FIRST reverse invoke's send is
    // delayed on the wire; the SECOND reverse invoke's send is serialized
    // behind it (send tail) and times out while waiting — before its send
    // ever starts. When the first send finally completes, the queued send is
    // skipped: its waiter already settled, so transmitting it late would be
    // a duplicate dispatch on the dialer. The skip must instead close the
    // session (the allocated outbound sequence never hit the wire — the
    // dialer's inbound gate would be stuck at it).
    let sent_sequences: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        responder_timeout_ms: Some(100),
        responder_transport: Some(Box::new({
            let sent_sequences = Arc::clone(&sent_sequences);
            move |server_end| {
                Arc::new(DelayedFirstSendTransport {
                    inner: server_end,
                    sent_sequences: Arc::clone(&sent_sequences),
                    first_delayed: AtomicBool::new(false),
                })
            }
        })),
        ..ResponderDialOptions::default()
    })
    .await;
    client.register_tool_handler(
        "tools.math.add",
        add_handler(Arc::new(Mutex::new(Vec::new()))),
    );
    let (first, second) = tokio::join!(
        responder.invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 })), // sequence 0 — send delayed
        responder.invoke_tool("tools.math.add", json!({ "a": 3, "b": 4 })), // sequence 1 — times out queued behind it
    );
    for result in [&first, &second] {
        match result {
            SpokeResult::Ok(_) => panic!("poison-close invokes must time out"),
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InternalError);
                assert!(reject.message.contains("timed out after 100ms"));
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(Value::as_str),
                    Some("timeout")
                );
            }
        }
    }
    // The delayed first send completes, the queued second send is skipped,
    // and the skip closes the session.
    until_state(
        || responder.state(),
        RemoteAdapterState::Closed,
        "responder",
    )
    .await;
    // Only the first reverse invoke ever reached the wire; the timed-out-
    // while-queued second invoke was never transmitted.
    assert_eq!(
        sent_sequences.lock().expect("sent lock").as_slice(),
        &[0]
    );
    // The session is closed, not poisoned: a follow-up reverse invoke fails
    // with session_closed instead of hanging or being mis-rejected by the
    // dialer's stuck inbound gate.
    let after = responder
        .invoke_tool("tools.math.add", json!({ "a": 5, "b": 6 }))
        .await;
    match after {
        SpokeResult::Ok(_) => panic!("post-close invoke must fail"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert!(reject.message.contains("connect session is not established"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str),
                Some("session_closed")
            );
        }
    }
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_serves_a_forward_invoke_tool_through_a_registered_handler() {
    let responder_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    responder.register_tool_handler("tools.math.add", add_handler(Arc::clone(&responder_calls)));
    let result = client
        .invoke_tool("tools.math.add", json!({ "a": 21, "b": 21 }))
        .await;
    match result {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("forward invoke must succeed: {reject:?}"),
    }
    assert_eq!(
        responder_calls.lock().expect("calls lock").as_slice(),
        &[json!({ "a": 21, "b": 21 })]
    );
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_answers_the_error_branch_when_a_served_handler_panics_without_loop_damage() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    responder.register_tool_handler("tools.echo.boom", Arc::new(|_args: Value| {
        Box::pin(async move {
            panic!("provider exploded");
        })
    }));
    let thrown = client.invoke_tool("tools.echo.boom", json!({})).await;
    match thrown {
        SpokeResult::Ok(_) => panic!("panicking handler must answer the error branch"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject.message, "provider exploded");
        }
    }
    // Loop damage check: the responder's serve loop survived — a different
    // forward invoke for a healthy handler still succeeds.
    responder.register_tool_handler(
        "tools.math.add",
        add_handler(Arc::new(Mutex::new(Vec::new()))),
    );
    let healthy = client
        .invoke_tool("tools.math.add", json!({ "a": 40, "b": 2 }))
        .await;
    match healthy {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 42 })),
        SpokeResult::Reject(reject) => panic!("healthy handler must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_rejects_register_tool_handler_for_a_non_tool_capability_id() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions::default()).await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        responder.register_tool_handler(
            "spoke-baseline",
            Arc::new(|_args: Value| Box::pin(async move { spoke_ok(Value::Null) })),
        );
    }));
    assert!(
        result.is_err(),
        "non-tool register_tool_handler must panic (grammar gate)"
    );
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_still_establishes_on_an_empty_capabilities_intersection() {
    // Disjoint capability sets: the negotiated intersection is empty, so
    // the responder's signed session snapshot must fall back to
    // `["spoke-baseline"]` (wire minItems 1). The dialer computes its own
    // intersection for gating, so the fallback has no authorization impact —
    // the dial must simply establish.
    let client_only: HostCapabilityManifest = serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": "test-client",
        "roles": ["data-store"],
        "capabilities": ["client-only-capability"],
        "namespaces": ["toy_world"],
        "extensions": {},
    }))
    .expect("valid manifest");
    let responder_only: HostCapabilityManifest = serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": "test-responder",
        "roles": ["data-store"],
        "capabilities": ["responder-only-capability"],
        "namespaces": ["toy_world"],
        "extensions": {},
    }))
    .expect("valid manifest");
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        client_manifest: Some(client_only),
        responder_manifest: Some(responder_only),
        ..ResponderDialOptions::default()
    })
    .await;
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    assert_eq!(responder.session_id(), client.session_id());
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_closes_the_connection_on_an_unparseable_inbound_frame() {
    let (responder, client, pair) = dial_with_responder(ResponderDialOptions::default()).await;
    // A frame that fails JSON decode is a protocol violation: the
    // responder's serve loop must actually close the transport (the
    // carried-over demo behavior) — a bare return would leave the client's
    // established session hanging on its next invoke.
    let _ = pair.client.send(b"not json {{{").await;
    until_state(
        || responder.state(),
        RemoteAdapterState::Closed,
        "responder",
    )
    .await;
    // The dialer observes transport loss and closes too.
    until_state(
        || client.state(),
        RemoteAdapterState::Closed,
        "client",
    )
    .await;
    client.close();
    responder.close();
}

// ── connect_responder per-invoke gate (raw-wire fixtures) ─────────────────

/// Start a responder WITHOUT dialing (raw-wire tests drive the wire).
async fn start_raw_responder() -> (
    Arc<ConnectResponder>,
    LoopbackTransportPair,
    [u8; 32],
    String,
) {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity {
            seed: seed_host(),
        },
        manifest: tool_manifest("test-responder"),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client.clone(), pubkey_client())]),
        ports: Some(Arc::new(ToyWorldAdapter::with_committed_fixtures())),
        invoke_timeout_ms: None,
    })
    .await;
    (responder, pair, seed_client(), peer_id_client)
}

/// Raw initiator handshake (the real library client is exercised elsewhere):
/// send a signed initiator hello, consume the responder hello + snapshot,
/// and return the assigned session id.
async fn raw_handshake(
    client: &LoopbackTransport,
    seed: [u8; 32],
    manifest: &HostCapabilityManifest,
) -> String {
    let nonce = "raw-handshake-nonce-0000".to_owned();
    let hello = sign_hello_ed25519(&seed, &nonce, &connect_manifest(manifest), None)
        .expect("initiator hello sign");
    client
        .send(&serde_json::to_vec(&hello).expect("hello bytes"))
        .await
        .expect("hello send");
    let _responder_hello: ConnectHello = serde_json::from_slice(
        &client.recv().await.expect("responder hello recv"),
    )
    .expect("responder hello decode");
    let bytes = client.recv().await.expect("snapshot recv");
    let session: ConnectSession = serde_json::from_slice(&bytes).expect("session decode");
    session.session_id.to_string()
}

/// Sign a raw wire `ConnectInvokeRequest` over the locked 5-field set.
fn sign_invoke_request(
    seed: [u8; 32],
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let signed_object = json!({
        "session_id": session_id,
        "sequence": sequence,
        "request_id": request_id,
        "op": op,
        "payload": payload,
    });
    let signature = sign_envelope(&seed, &signed_object);
    let mut wire = signed_object
        .as_object()
        .expect("signed object is an object")
        .clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    Value::Object(wire)
}

#[tokio::test]
async fn responder_rejects_a_sequence_gap_invoke_with_invalid_sequence_and_no_advance() {
    let (responder, pair, seed_client, _peer_id_client) = start_raw_responder().await;
    let session_id = raw_handshake(&pair.client, seed_client, &tool_manifest("test-client")).await;
    // A wire-valid, properly signed invoke at a non-expected sequence: the
    // peek fails — invalid_sequence, counter unchanged.
    let gap = sign_invoke_request(
        seed_client,
        &session_id,
        5,
        "seq-gap",
        "port.knowledge.get",
        json!({ "entry_id": "kb_tw_mira" }),
    );
    pair.client
        .send(&serde_json::to_vec(&gap).expect("bytes"))
        .await
        .expect("send");
    let rejection: Value =
        serde_json::from_slice(&pair.client.recv().await.expect("recv")).expect("decode");
    assert_eq!(rejection["error"]["code"], "invalid_sequence");

    // The inbound counter is still at 0: a valid invoke at sequence 0
    // dispatches and succeeds.
    let valid = sign_invoke_request(
        seed_client,
        &session_id,
        0,
        "valid-after-gap",
        "port.knowledge.get",
        json!({ "entry_id": "kb_tw_mira" }),
    );
    pair.client
        .send(&serde_json::to_vec(&valid).expect("bytes"))
        .await
        .expect("send");
    let ok_response: Value =
        serde_json::from_slice(&pair.client.recv().await.expect("recv")).expect("decode");
    assert_eq!(ok_response["payload"]["entry_id"], "kb_tw_mira");
    responder.close();
}

#[tokio::test]
async fn responder_rejects_a_tampered_invoke_with_auth_failed_and_no_advance() {
    let (responder, pair, seed_client, _peer_id_client) = start_raw_responder().await;
    let session_id = raw_handshake(&pair.client, seed_client, &tool_manifest("test-client")).await;
    // Wire-level tamper: mutate the payload AFTER signing — the envelope-auth
    // verify must fail BEFORE advance (auth-before-advance), answering
    // auth_failed with the locked details.kind.
    let mut tampered = sign_invoke_request(
        seed_client,
        &session_id,
        0,
        "tampered",
        "port.knowledge.get",
        json!({ "entry_id": "kb_tw_mira" }),
    );
    tampered["payload"]["tampered"] = json!(true);
    pair.client
        .send(&serde_json::to_vec(&tampered).expect("bytes"))
        .await
        .expect("send");
    let rejection: Value =
        serde_json::from_slice(&pair.client.recv().await.expect("recv")).expect("decode");
    assert_eq!(rejection["error"]["code"], "auth_failed");
    assert_eq!(
        rejection["error"]["details"]["kind"],
        "envelope_auth_invalid"
    );

    // Auth-before-advance: the inbound counter is UNCHANGED, so the same
    // sequence re-issued with a valid signature succeeds.
    let retry = sign_invoke_request(
        seed_client,
        &session_id,
        0,
        "valid-after-tamper",
        "port.knowledge.get",
        json!({ "entry_id": "kb_tw_mira" }),
    );
    pair.client
        .send(&serde_json::to_vec(&retry).expect("bytes"))
        .await
        .expect("send");
    let ok_response: Value =
        serde_json::from_slice(&pair.client.recv().await.expect("recv")).expect("decode");
    assert_eq!(ok_response["payload"]["entry_id"], "kb_tw_mira");
    responder.close();
}

#[tokio::test]
async fn responder_answers_an_unknown_port_method_with_the_dispatch_deny_branch() {
    let (responder, pair, seed_client, _peer_id_client) = start_raw_responder().await;
    let session_id = raw_handshake(&pair.client, seed_client, &tool_manifest("test-client")).await;
    // A wire-valid, properly signed invoke for an op outside the D4
    // catalogue: the dispatch gate denies it (no core row, no product map
    // row) with the existing `op_unsupported` error branch.
    let unknown_op = sign_invoke_request(
        seed_client,
        &session_id,
        0,
        "unknown-port-op",
        "port.nope",
        json!({}),
    );
    pair.client
        .send(&serde_json::to_vec(&unknown_op).expect("bytes"))
        .await
        .expect("send");
    let response: Value =
        serde_json::from_slice(&pair.client.recv().await.expect("recv")).expect("decode");
    assert_eq!(response["error"]["code"], "op_unsupported");
    assert!(
        response["error"]["message"]
            .as_str()
            .expect("message")
            .contains("port.nope")
    );
    assert_eq!(response["signature"].as_str().expect("sig").len(), 86);
    responder.close();
}
