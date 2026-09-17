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
    connect_multi_peer_router, connect_remote_adapter, connect_responder, loopback_transport_pair,
    reset_accepted_server_hellos_for_test, ConnectResponder, ConnectResponderOptions,
    LoopbackTransport, LoopbackTransportPair, MultiPeerRouterOptions, RemoteAdapter,
    RemoteAdapterError, RemoteAdapterOptions, RemoteAdapterState, RemoteExtractService,
    RemoteIdentity, RemoteServePorts, RemoteServePortsComposite, ToolHandler, Transport,
    TransportError,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_operations::{
    orchestrate_check, orchestrate_extract, orchestrate_upsert, spoke_ok, spoke_reject,
    BaselinePorts, CheckRunInput, ComputablePort, ExtractRunInput, ExtractionPort,
    ExtractionResult, FindingPort, ForkTimelineQueryPort, HostManifestPort, KnowledgeEntryPort,
    RelationPort, RuleQueryPort, ScopeQueryPort, SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::host_capability_manifest::HostCapabilityManifestExtensionsKey;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::connect::ConnectSession;
use spoke_schemas::{
    CheckRequest, ComputeRequest, ComputeResponse, ExtractRequest, ExtractResponse, Finding,
    HostCapabilityManifest, KnowledgeEntry, ProjectRequest, ProjectResponse, Relation, Rule, Scope,
    TimelineEvent, UpsertRequest,
};
use tokio::sync::Notify;

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
async fn adapter_answers_invalid_sequence_for_a_present_but_non_numeric_sequence_reverse_invoke() {
    // Parity with the TS gate (deny observability) on the ADAPTER's reverse
    // gate: a PRESENT but non-numeric `sequence` on a reverse invoke is a
    // malformed wire request — deny `invalid_sequence` instead of silently
    // ignoring it as `Stray` (a silent ignore makes the sender wait out its
    // timeout for no answer). The test drives the real
    // `connect_remote_adapter` against a raw test-side peer (signed hello +
    // session snapshot handshake), then injects the malformed frame.
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let peer_id_responder = derive_peer_id_from_ed25519_pubkey(&pubkey_host());

    let dial = {
        let peer_id_responder = peer_id_responder.clone();
        tokio::spawn(async move {
            connect_remote_adapter(RemoteAdapterOptions {
                transport: Arc::new(pair.client.clone()),
                local_identity: RemoteIdentity {
                    seed: seed_client(),
                },
                local_manifest: tool_manifest("test-client"),
                remote_pubkey: pubkey_host(),
                allowlist: vec![peer_id_responder],
                invoke_timeout_ms: Some(5000),
                capability_token: None,
            })
            .await
        })
    };

    // Raw responder-side handshake (mirror of the minimal responder's):
    // read the initiator hello, answer with a dial-bound responder hello
    // (`peer_nonce` = initiator nonce) + signed session snapshot.
    let initiator_hello: ConnectHello = serde_json::from_slice(
        &pair.server.recv().await.expect("initiator hello recv"),
    )
    .expect("initiator hello decode");
    assert_eq!(initiator_hello.peer_id.as_str(), peer_id_client.as_str());
    let nonce = "raw-peer-nonce-0001".to_owned();
    let responder_hello = sign_hello_ed25519(
        &seed_host(),
        &nonce,
        &serde_json::from_value(
            serde_json::to_value(tool_manifest("test-responder")).expect("manifest serializes"),
        )
        .expect("manifest converts"),
        Some(initiator_hello.nonce.as_str()),
    )
    .expect("responder hello sign");
    pair.server
        .send(&serde_json::to_vec(&responder_hello).expect("bytes"))
        .await
        .expect("send");
    let snapshot = json!({
        "session_id": format!("connect-responder-session-{peer_id_client}"),
        "initiator_peer_id": peer_id_client,
        "responder_peer_id": peer_id_responder,
        "opened_at": "2026-01-01T00:00:00Z",
        "negotiated_capabilities": [
            "spoke-baseline",
            "tools.math.add",
            "tools.echo.echo",
            "tools.echo.boom",
        ],
        "initial_sequence": 0,
    });
    let signature = sign_envelope(&seed_host(), &snapshot);
    let mut wire = snapshot.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    pair.server
        .send(&serde_json::to_vec(&Value::Object(wire)).expect("bytes"))
        .await
        .expect("send");

    let client = dial.await.expect("dial task").expect("dial");
    client.register_tool_handler(
        "tools.math.add",
        add_handler(Arc::new(Mutex::new(Vec::new()))),
    );
    let session_id = client.session_id().expect("session id");

    // A reverse invoke whose `sequence` is present but is a STRING: signed
    // over the exact wire object (the deny fires before the signature is
    // ever verified).
    let malformed = json!({
        "session_id": session_id,
        "sequence": "5",
        "request_id": "non-numeric-seq",
        "op": "tools.math.add",
        "payload": { "arguments": { "a": 1, "b": 2 } },
    });
    let signature = sign_envelope(&seed_host(), &malformed);
    let mut wire = malformed.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    pair.server
        .send(&serde_json::to_vec(&Value::Object(wire)).expect("bytes"))
        .await
        .expect("send");
    let rejection: Value =
        serde_json::from_slice(&pair.server.recv().await.expect("recv")).expect("decode");
    assert_eq!(rejection["error"]["code"], "invalid_sequence");

    // The inbound counter is still at 0: a valid reverse invoke at sequence
    // 0 dispatches to the registered handler and succeeds.
    let valid = sign_invoke_request(
        seed_host(),
        &session_id,
        0,
        "valid-after-non-numeric",
        "tools.math.add",
        json!({ "arguments": { "a": 2, "b": 3 } }),
    );
    pair.server
        .send(&serde_json::to_vec(&valid).expect("bytes"))
        .await
        .expect("send");
    let ok_response: Value =
        serde_json::from_slice(&pair.server.recv().await.expect("recv")).expect("decode");
    assert_eq!(ok_response["payload"]["result"]["sum"], 5);

    client.close();
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

// ── Multi-peer router tool routing (frozen §6, mirror of TS
//    multi-peer-router.test.ts) ────────────────────────────────────────────

/// Manifest carrying a `tools[]` array (frozen §2 descriptors).
fn manifest_with_tools(
    host_id: &str,
    capabilities: &[&str],
    namespaces: &[&str],
    tools: Value,
) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": capabilities,
        "namespaces": namespaces,
        "extensions": {},
        "tools": tools,
    }))
    .expect("valid HostCapabilityManifest with tools")
}

/// Dial a client (initiator) against a tool-serving minimal responder
/// double whose manifest carries ONLY `tools` — the peer's cached hello
/// manifest is exactly what the router's hard filter sees. Distinct host
/// seeds give distinct peer ids (tie-break). The client advertises the
/// full fixture tool set so the negotiated intersection includes every
/// tool under test.
async fn dial_tool_peer(
    host_seed: [u8; 32],
    responder_manifest: HostCapabilityManifest,
) -> (Arc<RemoteAdapter>, Arc<MinimalResponder>) {
    let pair = loopback_transport_pair();
    let host_pubkey = SigningKey::from_bytes(&host_seed)
        .verifying_key()
        .to_bytes();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let responder = start_minimal_responder(MinimalResponderOptions {
        transport: Arc::new(pair.server),
        seed: host_seed,
        client_pubkey: pubkey_client(),
        allowlist: vec![peer_id_client.clone()],
        manifest: responder_manifest,
        invoke_timeout_ms: None,
        response_override: None,
    })
    .await;
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&host_pubkey);
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: tool_manifest("test-client"),
        remote_pubkey: host_pubkey,
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial");
    (client, responder)
}

#[tokio::test]
async fn multi_peer_router_routes_tool_invokes_to_the_serving_responder() {
    // Router-registered adapters are LOCALLY-DIALED: router tool invokes
    // travel initiator→responder and are served by the PEER's responder-side
    // tool serving — the proof drives tool-serving responder doubles with
    // DISJOINT tool sets (the minimal responder's frozen §4 pipeline), not
    // dialer-side register_tool_handler alone.
    let add_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let echo_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    let (client_a, responder_a) = dial_tool_peer(
        [0xf1; 32],
        manifest_with_tools(
            "host-a",
            &["spoke-baseline", "tools.math.add"],
            &["math"],
            json!([add_descriptor()]),
        ),
    )
    .await;
    let (client_b, responder_b) = dial_tool_peer(
        [0xf2; 32],
        manifest_with_tools(
            "host-b",
            &["spoke-baseline", "tools.echo.echo"],
            &["echo"],
            json!([echo_descriptor()]),
        ),
    )
    .await;

    responder_a.register_tool_handler("tools.math.add", add_handler(add_calls.clone()));
    let echo_calls_served = Arc::clone(&echo_calls);
    responder_b.register_tool_handler(
        "tools.echo.echo",
        Arc::new(move |args: Value| {
            let echo_calls_served = Arc::clone(&echo_calls_served);
            Box::pin(async move {
                echo_calls_served.lock().expect("calls lock").push(args.clone());
                spoke_ok(args)
            })
        }),
    );

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    router
        .register_peer(client_a.clone())
        .expect("register a");
    router
        .register_peer(client_b.clone())
        .expect("register b");

    // tools.math.add is advertised + served only by responder A.
    let add = router
        .invoke_tool("tools.math.add", json!({ "a": 2, "b": 3 }))
        .await;
    match add {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "sum": 5 })),
        SpokeResult::Reject(reject) => panic!("add must route to responder A: {reject:?}"),
    }
    assert_eq!(add_calls.lock().expect("calls lock").len(), 1);
    assert!(echo_calls.lock().expect("calls lock").is_empty());

    // tools.echo.echo is advertised + served only by responder B.
    let echo = router
        .invoke_tool("tools.echo.echo", json!({ "v": 1 }))
        .await;
    match echo {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "v": 1 })),
        SpokeResult::Reject(reject) => panic!("echo must route to responder B: {reject:?}"),
    }
    assert_eq!(echo_calls.lock().expect("calls lock").len(), 1);
    assert_eq!(add_calls.lock().expect("calls lock").len(), 1);

    client_a.close();
    client_b.close();
    responder_a.close();
    responder_b.close();
}

#[tokio::test]
async fn multi_peer_router_rejects_no_capable_peer_for_an_unadvertised_tool() {
    let (client_a, responder_a) = dial_tool_peer(
        [0xf1; 32],
        manifest_with_tools(
            "host-a",
            &["spoke-baseline", "tools.math.add"],
            &["math"],
            json!([add_descriptor()]),
        ),
    )
    .await;
    let (client_b, responder_b) = dial_tool_peer(
        [0xf2; 32],
        manifest_with_tools(
            "host-b",
            &["spoke-baseline", "tools.echo.echo"],
            &["echo"],
            json!([echo_descriptor()]),
        ),
    )
    .await;

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    router
        .register_peer(client_a.clone())
        .expect("register a");
    router
        .register_peer(client_b.clone())
        .expect("register b");

    // No registered peer's cached manifest offers tools.echo.boom.
    let result = router.invoke_tool("tools.echo.boom", json!({})).await;
    match result {
        SpokeResult::Reject(reject) => {
            // §5 locked reject: CAPABILITY_PORT_MISSING + details.kind /
            // wire_code = no_capable_peer, details.op = capability_id.
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
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("op"))
                    .and_then(Value::as_str),
                Some("tools.echo.boom")
            );
        }
        SpokeResult::Ok(_) => panic!("no capable peer must reject"),
    }
    // Terminal: no responder served anything (selection rejects pre-wire).
    assert_eq!(
        responder_a.stats.handlers_run.load(Ordering::SeqCst),
        0
    );
    assert_eq!(
        responder_b.stats.handlers_run.load(Ordering::SeqCst),
        0
    );

    client_a.close();
    client_b.close();
    responder_a.close();
    responder_b.close();
}

#[tokio::test]
async fn multi_peer_router_composes_tools_union_over_loopback_peers() {
    // Frozen §6 over real signed hellos: `tools[]` unions across the
    // connected peers' cached manifests, dedup by `capability_id`
    // (tools.echo.echo is shared), lexicographic order for stability.
    let (peer_a_adapter, peer_a_responder) = dial_tool_peer(
        [0xf3; 32],
        manifest_with_tools(
            "host-a",
            &["spoke-baseline", "tools.math.add", "tools.echo.echo"],
            &["math", "echo"],
            json!([add_descriptor(), echo_descriptor()]),
        ),
    )
    .await;
    let (peer_b_adapter, peer_b_responder) = dial_tool_peer(
        [0xf4; 32],
        manifest_with_tools(
            "host-b",
            &["spoke-baseline", "tools.echo.echo", "tools.echo.boom"],
            &["echo"],
            json!([echo_descriptor(), boom_descriptor()]),
        ),
    )
    .await;

    let router = connect_multi_peer_router(MultiPeerRouterOptions {
        host_id: Some("test-router".to_string()),
    });
    router
        .register_peer(peer_a_adapter.clone())
        .expect("register a");
    router
        .register_peer(peer_b_adapter.clone())
        .expect("register b");

    let composed = router.get_host_capability_manifest().await;
    match composed {
        SpokeResult::Ok(composed) => {
            let capability_ids: Vec<&str> = composed
                .tools
                .iter()
                .map(|descriptor| descriptor.capability_id.as_str())
                .collect();
            assert_eq!(
                capability_ids,
                vec!["tools.echo.boom", "tools.echo.echo", "tools.math.add"]
            );
        }
        SpokeResult::Reject(reject) => panic!("composed view must succeed: {reject:?}"),
    }

    peer_a_adapter.close();
    peer_b_adapter.close();
    peer_a_responder.close();
    peer_b_responder.close();
}

// ── connect_responder (mirror of TS responder.test.ts) ────────────────────

/// Transport wrapper hook (wire-level injector fixtures).
type TransportWrap = Box<dyn Fn(Arc<dyn Transport>) -> Arc<dyn Transport> + Send + Sync>;

/// Dial options for the responder loopback pair.
#[derive(Default)]
struct ResponderDialOptions {
    client_manifest: Option<HostCapabilityManifest>,
    responder_manifest: Option<HostCapabilityManifest>,
    ports: Option<Arc<dyn RemoteServePorts + Send + Sync>>,
    responder_timeout_ms: Option<u64>,
    /// The dialer's own invoke budget, ms — the serve-wait cases keep it far
    /// longer than the responder's local serve budget so a served timeout is
    /// never confused with the caller's timer.
    client_timeout_ms: Option<u64>,
    responder_transport: Option<TransportWrap>,
    client_transport: Option<TransportWrap>,
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
    let client_end: Arc<dyn Transport> = match options.client_transport {
        Some(wrap) => wrap(Arc::new(pair.client.clone())),
        None => Arc::new(pair.client.clone()),
    };
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: client_end,
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: options
            .client_manifest
            .unwrap_or_else(|| tool_manifest("test-client")),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_responder.clone()],
        invoke_timeout_ms: options.client_timeout_ms,
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
            assert!(reject.message.contains("no ports face configured"));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn responder_concurrent_reverse_invokes_reach_the_wire_in_allocation_order() {
    // Mirror of the adapter's `concurrent_invokes_reach_the_wire_in_allocation_order`
    // (W-1 regression guard): the responder's reverse-invoke send tail must
    // serialize the wire so concurrent `invoke_tool` calls reach the dialer
    // in exactly the allocation order 0..N-1. The yield inside every send
    // widens the interleave window at the responder's `transport.send().await`
    // yield point on a multi-threaded runtime; without the
    // tail-before-allocation lock, a later-allocated request can win the
    // tail race, the dialer's strict inbound gate rejects it
    // (`invalid_sequence`), and the invoke fails.
    const CONCURRENT_INVOKES: i64 = 32;

    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        // Yield inside every responder send so concurrent reverse invokes
        // genuinely race for the wire position.
        responder_transport: Some(Box::new(|server_end| {
            Arc::new(YieldingSendTransport { inner: server_end })
        })),
        // Record every envelope the dialer receives; the handshake hellos /
        // snapshot are filtered out below by the op-bearing check.
        client_transport: Some(Box::new({
            let captured = Arc::clone(&captured);
            move |client_end| {
                Arc::new(RecordingTransport {
                    inner: client_end,
                    captured: Arc::clone(&captured),
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

    // Fire all reverse invokes concurrently; each allocates its outbound
    // sequence in call order. Serialized sends must put them on the wire
    // 0..N-1.
    let mut handles = Vec::new();
    for _ in 0..CONCURRENT_INVOKES {
        let responder = Arc::clone(&responder);
        handles.push(tokio::spawn(async move {
            responder
                .invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 }))
                .await
        }));
    }
    for handle in handles {
        let result = handle.await.expect("invoke task must not panic");
        assert!(
            result.is_ok(),
            "concurrent reverse invoke must succeed: {result:?}"
        );
    }

    // Wire witness: the request envelopes the dialer received carry exactly
    // the monotonic allocation order 0..N-1 (an out-of-order request would
    // have been denied invalid_sequence and that invoke would have failed
    // above).
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
async fn responder_answers_invalid_sequence_for_a_present_but_non_numeric_sequence() {
    // Parity with the TS gate (deny observability): a PRESENT but
    // non-numeric `sequence` on an inbound invoke is a malformed wire
    // request — the gate must answer the `invalid_sequence` deny branch
    // instead of silently ignoring it as `Stray` (a silent ignore makes the
    // sender wait out its timeout for no answer). The deny fires at the
    // sequence extraction (the first gate step), before the envelope-auth
    // verify.
    let (responder, pair, seed_client, _peer_id_client) = start_raw_responder().await;
    let session_id = raw_handshake(&pair.client, seed_client, &tool_manifest("test-client")).await;

    // A wire request whose `sequence` is present but is a STRING: signed
    // over the exact wire object (the deny fires before the signature is
    // ever verified).
    let malformed = json!({
        "session_id": session_id,
        "sequence": "5",
        "request_id": "non-numeric-seq",
        "op": "port.knowledge.get",
        "payload": { "entry_id": "kb_tw_mira" },
    });
    let signature = sign_envelope(&seed_client, &malformed);
    let mut wire = malformed.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    pair.client
        .send(&serde_json::to_vec(&Value::Object(wire)).expect("bytes"))
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
        "valid-after-non-numeric",
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

// ── Optional-port delegation (port.computable.* / port.fork.*) ───────────
// Parity mirror of the TS suite (T2): happy loopback round-trip per
// optional op — host + client manifests declare the optional families —
// and the capability-gate deny per op — default manifests advertise
// spoke-baseline only ⇒ the negotiated set lacks l2-computable / l5-fork
// ⇒ the host's dispatch gate denies wire `op_unsupported` ⇒ the D7 row
// maps it to CAPABILITY_PORT_MISSING with `details.wire_code` preserved
// (no local dialer pre-gate; the deny is responder-side, client-mapped).

/// Host/client manifest declaring the optional families (happy path).
fn optional_manifest(host_id: &str) -> HostCapabilityManifest {
    manifest(host_id, &["spoke-baseline", "l2-computable", "l5-fork"])
}

#[tokio::test]
async fn round_trips_project_over_the_loopback() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let optional = optional_manifest("test-host");
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_manifest: Some(optional.clone()),
            client_manifest: Some(optional),
            ..Default::default()
        },
    )
    .await;
    let request: ProjectRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "state": { "tide_level": 2.1, "cargo_tons": 40 },
    }))
    .expect("valid ProjectRequest");
    let result = client.project(request).await;
    match result {
        SpokeResult::Ok(ProjectResponse::Variant0 {
            session_id,
            entry_id,
            computable,
            ..
        }) => {
            assert_eq!(session_id.as_str(), "sess_tw_dawn_arrival");
            assert_eq!(entry_id.as_str(), "kb_tw_harbor");
            // The committed project fixture's computable view is echoed.
            assert_eq!(
                Value::Object(computable),
                json!({ "tide_level": 2.4, "cargo_tons": 38 })
            );
        }
        SpokeResult::Ok(ProjectResponse::Variant1 { error, .. }) => {
            panic!("project round-trip must succeed: {}", error.message)
        }
        SpokeResult::Reject(reject) => panic!("project round-trip must succeed: {reject:?}"),
    }
    // The host served the op (its dispatch_op would reject a wrong op /
    // malformed payload) — one dispatch, zero denials.
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 1);
    assert_eq!(stats.dispatch_denials, 0);
    client.close();
    host.close();
}

#[tokio::test]
async fn round_trips_compute_over_the_loopback() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let optional = optional_manifest("test-host");
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_manifest: Some(optional.clone()),
            client_manifest: Some(optional),
            ..Default::default()
        },
    )
    .await;
    let request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5, "cargo_tons": 37 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");
    let result = client.compute(request).await;
    match result {
        SpokeResult::Ok(ComputeResponse::Variant0 {
            computable,
            state,
            ..
        }) => {
            // The request computable is echoed and the settle response
            // carries the merged static state.
            assert_eq!(
                Value::Object(computable),
                json!({ "tide_level": 2.5, "cargo_tons": 37 })
            );
            assert_eq!(
                Value::Object(state),
                json!({ "tide_level": 2.5, "cargo_tons": 37 })
            );
        }
        SpokeResult::Ok(ComputeResponse::Variant1 { error, .. }) => {
            panic!("compute round-trip must succeed: {}", error.message)
        }
        SpokeResult::Reject(reject) => panic!("compute round-trip must succeed: {reject:?}"),
    }
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 1);
    assert_eq!(stats.dispatch_denials, 0);
    client.close();
    host.close();
}

#[tokio::test]
async fn round_trips_list_fork_timeline_events_over_the_loopback() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let optional = optional_manifest("test-host");
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_manifest: Some(optional.clone()),
            client_manifest: Some(optional),
            ..Default::default()
        },
    )
    .await;
    let scope: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    let result = client.list_fork_timeline_events(&scope).await;
    match result {
        SpokeResult::Ok(events) => {
            // The committed fixtures carry exactly one fork-branch event —
            // proves the payload carried the fork_id through the loopback.
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].timeline_event_id.as_str(),
                "evt_tw_harbor_storm_delay"
            );
            assert_eq!(
                events[0].fork_id.as_ref().map(|id| id.as_str()),
                Some("fork_tw_storm_branch")
            );
        }
        SpokeResult::Reject(reject) => panic!("fork round-trip must succeed: {reject:?}"),
    }
    // A fork id with no events still round-trips (empty array).
    let unknown: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_unknown",
    }))
    .expect("valid Scope");
    let empty = client.list_fork_timeline_events(&unknown).await;
    match empty {
        SpokeResult::Ok(events) => {
            assert!(events.is_empty(), "unknown fork id must round-trip to []")
        }
        SpokeResult::Reject(reject) => panic!("unknown fork round-trip must succeed: {reject:?}"),
    }
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 2);
    assert_eq!(stats.dispatch_denials, 0);
    client.close();
    host.close();
}

#[tokio::test]
async fn maps_the_capability_gate_deny_for_project_when_the_peer_manifest_lacks_l2_computable() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    // Default manifests advertise spoke-baseline only ⇒ the negotiated set
    // lacks l2-computable ⇒ the host's dispatch gate denies the op.
    let (client, host) = dial(host_adapter, DialOptions::default()).await;
    let request: ProjectRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "state": { "tide_level": 2.1, "cargo_tons": 40 },
    }))
    .expect("valid ProjectRequest");
    let result = client.project(request).await;
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
            assert!(reject.message.contains("not authorized"));
        }
        SpokeResult::Ok(_) => panic!("capability-gate deny must reject"),
    }
    let stats = host.stats();
    assert_eq!(stats.dispatch_denials, 1);
    assert_eq!(stats.invokes_dispatched, 0);
    client.close();
    host.close();
}

#[tokio::test]
async fn maps_the_capability_gate_deny_for_compute_when_the_peer_manifest_lacks_l2_computable() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(host_adapter, DialOptions::default()).await;
    let request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5, "cargo_tons": 37 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");
    let result = client.compute(request).await;
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
            assert!(reject.message.contains("not authorized"));
        }
        SpokeResult::Ok(_) => panic!("capability-gate deny must reject"),
    }
    let stats = host.stats();
    assert_eq!(stats.dispatch_denials, 1);
    assert_eq!(stats.invokes_dispatched, 0);
    client.close();
    host.close();
}

#[tokio::test]
async fn maps_the_capability_gate_deny_for_list_fork_timeline_events_when_the_peer_manifest_lacks_l5_fork(
) {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(host_adapter, DialOptions::default()).await;
    let scope: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    let result = client.list_fork_timeline_events(&scope).await;
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
            assert!(reject.message.contains("not authorized"));
        }
        SpokeResult::Ok(_) => panic!("capability-gate deny must reject"),
    }
    let stats = host.stats();
    assert_eq!(stats.dispatch_denials, 1);
    assert_eq!(stats.invokes_dispatched, 0);
    client.close();
    host.close();
}

// ── Optional-port serving (gate → probe → serve/deny) ─────────────────────
// Parity mirror of the TS responder suite (T4): the REAL connect_responder
// serves the optional families through the widened `RemoteServePorts` seam —
// happy loopback round-trip per optional op (full provider via the blanket
// impl), the capability-gate deny per op (default manifests negotiate
// spoke-baseline only ⇒ the gate denies wire `op_unsupported` ⇒ the D7 row
// maps it to CAPABILITY_PORT_MISSING with `details.wire_code` preserved),
// and the declared-but-not-provided deny (optional capabilities declared,
// but the injected ports are a baseline-only composite ⇒ the responder's
// probe answers the same dispatch-deny branch).

/// Tool-carrying manifest declaring the optional families on top of the
/// baseline + tool capabilities (mirror of TS `optionalManifest`).
fn optional_tool_manifest(host_id: &str) -> HostCapabilityManifest {
    let mut manifest = tool_manifest(host_id);
    manifest.capabilities.push("l2-computable".into());
    manifest.capabilities.push("l5-fork".into());
    manifest
}

/// Full optional-port provider: ToyWorldAdapter is a FullAdapter (the
/// blanket `RemoteServePorts` impl serves all families).
fn full_ports() -> Arc<dyn RemoteServePorts + Send + Sync> {
    Arc::new(ToyWorldAdapter::with_committed_fixtures())
}

/// Both peers declare the optional families (happy-path fixtures).
async fn optional_responder_dial() -> (
    Arc<ConnectResponder>,
    Arc<RemoteAdapter>,
    LoopbackTransportPair,
) {
    dial_with_responder(ResponderDialOptions {
        ports: Some(full_ports()),
        client_manifest: Some(optional_tool_manifest("test-client")),
        responder_manifest: Some(optional_tool_manifest("test-responder")),
        ..ResponderDialOptions::default()
    })
    .await
}

#[tokio::test]
async fn responder_round_trips_project_while_baseline_serving_stays_green() {
    let (responder, client, _pair) = optional_responder_dial().await;
    // Baseline smoke in the SAME session: the catalogue extension must not
    // disturb the baseline six (serving-order preservation).
    let mira = client.get_knowledge_entry("kb_tw_mira").await;
    match mira {
        SpokeResult::Ok(entry) => assert_eq!(entry.entry_id.as_str(), "kb_tw_mira"),
        SpokeResult::Reject(reject) => panic!("baseline smoke must succeed: {reject:?}"),
    }
    let request: ProjectRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "state": { "tide_level": 2.1, "cargo_tons": 40 },
    }))
    .expect("valid ProjectRequest");
    let result = client.project(request).await;
    match result {
        SpokeResult::Ok(ProjectResponse::Variant0 {
            session_id,
            entry_id,
            computable,
            ..
        }) => {
            assert_eq!(session_id.as_str(), "sess_tw_dawn_arrival");
            assert_eq!(entry_id.as_str(), "kb_tw_harbor");
            // The committed project fixture's computable view is echoed —
            // proves the payload carried the request through the loopback
            // and the responder delegated it.
            assert_eq!(
                Value::Object(computable),
                json!({ "tide_level": 2.4, "cargo_tons": 38 })
            );
        }
        SpokeResult::Ok(ProjectResponse::Variant1 { error, .. }) => {
            panic!("project round-trip must succeed: {}", error.message)
        }
        SpokeResult::Reject(reject) => panic!("project round-trip must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_round_trips_compute_over_the_loopback() {
    let (responder, client, _pair) = optional_responder_dial().await;
    let request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5, "cargo_tons": 37 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");
    let result = client.compute(request).await;
    match result {
        SpokeResult::Ok(ComputeResponse::Variant0 {
            computable,
            state,
            ..
        }) => {
            // The request computable is echoed and the settle response
            // carries the merged static state.
            assert_eq!(
                Value::Object(computable),
                json!({ "tide_level": 2.5, "cargo_tons": 37 })
            );
            assert_eq!(
                Value::Object(state),
                json!({ "tide_level": 2.5, "cargo_tons": 37 })
            );
        }
        SpokeResult::Ok(ComputeResponse::Variant1 { error, .. }) => {
            panic!("compute round-trip must succeed: {}", error.message)
        }
        SpokeResult::Reject(reject) => panic!("compute round-trip must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_round_trips_list_fork_timeline_events_over_the_loopback() {
    let (responder, client, _pair) = optional_responder_dial().await;
    let scope: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    let result = client.list_fork_timeline_events(&scope).await;
    match result {
        SpokeResult::Ok(events) => {
            // The committed fixtures carry exactly one fork-branch event —
            // proves the fork_id survived the loopback into the provider.
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].timeline_event_id.as_str(),
                "evt_tw_harbor_storm_delay"
            );
            assert_eq!(
                events[0].fork_id.as_ref().map(|id| id.as_str()),
                Some("fork_tw_storm_branch")
            );
        }
        SpokeResult::Reject(reject) => panic!("fork round-trip must succeed: {reject:?}"),
    }
    // A fork id with no events still round-trips (empty array).
    let unknown: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_unknown",
    }))
    .expect("valid Scope");
    let empty = client.list_fork_timeline_events(&unknown).await;
    match empty {
        SpokeResult::Ok(events) => {
            assert!(events.is_empty(), "unknown fork id must round-trip to []")
        }
        SpokeResult::Reject(reject) => panic!("unknown fork round-trip must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_denies_project_when_l2_computable_is_not_negotiated() {
    // Default manifests advertise spoke-baseline only ⇒ the negotiated set
    // lacks l2-computable ⇒ the responder's dispatch gate denies the op
    // even though the injected provider implements it (gate FIRST).
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        ports: Some(full_ports()),
        ..ResponderDialOptions::default()
    })
    .await;
    let request: ProjectRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "state": { "tide_level": 2.1, "cargo_tons": 40 },
    }))
    .expect("valid ProjectRequest");
    let result = client.project(request).await;
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
            assert!(reject.message.contains("not authorized"));
        }
        SpokeResult::Ok(_) => panic!("capability-gate deny must reject"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_denies_compute_when_l2_computable_is_not_negotiated() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        ports: Some(full_ports()),
        ..ResponderDialOptions::default()
    })
    .await;
    let request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5, "cargo_tons": 37 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");
    let result = client.compute(request).await;
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
            assert!(reject.message.contains("not authorized"));
        }
        SpokeResult::Ok(_) => panic!("capability-gate deny must reject"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_denies_list_fork_timeline_events_when_l5_fork_is_not_negotiated() {
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        ports: Some(full_ports()),
        ..ResponderDialOptions::default()
    })
    .await;
    let scope: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    let result = client.list_fork_timeline_events(&scope).await;
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
            assert!(reject.message.contains("not authorized"));
        }
        SpokeResult::Ok(_) => panic!("capability-gate deny must reject"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_denies_all_three_optional_ops_when_the_capability_is_declared_but_the_provider_is_baseline_only(
) {
    // Both peers declare the optional families (gate passes) but the
    // injected ports face is a baseline-only composite without the optional
    // faces — the responder's probe (gate → probe → deny) answers the same
    // dispatch-deny branch as absent ports.
    let baseline_only = RemoteServePortsComposite::new(
        Arc::new(ToyWorldAdapter::with_committed_fixtures()),
        None,
        None,
    );
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        ports: Some(Arc::new(baseline_only)),
        client_manifest: Some(optional_tool_manifest("test-client")),
        responder_manifest: Some(optional_tool_manifest("test-responder")),
        ..ResponderDialOptions::default()
    })
    .await;
    // project → probe deny (not the gate deny): the capability gate passed
    // and the responder's probe reports the missing face.
    let project_request: ProjectRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "state": { "tide_level": 2.1, "cargo_tons": 40 },
    }))
    .expect("valid ProjectRequest");
    let project_result = client.project(project_request).await;
    match project_result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("requires optional port method project"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
        SpokeResult::Ok(_) => panic!("probe deny must reject"),
    }
    // compute → same probe deny.
    let compute_request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5, "cargo_tons": 37 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");
    let compute_result = client.compute(compute_request).await;
    match compute_result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject.message.contains("requires optional port method compute"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
        SpokeResult::Ok(_) => panic!("probe deny must reject"),
    }
    // fork → same probe deny.
    let fork_scope: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    let fork_result = client.list_fork_timeline_events(&fork_scope).await;
    match fork_result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject
                .message
                .contains("requires optional port method list_fork_timeline_events"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
        SpokeResult::Ok(_) => panic!("probe deny must reject"),
    }
    // Baseline serving is untouched by the optional probe.
    let mira = client.get_knowledge_entry("kb_tw_mira").await;
    match mira {
        SpokeResult::Ok(entry) => assert_eq!(entry.entry_id.as_str(), "kb_tw_mira"),
        SpokeResult::Reject(reject) => panic!("baseline smoke must succeed: {reject:?}"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn responder_serves_computable_and_probe_denies_fork_on_a_mixed_composite() {
    // Mixed host: the composite carries the baseline + l2-computable faces
    // without the l5-fork face — computable ops serve through the present
    // face; the fork op probe-denies (the gate passes, so the missing face
    // is host misconfiguration).
    let adapter = Arc::new(ToyWorldAdapter::with_committed_fixtures());
    let mixed = RemoteServePortsComposite::new(adapter.clone(), Some(adapter), None);
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        ports: Some(Arc::new(mixed)),
        client_manifest: Some(optional_tool_manifest("test-client")),
        responder_manifest: Some(optional_tool_manifest("test-responder")),
        ..ResponderDialOptions::default()
    })
    .await;
    // Computable op succeeds through the present face.
    let project_request: ProjectRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "state": { "tide_level": 2.1, "cargo_tons": 40 },
    }))
    .expect("valid ProjectRequest");
    let project_result = client.project(project_request).await;
    match project_result {
        SpokeResult::Ok(ProjectResponse::Variant0 { session_id, .. }) => {
            assert_eq!(session_id.as_str(), "sess_tw_dawn_arrival");
        }
        SpokeResult::Ok(_) => panic!("project must return Variant0"),
        SpokeResult::Reject(reject) => panic!("computable serve must succeed: {reject:?}"),
    }
    // Fork op probe-denies: the gate passed but the face is absent.
    let fork_scope: Scope = serde_json::from_value(json!({
        "scope_id": "pkt_tw_scope",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    let fork_result = client.list_fork_timeline_events(&fork_scope).await;
    match fork_result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(reject
                .message
                .contains("requires optional port method list_fork_timeline_events"));
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("op_unsupported")
            );
        }
        SpokeResult::Ok(_) => panic!("fork probe deny must reject"),
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

// ── KE remote: `extract` service face (F1/F3) + ownership gate (F2) ───────

/// Loader-only canary. It exists only inside the host's loaded input value
/// and must never appear in any wire envelope in either direction.
const LOADER_CANARY: &str = "spoke-ke-remote-loader-canary-7f3a91";

/// How the test host's extraction behaves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtractBehavior {
    /// One provisional candidate per source anchor.
    Batch,
    /// A successful zero-result run.
    EmptyBatch,
    /// An application reject raised inside the host orchestration.
    Reject,
    /// The `ExtractResponse` error branch returned as a service success.
    ErrorBranch,
}

#[derive(Default)]
struct ExtractHostState {
    loads: AtomicUsize,
    runs: AtomicUsize,
    canary_consumed: AtomicBool,
}

/// Host-side `extract` service double: a real host-local loader, a real
/// in-process extractor, and the operations crate's `orchestrate_extract`
/// doing the boundary / provisional / assembly work. The loaded value is a
/// host-local in-process value — never a wire parameter.
#[derive(Clone)]
struct CanonicalExtractHost {
    state: Arc<ExtractHostState>,
    behavior: ExtractBehavior,
}

impl CanonicalExtractHost {
    fn new(behavior: ExtractBehavior) -> Self {
        Self {
            state: Arc::new(ExtractHostState::default()),
            behavior,
        }
    }

    /// `(loader calls, service calls)`.
    fn calls(&self) -> (usize, usize) {
        (
            self.state.loads.load(Ordering::SeqCst),
            self.state.runs.load(Ordering::SeqCst),
        )
    }

    fn canary_consumed(&self) -> bool {
        self.state.canary_consumed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ExtractionPort for CanonicalExtractHost {
    async fn load_extraction_input(&self, _request: &ExtractRequest) -> SpokeResult<Value> {
        self.state.loads.fetch_add(1, Ordering::SeqCst);
        spoke_ok(json!({
            "canary": LOADER_CANARY,
            "manuscript": "host-local source content that must never reach the wire",
        }))
    }
}

#[async_trait]
impl RemoteExtractService for CanonicalExtractHost {
    async fn extract(&self, request: ExtractRequest) -> SpokeResult<ExtractResponse> {
        self.state.runs.fetch_add(1, Ordering::SeqCst);
        if self.behavior == ExtractBehavior::ErrorBranch {
            return spoke_ok(extract_error_branch());
        }
        let host = self.clone();
        let behavior = self.behavior;
        let state = Arc::clone(&self.state);
        // `orchestrate_extract` takes `&dyn ExtractionPort` (a plain trait
        // object), so its returned future is `!Send`; the connect service
        // face is `Send`. The host drives the real orchestration on a
        // blocking worker — the same place a host's loader callback belongs.
        let joined = tokio::task::spawn_blocking(move || {
            futures::executor::block_on(orchestrate_extract(
                &host,
                request,
                move |input: ExtractRunInput| async move {
                    state.canary_consumed.store(
                        input.input.get("canary").and_then(Value::as_str) == Some(LOADER_CANARY),
                        Ordering::SeqCst,
                    );
                    if behavior == ExtractBehavior::Reject {
                        return spoke_reject(
                            SpokeRejectCode::CandidateNotProvisional,
                            "the extractor declined every candidate",
                            None,
                        );
                    }
                    let candidates = if behavior == ExtractBehavior::EmptyBatch {
                        Vec::new()
                    } else {
                        input
                            .request
                            .sources
                            .iter()
                            .enumerate()
                            .map(|(index, _)| {
                                provisional_candidate(input.request.run_id.as_str(), index)
                            })
                            .collect()
                    };
                    spoke_ok(ExtractionResult {
                        candidates,
                        method: Some("canonical".to_owned()),
                        coverage_hint: None,
                    })
                },
            ))
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => spoke_reject(
                SpokeRejectCode::InternalError,
                format!("extract host task failed: {error}"),
                None,
            ),
        }
    }
}

/// One provisional candidate — the operation invariant the orchestrator
/// enforces on every returned entry.
fn provisional_candidate(run_id: &str, index: usize) -> KnowledgeEntry {
    serde_json::from_value(json!({
        "schema_version": 1,
        "entry_id": format!("ke-remote-{run_id}-{index}"),
        "entry_type": "note",
        "canonical_name": format!("Extracted note {index}"),
        "status": "provisional",
        "body": { "summary": format!("provisional candidate {index}") },
        "extensions": {},
    }))
    .expect("valid provisional KnowledgeEntry")
}

/// The `ExtractResponse` error branch an injected service can return as a
/// SUCCESS value — the shape the responder must normalize to the reject path.
fn extract_error_branch() -> ExtractResponse {
    serde_json::from_value(json!({
        "error": {
            "code": "KNOWLEDGE_ENTRY_NOT_FOUND",
            "message": "the extractor's referenced source is gone",
            "extensions": {},
        },
    }))
    .expect("valid ExtractResponse error branch")
}

/// KE remote hello manifests: the tool fixture plus the capabilities the
/// scenario negotiates (both hellos must declare a capability for it to be
/// negotiated).
fn ke_remote_manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
    let mut manifest = tool_manifest(host_id);
    manifest
        .capabilities
        .extend(capabilities.iter().map(|capability| (*capability).to_owned()));
    manifest
}

/// Baseline provider that records every `Scope` it is asked to list, so the
/// tests can assert what actually reached the provider.
struct RecordingScopePorts {
    inner: ToyWorldAdapter,
    scopes: Mutex<Vec<Value>>,
}

impl RecordingScopePorts {
    fn new() -> Self {
        Self {
            inner: ToyWorldAdapter::with_committed_fixtures(),
            scopes: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, scope: &Scope) {
        self.scopes
            .lock()
            .expect("scopes lock")
            .push(serde_json::to_value(scope).expect("scope serializes"));
    }

    fn scopes(&self) -> Vec<Value> {
        self.scopes.lock().expect("scopes lock").clone()
    }
}

#[async_trait]
impl KnowledgeEntryPort for RecordingScopePorts {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.inner.get_knowledge_entry(entry_id).await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.inner
            .put_knowledge_entry(entry, expected_base_revision)
            .await
    }
}

#[async_trait]
impl RelationPort for RecordingScopePorts {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.inner.get_relation(relation_id).await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.inner
            .put_relation(relation, expected_base_revision)
            .await
    }
}

#[async_trait]
impl ScopeQueryPort for RecordingScopePorts {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.record(scope);
        self.inner.list_knowledge_entries(scope).await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.record(scope);
        self.inner.list_timeline_events(scope).await
    }
}

#[async_trait]
impl FindingPort for RecordingScopePorts {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.inner.put_findings(findings).await
    }
}

#[async_trait]
impl RuleQueryPort for RecordingScopePorts {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.inner.list_rules(rule_refs).await
    }
}

#[async_trait]
impl HostManifestPort for RecordingScopePorts {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        self.inner.get_host_capability_manifest().await
    }

    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.inner.list_peer_host_capability_manifests().await
    }
}

/// Both-directions wire capture: the whole session view of an on-path
/// observer. Backs the F1 "no loader value on the wire" assertion.
struct CapturingTransport {
    inner: Arc<dyn Transport>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    received: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait]
impl Transport for CapturingTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        self.sent.lock().expect("sent lock").push(envelope.to_vec());
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let bytes = self.inner.recv().await?;
        self.received
            .lock()
            .expect("received lock")
            .push(bytes.clone());
        Ok(bytes)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

type CapturedWire = Arc<Mutex<Vec<Vec<u8>>>>;

fn captured_contains(envelopes: &CapturedWire, needle: &str) -> bool {
    envelopes
        .lock()
        .expect("capture lock")
        .iter()
        .any(|bytes| String::from_utf8_lossy(bytes).contains(needle))
}

/// The payload of the single captured outbound envelope carrying `op`.
fn captured_request_payload(envelopes: &CapturedWire, op: &str) -> Value {
    for bytes in envelopes.lock().expect("capture lock").iter() {
        let Ok(doc) = serde_json::from_slice::<Value>(bytes) else {
            continue;
        };
        if doc.get("op").and_then(Value::as_str) == Some(op) {
            return doc.get("payload").cloned().unwrap_or(Value::Null);
        }
    }
    panic!("no captured outbound request for op {op}");
}

fn captured_request_ops(envelopes: &CapturedWire) -> Vec<String> {
    envelopes
        .lock()
        .expect("capture lock")
        .iter()
        .filter_map(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .filter_map(|doc| {
            doc.get("op")
                .and_then(Value::as_str)
                .map(|op| op.to_owned())
        })
        .collect()
}

/// KE remote scenario dial options.
#[derive(Default)]
struct KeRemoteDial {
    client_capabilities: Vec<&'static str>,
    responder_capabilities: Vec<&'static str>,
    extract: Option<CanonicalExtractHost>,
    baseline: Option<Arc<RecordingScopePorts>>,
    /// Full ports-face override for the serve-wait scenarios, which compose
    /// their own provider + optional faces.
    ports: Option<Arc<dyn RemoteServePorts + Send + Sync>>,
    /// Responder local serve-wait budget, ms (default: the responder default).
    serve_budget_ms: Option<u64>,
    /// Dialer invoke budget, ms (the serve-wait scenarios keep it far longer
    /// than `serve_budget_ms`).
    client_timeout_ms: Option<u64>,
    /// Responder-side outbound capture. The close witnesses observe the
    /// responder's own send ATTEMPT: the loopback rejects a send once the
    /// connection is closed, so a delivered frame is no longer observable.
    responder_sent: Option<CapturedWire>,
}

/// Loopback pair for the KE remote scenarios: the productized responder
/// serving `extract` through the injected service and the Scope ops through
/// the (optionally recording) baseline provider, with both wire directions
/// captured on the client end.
async fn ke_remote_dial(
    options: KeRemoteDial,
) -> (
    Arc<ConnectResponder>,
    Arc<RemoteAdapter>,
    CapturedWire,
    CapturedWire,
) {
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = match options.ports {
        Some(ports) => ports,
        None => {
            let baseline: Arc<dyn BaselinePorts + Send + Sync> = match options.baseline {
                Some(recording) => recording,
                None => Arc::new(ToyWorldAdapter::with_committed_fixtures()),
            };
            let mut ports = RemoteServePortsComposite::new(baseline, None, None);
            if let Some(extract) = options.extract {
                ports = ports.with_extract(Arc::new(extract));
            }
            Arc::new(ports)
        }
    };
    let sent: CapturedWire = Arc::new(Mutex::new(Vec::new()));
    let received: CapturedWire = Arc::new(Mutex::new(Vec::new()));
    let sent_wrap = Arc::clone(&sent);
    let received_wrap = Arc::clone(&received);
    let (responder, client, _pair) = dial_with_responder(ResponderDialOptions {
        client_manifest: Some(ke_remote_manifest(
            "test-client",
            &options.client_capabilities,
        )),
        responder_manifest: Some(ke_remote_manifest(
            "test-responder",
            &options.responder_capabilities,
        )),
        ports: Some(ports),
        responder_timeout_ms: options.serve_budget_ms,
        client_timeout_ms: options.client_timeout_ms,
        responder_transport: options.responder_sent.map(|sent| {
            let received: CapturedWire = Arc::new(Mutex::new(Vec::new()));
            let wrap: TransportWrap = Box::new(move |inner| {
                Arc::new(CapturingTransport {
                    inner,
                    sent: Arc::clone(&sent),
                    received: Arc::clone(&received),
                })
            });
            wrap
        }),
        client_transport: Some(Box::new(move |inner| {
            Arc::new(CapturingTransport {
                inner,
                sent: Arc::clone(&sent_wrap),
                received: Arc::clone(&received_wrap),
            })
        })),
        ..ResponderDialOptions::default()
    })
    .await;
    (responder, client, sent, received)
}

/// A responder without a dialing client, so a test can put a signed request
/// on the wire that the typed adapter cannot express (a malformed declared
/// Scope).
async fn start_raw_ke_responder(
    ports: Arc<dyn RemoteServePorts + Send + Sync>,
    capabilities: &[&str],
) -> (Arc<ConnectResponder>, LoopbackTransportPair, [u8; 32]) {
    start_raw_ke_responder_with_budget(ports, capabilities, None).await
}

/// [`start_raw_ke_responder`] with an explicit local serve-wait budget, so a
/// serve-bound witness can drive a raw request the typed adapter cannot
/// express against a zero/short budget.
async fn start_raw_ke_responder_with_budget(
    ports: Arc<dyn RemoteServePorts + Send + Sync>,
    capabilities: &[&str],
    serve_budget_ms: Option<u64>,
) -> (Arc<ConnectResponder>, LoopbackTransportPair, [u8; 32]) {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity {
            seed: seed_host(),
        },
        manifest: ke_remote_manifest("test-responder", capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: serve_budget_ms,
    })
    .await;
    (responder, pair, seed_client())
}

/// A Scope carrying a reader viewpoint plus opaque extension values the
/// provider must receive unchanged.
fn viewpoint_scope() -> Scope {
    serde_json::from_value(json!({
        "scope_id": "toy-scope-001",
        "viewpoint": "kb_tw_mira",
        "entry_types": ["note"],
        "extensions": { "product": { "viewpoint": "decoy", "owner": "someone" } },
    }))
    .expect("valid Scope")
}

#[tokio::test]
async fn ke_remote_extract_round_trips_a_provisional_batch_without_the_loader_value() {
    let host = CanonicalExtractHost::new(ExtractBehavior::Batch);
    let (responder, client, sent, received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-extraction"],
        responder_capabilities: vec!["ke-extraction"],
        extract: Some(host.clone()),
        ..KeRemoteDial::default()
    })
    .await;

    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-ke-remote-1",
        "sources": [
            { "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} },
            { "schema_version": 1, "source_id": "manuscript/ch2", "extensions": {} },
        ],
    }))
    .expect("valid ExtractRequest");

    let result = client.extract(request).await;
    match result {
        SpokeResult::Ok(ExtractResponse::Variant0 { candidates, run, .. }) => {
            // Correlation: the batch id is echoed verbatim.
            assert_eq!(run.run_id.as_str(), "run-ke-remote-1");
            assert_eq!(candidates.len(), 2);
            // Operation invariant: every returned candidate is provisional.
            for candidate in &candidates {
                assert_eq!(candidate.status.as_str(), "provisional");
            }
        }
        SpokeResult::Ok(ExtractResponse::Variant1 { error, .. }) => {
            panic!("extract round-trip must succeed: {}", error.message)
        }
        SpokeResult::Reject(reject) => panic!("extract round-trip must succeed: {reject:?}"),
    }

    // The host-local load and the in-process extractor both ran exactly once,
    // and the extractor consumed the loaded value.
    assert_eq!(host.calls(), (1, 1));
    assert!(
        host.canary_consumed(),
        "the extractor must consume the loaded value on the serving host"
    );

    // The invoke payload IS the `ExtractRequest` — no wrapper object.
    let payload = captured_request_payload(&sent, "extract");
    assert_eq!(payload["run_id"], json!("run-ke-remote-1"));
    assert_eq!(
        payload["sources"].as_array().expect("sources").len(),
        2,
        "the request carries the source references, not their content"
    );
    assert!(payload.get("request").is_none());
    assert!(payload.get("arguments").is_none());

    // No content on the wire: the loader-only canary appears in no captured
    // envelope in either direction.
    assert!(
        !captured_contains(&sent, LOADER_CANARY),
        "the extract request must not carry the loader value"
    );
    assert!(
        !captured_contains(&received, LOADER_CANARY),
        "the extract response must not carry the loader value"
    );

    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_extract_serves_a_successful_empty_candidate_batch() {
    let host = CanonicalExtractHost::new(ExtractBehavior::EmptyBatch);
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-extraction"],
        responder_capabilities: vec!["ke-extraction"],
        extract: Some(host.clone()),
        ..KeRemoteDial::default()
    })
    .await;

    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-ke-remote-empty",
        "sources": [{ "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} }],
    }))
    .expect("valid ExtractRequest");

    match client.extract(request).await {
        SpokeResult::Ok(ExtractResponse::Variant0 { candidates, run, .. }) => {
            assert!(candidates.is_empty(), "zero results is a success, not a deny");
            assert_eq!(run.run_id.as_str(), "run-ke-remote-empty");
        }
        SpokeResult::Ok(ExtractResponse::Variant1 { error, .. }) => {
            panic!("empty batch must succeed: {}", error.message)
        }
        SpokeResult::Reject(reject) => panic!("empty batch must succeed: {reject:?}"),
    }
    assert_eq!(host.calls(), (1, 1));

    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_extract_denies_when_ke_extraction_is_not_negotiated() {
    let host = CanonicalExtractHost::new(ExtractBehavior::Batch);
    // Both hellos omit `ke-extraction`, so the negotiated intersection does
    // not contain it: the responder's static gate must deny before the
    // service is probed or called.
    let (responder, client, sent, _received) = ke_remote_dial(KeRemoteDial {
        extract: Some(host.clone()),
        ..KeRemoteDial::default()
    })
    .await;

    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-ke-remote-denied",
        "sources": [{ "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} }],
    }))
    .expect("valid ExtractRequest");

    match client.extract(request).await {
        SpokeResult::Ok(_) => panic!("an unnegotiated extract must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
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
    // The dialer never pre-gates: the request reached the wire, and the host
    // neither probed the service nor loaded anything.
    assert!(
        captured_request_ops(&sent).iter().any(|op| op == "extract"),
        "the dialer must invoke the peer rather than pre-gate locally"
    );
    assert_eq!(host.calls(), (0, 0));

    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_extract_probe_denies_when_the_capability_is_declared_but_the_service_is_absent() {
    // `ke-extraction` is negotiated but the composed ports face carries no
    // extract service: the existing not-serving dispatch-deny branch, which
    // is distinct from the negotiated-capability deny above.
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-extraction"],
        responder_capabilities: vec!["ke-extraction"],
        ..KeRemoteDial::default()
    })
    .await;

    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-ke-remote-absent",
        "sources": [{ "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} }],
    }))
    .expect("valid ExtractRequest");

    match client.extract(request).await {
        SpokeResult::Ok(_) => panic!("an absent extract service must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert!(
                reject.message.contains("no extract service configured"),
                "probe deny must name the missing service: {}",
                reject.message
            );
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
async fn ke_remote_extract_maps_a_service_application_reject() {
    // A served request whose extraction legitimately fails keeps its own
    // reject code — it is NOT recast as an unavailable capability.
    let host = CanonicalExtractHost::new(ExtractBehavior::Reject);
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-extraction"],
        responder_capabilities: vec!["ke-extraction"],
        extract: Some(host.clone()),
        ..KeRemoteDial::default()
    })
    .await;

    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-ke-remote-reject",
        "sources": [{ "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} }],
    }))
    .expect("valid ExtractRequest");

    match client.extract(request).await {
        SpokeResult::Ok(_) => panic!("a rejected extraction must not answer success"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CandidateNotProvisional);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code")),
                None,
                "an application reject is not a dispatch deny"
            );
        }
    }
    assert_eq!(host.calls(), (1, 1), "the host ran once before rejecting");

    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_extract_normalizes_the_response_error_branch_to_the_reject_path() {
    // A service that returns the `ExtractResponse` error branch as a SUCCESS
    // value must be normalized through the existing envelope map — never
    // relayed as a nested success carrying an error.
    let host = CanonicalExtractHost::new(ExtractBehavior::ErrorBranch);
    let (responder, client, _sent, received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-extraction"],
        responder_capabilities: vec!["ke-extraction"],
        extract: Some(host.clone()),
        ..KeRemoteDial::default()
    })
    .await;

    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-ke-remote-error-branch",
        "sources": [{ "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} }],
    }))
    .expect("valid ExtractRequest");

    match client.extract(request).await {
        SpokeResult::Ok(response) => panic!("the error branch must not answer success: {response:?}"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::KnowledgeEntryNotFound);
            assert!(reject.message.contains("referenced source is gone"));
        }
    }
    // The wire answered the error branch (not a success payload).
    let response_error = received
        .lock()
        .expect("capture lock")
        .iter()
        .filter_map(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .find_map(|doc| doc.get("error").cloned());
    assert_eq!(
        response_error
            .as_ref()
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("KNOWLEDGE_ENTRY_NOT_FOUND")
    );
    assert_eq!(host.calls(), (0, 1), "the error branch never loads a source");

    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_scope_viewpoint_with_ownership_negotiated_reaches_the_provider_unchanged() {
    let recording = Arc::new(RecordingScopePorts::new());
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-ownership"],
        responder_capabilities: vec!["ke-ownership"],
        baseline: Some(Arc::clone(&recording)),
        ..KeRemoteDial::default()
    })
    .await;

    let result = client.list_knowledge_entries(&viewpoint_scope()).await;
    if let SpokeResult::Reject(reject) = &result {
        assert_ne!(
            reject.code,
            SpokeRejectCode::CapabilityPortMissing,
            "a negotiated viewpoint request must not be capability-denied: {reject:?}"
        );
    }
    // The provider received the declared Scope unchanged: the viewpoint and
    // the opaque extension values are not stripped to make the request
    // succeed.
    let received = recording.scopes();
    assert_eq!(received.len(), 1, "the provider must be reached exactly once");
    assert_eq!(received[0]["viewpoint"], json!("kb_tw_mira"));
    assert_eq!(received[0]["entry_types"], json!(["note"]));
    assert_eq!(
        received[0]["extensions"]["product"]["viewpoint"],
        json!("decoy"),
        "an extension value that merely resembles the gate carrier is preserved"
    );
    assert_eq!(received[0]["extensions"]["product"]["owner"], json!("someone"));

    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_scope_viewpoint_without_a_negotiated_ownership_capability_is_refused() {
    // Either hello omitting `ke-ownership` leaves it out of the negotiated
    // intersection — the request must fail loudly instead of silently
    // returning an unfiltered success.
    for (client_capabilities, responder_capabilities) in [
        (vec!["ke-ownership"], Vec::new()),
        (Vec::new(), vec!["ke-ownership"]),
    ] {
        let recording = Arc::new(RecordingScopePorts::new());
        let (responder, client, sent, _received) = ke_remote_dial(KeRemoteDial {
            client_capabilities,
            responder_capabilities,
            baseline: Some(Arc::clone(&recording)),
            ..KeRemoteDial::default()
        })
        .await;

        match client.list_knowledge_entries(&viewpoint_scope()).await {
            SpokeResult::Ok(_) => {
                panic!("a viewpoint request without a negotiated `ke-ownership` must refuse")
            }
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
                assert!(
                    reject.message.contains("ke-ownership"),
                    "the deny must name the missing capability: {}",
                    reject.message
                );
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
        // Refused before any host work, and never pre-gated by the dialer.
        assert!(
            recording.scopes().is_empty(),
            "the provider must not be reached on a refused viewpoint request"
        );
        assert!(
            captured_request_ops(&sent)
                .iter()
                .any(|op| op == "port.scope.list_knowledge_entries"),
            "the dialer must invoke the peer rather than pre-gate locally"
        );

        client.close();
        responder.close();
    }
}

#[tokio::test]
async fn ke_remote_scope_viewpoint_absent_keeps_baseline_serving() {
    let recording = Arc::new(RecordingScopePorts::new());
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        baseline: Some(Arc::clone(&recording)),
        ..KeRemoteDial::default()
    })
    .await;

    let scope: Scope =
        serde_json::from_value(json!({ "scope_id": "toy-scope-001" })).expect("valid Scope");
    match client.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(entries) => assert!(
            entries
                .iter()
                .any(|entry| entry.entry_id.as_str() == "kb_tw_mira")
        ),
        SpokeResult::Reject(reject) => panic!("baseline serving must stay green: {reject:?}"),
    }
    assert_eq!(recording.scopes().len(), 1);

    client.close();
    responder.close();
}

#[tokio::test]
async fn ke_remote_scope_malformed_declaration_rejects_invalid_input_before_the_provider() {
    let recording = Arc::new(RecordingScopePorts::new());
    let baseline: Arc<dyn BaselinePorts + Send + Sync> = recording.clone();
    let ports: Arc<dyn RemoteServePorts + Send + Sync> =
        Arc::new(RemoteServePortsComposite::new(baseline, None, None));
    let (responder, pair, seed) = start_raw_ke_responder(ports, &["ke-ownership"]).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &["ke-ownership"]),
    )
    .await;

    // Every malformed declared Scope: a present `viewpoint` that is not a
    // non-empty string (note that a present JSON null would otherwise decode
    // as an absent viewpoint), a missing Scope, and a non-object Scope.
    let malformed_payloads = [
        json!({ "scope": { "scope_id": "s1", "viewpoint": "" } }),
        json!({ "scope": { "scope_id": "s1", "viewpoint": null } }),
        json!({ "scope": { "scope_id": "s1", "viewpoint": 3 } }),
        json!({ "scope": { "scope_id": "s1", "viewpoint": [] } }),
        json!({ "scope": { "scope_id": "s1", "viewpoint": {} } }),
        json!({}),
        json!({ "scope": "not-an-object" }),
    ];
    for (sequence, payload) in malformed_payloads.iter().enumerate() {
        let request = sign_invoke_request(
            seed,
            &session_id,
            sequence as i64,
            &format!("ke-malformed-{sequence}"),
            "port.scope.list_knowledge_entries",
            payload.clone(),
        );
        pair.client
            .send(&serde_json::to_vec(&request).expect("bytes"))
            .await
            .expect("send");
        let response: Value = serde_json::from_slice(&pair.client.recv().await.expect("recv"))
            .expect("response decode");
        assert_eq!(
            response["error"]["code"], "INVALID_INPUT",
            "payload {payload} must be an input failure, got {response}"
        );
    }
    assert!(
        recording.scopes().is_empty(),
        "a malformed declared Scope must never reach the provider"
    );

    // Control: a valid non-empty viewpoint on the next sequence serves
    // normally — the validation is not a blanket refusal.
    let valid = sign_invoke_request(
        seed,
        &session_id,
        malformed_payloads.len() as i64,
        "ke-valid-viewpoint",
        "port.scope.list_knowledge_entries",
        json!({ "scope": { "scope_id": "toy-scope-001", "viewpoint": "kb_tw_mira" } }),
    );
    pair.client
        .send(&serde_json::to_vec(&valid).expect("bytes"))
        .await
        .expect("send");
    let response: Value = serde_json::from_slice(&pair.client.recv().await.expect("recv"))
        .expect("response decode");
    assert!(
        response.get("error").is_none(),
        "the valid viewpoint must serve, got {response}"
    );
    let scopes = recording.scopes();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0]["viewpoint"], json!("kb_tw_mira"));

    responder.close();
}

/// Send one signed raw invoke and return the decoded response envelope (the
/// typed adapter cannot express a malformed declared Scope).
async fn raw_invoke(
    client: &LoopbackTransport,
    seed: [u8; 32],
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let request = sign_invoke_request(seed, session_id, sequence, request_id, op, payload);
    client
        .send(&serde_json::to_vec(&request).expect("request bytes"))
        .await
        .expect("request send");
    serde_json::from_slice(&client.recv().await.expect("response recv")).expect("response decode")
}

#[tokio::test]
async fn ke_remote_scope_malformed_declaration_is_an_input_failure_without_the_ownership_capability()
{
    // The requirement predicate is false for a malformed viewpoint, so a
    // malformed declared Scope answers an input failure even when
    // `ke-ownership` was never negotiated — never a capability deny.
    let recording = Arc::new(RecordingScopePorts::new());
    let baseline: Arc<dyn BaselinePorts + Send + Sync> = recording.clone();
    let ports: Arc<dyn RemoteServePorts + Send + Sync> =
        Arc::new(RemoteServePortsComposite::new(baseline, None, None));
    let (responder, pair, seed) = start_raw_ke_responder(ports, &[]).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &[]),
    )
    .await;

    let malformed = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        0,
        "ke-malformed-unnegotiated",
        "port.scope.list_knowledge_entries",
        json!({ "scope": { "scope_id": "s1", "viewpoint": null } }),
    )
    .await;
    assert_eq!(
        malformed["error"]["code"], "INVALID_INPUT",
        "a malformed viewpoint is an input failure, got {malformed}"
    );

    // Structurally malformed declared Scopes (closed key set / field types)
    // that still carry a request-qualifying viewpoint: the predicate is true,
    // so an input failure winning over the capability deny pins the declared
    // Scope decode to the responder gate.
    let structurally_malformed = [
        (
            "ke-malformed-unknown-key-unnegotiated",
            json!({ "scope": { "scope_id": "s1", "viewpoint": "kb_tw_mira", "bogus": 1 } }),
        ),
        (
            "ke-malformed-scope-id-type-unnegotiated",
            json!({ "scope": { "scope_id": 3, "viewpoint": "kb_tw_mira" } }),
        ),
    ];
    for (offset, (request_id, payload)) in structurally_malformed.iter().enumerate() {
        let response = raw_invoke(
            &pair.client,
            seed,
            &session_id,
            offset as i64 + 1,
            request_id,
            "port.scope.list_knowledge_entries",
            payload.clone(),
        )
        .await;
        assert_eq!(
            response["error"]["code"], "INVALID_INPUT",
            "payload {payload} must be an input failure, got {response}"
        );
    }

    // Control on the same session: a valid viewpoint takes the capability
    // deny — the malformed rows are validation, not a blanket op refusal.
    let valid = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        structurally_malformed.len() as i64 + 1,
        "ke-valid-unnegotiated",
        "port.scope.list_knowledge_entries",
        json!({ "scope": { "scope_id": "s1", "viewpoint": "kb_tw_mira" } }),
    )
    .await;
    assert_eq!(
        valid["error"]["code"], "op_unsupported",
        "a valid viewpoint without the negotiated capability must deny, got {valid}"
    );

    assert!(
        recording.scopes().is_empty(),
        "no request may reach the provider"
    );
    responder.close();
}

#[tokio::test]
async fn ke_remote_scope_validation_precedes_the_optional_face_probe() {
    // Fork op with `l5-fork` + `ke-ownership` negotiated and NO fork face on
    // the provider: a malformed declared Scope still answers INVALID_INPUT
    // (validation runs before the probe), while a valid viewpoint takes the
    // probe deny that names the missing face.
    let recording = Arc::new(RecordingScopePorts::new());
    let baseline: Arc<dyn BaselinePorts + Send + Sync> = recording.clone();
    let ports: Arc<dyn RemoteServePorts + Send + Sync> =
        Arc::new(RemoteServePortsComposite::new(baseline, None, None));
    let capabilities = ["l5-fork", "ke-ownership"];
    let (responder, pair, seed) = start_raw_ke_responder(ports, &capabilities).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &capabilities),
    )
    .await;

    let malformed = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        0,
        "ke-fork-malformed",
        "port.fork.list_timeline_events",
        json!({ "scope": { "scope_id": "s1", "viewpoint": null } }),
    )
    .await;
    assert_eq!(
        malformed["error"]["code"], "INVALID_INPUT",
        "declared-Scope validation must precede the optional-face probe, got {malformed}"
    );

    let valid = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        1,
        "ke-fork-valid",
        "port.fork.list_timeline_events",
        json!({ "scope": { "scope_id": "s1", "viewpoint": "kb_tw_mira" } }),
    )
    .await;
    assert_eq!(
        valid["error"]["code"], "op_unsupported",
        "a valid viewpoint with no fork face must take the probe deny, got {valid}"
    );
    assert!(
        valid["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("list_fork_timeline_events"),
        "the probe deny must name the missing face: {valid}"
    );

    assert!(
        recording.scopes().is_empty(),
        "neither request may reach the provider"
    );
    responder.close();
}

#[tokio::test]
async fn ke_remote_extract_decodes_the_request_after_the_provider_probe() {
    // Frozen F3 serving order: gate → provider probe → decode/validate →
    // call the service once. With the service present a malformed payload is
    // an input failure and nothing is loaded; with the service absent the
    // declared-but-absent provider row (probe deny) answers first, whatever
    // the payload says.
    let host = CanonicalExtractHost::new(ExtractBehavior::Batch);
    let serving: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(
        RemoteServePortsComposite::new(
            Arc::new(ToyWorldAdapter::with_committed_fixtures()),
            None,
            None,
        )
        .with_extract(Arc::new(host.clone())),
    );
    let (responder, pair, seed) = start_raw_ke_responder(serving, &["ke-extraction"]).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &["ke-extraction"]),
    )
    .await;

    let malformed = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        0,
        "ke-extract-malformed",
        "extract",
        json!({}),
    )
    .await;
    assert_eq!(
        malformed["error"]["code"], "INVALID_INPUT",
        "a malformed ExtractRequest is an input failure, got {malformed}"
    );
    assert_eq!(
        host.calls(),
        (0, 0),
        "a malformed request must never reach the service or its loader"
    );
    responder.close();

    let unserved: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(
        RemoteServePortsComposite::new(
            Arc::new(ToyWorldAdapter::with_committed_fixtures()),
            None,
            None,
        ),
    );
    let (responder, pair, seed) = start_raw_ke_responder(unserved, &["ke-extraction"]).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &["ke-extraction"]),
    )
    .await;
    let absent = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        0,
        "ke-extract-absent-malformed",
        "extract",
        json!({}),
    )
    .await;
    assert_eq!(
        absent["error"]["code"], "op_unsupported",
        "the probe deny precedes request decoding, got {absent}"
    );
    assert!(
        absent["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no extract service configured"),
        "the probe deny must name the missing service: {absent}"
    );
    responder.close();
}

// ── serve-side timeout boundary (B1/B2) ──────────────────────────────────

/// Provider fixture for the serve-wait witnesses: the blocking methods record
/// entry, park until the test releases them and hold a [`ParkProbe`], so a
/// witness can prove the local budget bounded the wait and dropped the parked
/// provider future instead of letting it run to completion. Every other
/// method delegates to the committed toy-world adapter.
struct ParkingServePorts {
    inner: ToyWorldAdapter,
    calls: Arc<Mutex<Vec<String>>>,
    release: Arc<Notify>,
    parked: Arc<AtomicUsize>,
    released: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl ParkingServePorts {
    fn new() -> Self {
        Self {
            inner: ToyWorldAdapter::with_committed_fixtures(),
            calls: Arc::new(Mutex::new(Vec::new())),
            release: Arc::new(Notify::new()),
            parked: Arc::new(AtomicUsize::new(0)),
            released: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The provider methods entered, in order (one entry = one provider call).
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls lock").clone()
    }

    /// Release every parked provider call.
    fn release(&self) {
        self.release.notify_waiters();
    }

    fn parked(&self) -> usize {
        self.parked.load(Ordering::SeqCst)
    }

    /// Parked calls that ran past the release gate (a bounded-away call stays
    /// at 0).
    fn released(&self) -> usize {
        self.released.load(Ordering::SeqCst)
    }

    /// Parked provider futures the serve wait dropped.
    fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }

    /// Record entry, park until `release`, then record the release. The
    /// [`ParkProbe`] marks the future's drop: a serve-budget expiry drops the
    /// parked future, so `released` stays 0 while `dropped` rises.
    async fn park(&self, method: &str) {
        self.calls.lock().expect("calls lock").push(method.to_owned());
        self.parked.fetch_add(1, Ordering::SeqCst);
        let _probe = ParkProbe(Arc::clone(&self.dropped));
        self.release.notified().await;
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

/// Counts the drops of a parked provider call (the B2 bounded-wait model: the
/// timed-out future is dropped, never left running).
struct ParkProbe(Arc<AtomicUsize>);

impl Drop for ParkProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Bounded poll until the parking provider has recorded `count` entries, so a
/// close witness can close the session while the provider call is in flight.
async fn until_parked(ports: &ParkingServePorts, count: usize) {
    for _ in 0..200 {
        if ports.parked() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the parking provider never recorded {count} entries");
}

/// The responder ports face for the serve-wait witnesses: the baseline,
/// l2-computable and `extract` faces are all served by the parking provider.
fn serve_wait_ports(ports: &Arc<ParkingServePorts>) -> Arc<dyn RemoteServePorts + Send + Sync> {
    Arc::new(
        RemoteServePortsComposite::new(ports.clone(), Some(ports.clone()), None)
            .with_extract(ports.clone()),
    )
}

#[async_trait]
impl KnowledgeEntryPort for ParkingServePorts {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.inner.get_knowledge_entry(entry_id).await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.inner
            .put_knowledge_entry(entry, expected_base_revision)
            .await
    }
}

#[async_trait]
impl RelationPort for ParkingServePorts {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.inner.get_relation(relation_id).await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.inner
            .put_relation(relation, expected_base_revision)
            .await
    }
}

#[async_trait]
impl ScopeQueryPort for ParkingServePorts {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.park("list_knowledge_entries").await;
        self.inner.list_knowledge_entries(scope).await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.inner.list_timeline_events(scope).await
    }
}

#[async_trait]
impl FindingPort for ParkingServePorts {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.inner.put_findings(findings).await
    }
}

#[async_trait]
impl RuleQueryPort for ParkingServePorts {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.inner.list_rules(rule_refs).await
    }
}

#[async_trait]
impl HostManifestPort for ParkingServePorts {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        self.inner.get_host_capability_manifest().await
    }

    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.inner.list_peer_host_capability_manifests().await
    }
}

#[async_trait]
impl ComputablePort for ParkingServePorts {
    async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse> {
        self.inner.project(request).await
    }

    async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse> {
        self.park("compute").await;
        self.inner.compute(request).await
    }
}

#[async_trait]
impl RemoteExtractService for ParkingServePorts {
    async fn extract(&self, _request: ExtractRequest) -> SpokeResult<ExtractResponse> {
        self.park("extract").await;
        // A completion the serve wait already bounded away must never be
        // served — a late second response would surface this marker.
        spoke_reject(
            SpokeRejectCode::InternalError,
            "a completion that outlived the serve budget must never be served",
            None,
        )
    }
}

/// The `details.kind` of a reject, when present.
fn reject_detail_kind(reject: &SpokeReject) -> Option<&str> {
    reject
        .details
        .as_ref()
        .and_then(|details| details.get("kind"))
        .and_then(Value::as_str)
}

/// The `details.wire_code` of a reject, when present.
fn reject_wire_code(reject: &SpokeReject) -> Option<&str> {
    reject
        .details
        .as_ref()
        .and_then(|details| details.get("wire_code"))
        .and_then(Value::as_str)
}

/// The inbound envelopes carrying `field` — the serve-response accounting
/// witness (exactly one signed timeout response, never a late second one).
fn captured_frames_with(envelopes: &CapturedWire, field: &str) -> Vec<Value> {
    envelopes
        .lock()
        .expect("capture lock")
        .iter()
        .filter_map(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .filter(|doc| doc.get(field).is_some())
        .collect()
}

/// Assert the served B2 timeout projection on a captured wire response.
fn assert_served_timeout_frame(frame: &Value, budget_ms: u64) {
    assert_eq!(
        frame["error"]["code"], "INTERNAL_ERROR",
        "the served expiry is the existing application-reject map: {frame}"
    );
    assert_eq!(
        frame["error"]["details"]["kind"], "timeout",
        "the served expiry carries the timeout kind: {frame}"
    );
    assert!(
        frame["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains(&format!("serve wait exceeded the local budget of {budget_ms}ms")),
        "the served expiry must name the responder's own budget (never the dialer's): {frame}"
    );
}

#[tokio::test]
async fn serve_timeout_bounds_a_parked_scope_port_and_answers_one_signed_timeout() {
    // The provider parks; the responder's own 25ms local budget expires while
    // the dialer's budget is far longer (2000ms), so the observed reject can
    // only come from the serve-side bound.
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, client, sent, received) = ke_remote_dial(KeRemoteDial {
        ports: Some(serve_wait_ports(&ports)),
        serve_budget_ms: Some(25),
        client_timeout_ms: Some(2000),
        ..KeRemoteDial::default()
    })
    .await;
    let scope: Scope =
        serde_json::from_value(json!({ "scope_id": "serve-timeout-scope" })).expect("valid Scope");

    match client.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(_) => panic!("a parked provider must hit the local serve budget"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject_detail_kind(&reject), Some("timeout"));
            assert!(
                reject
                    .message
                    .contains("serve wait exceeded the local budget of 25ms"),
                "the reject is the responder's own budget, not the caller's timer: {}",
                reject.message
            );
        }
    }

    // The dialer never pre-gates, and the provider ran exactly once.
    assert!(
        captured_request_ops(&sent)
            .iter()
            .any(|op| op == "port.scope.list_knowledge_entries"),
        "the invoke must reach the wire"
    );
    assert_eq!(ports.calls(), vec!["list_knowledge_entries"]);
    assert_eq!(ports.parked(), 1);
    assert_eq!(
        ports.released(),
        0,
        "the parked provider call must not run past the serve wait"
    );
    assert_eq!(
        ports.dropped(),
        1,
        "the serve wait drops the timed-out provider future"
    );

    // Exactly one signed timeout response reached the wire.
    let errors = captured_frames_with(&received, "error");
    assert_eq!(errors.len(), 1, "exactly one error response: {errors:?}");
    assert_served_timeout_frame(&errors[0], 25);

    // A late completion is discarded: releasing the parked provider after
    // expiry produces no second response, and the dropped future never
    // observes the release.
    let payloads_before = captured_frames_with(&received, "payload").len();
    ports.release();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(captured_frames_with(&received, "error").len(), 1);
    assert_eq!(
        captured_frames_with(&received, "payload").len(),
        payloads_before
    );
    assert_eq!(ports.released(), 0);

    // The session stayed Established and the consumed inbound sequence is
    // preserved: the next invoke on the same session is served normally.
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    assert_eq!(client.state(), RemoteAdapterState::Established);
    match client.get_knowledge_entry("kb_tw_mira").await {
        SpokeResult::Ok(entry) => assert_eq!(entry.entry_id.as_str(), "kb_tw_mira"),
        SpokeResult::Reject(reject) => panic!("same-session serving must stay usable: {reject:?}"),
    }
    client.close();
    responder.close();
}

#[tokio::test]
async fn serve_timeout_bounds_a_parked_optional_family_provider() {
    // Optional-family dispatch shares the same serve bound (one boundary
    // covers the method family).
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, client, _sent, received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["l2-computable"],
        responder_capabilities: vec!["l2-computable"],
        ports: Some(serve_wait_ports(&ports)),
        serve_budget_ms: Some(25),
        client_timeout_ms: Some(2000),
        ..KeRemoteDial::default()
    })
    .await;
    let request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");

    match client.compute(request).await {
        SpokeResult::Ok(_) => panic!("a parked optional-family provider must hit the serve budget"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject_detail_kind(&reject), Some("timeout"));
            assert!(
                reject
                    .message
                    .contains("serve wait exceeded the local budget of 25ms"),
                "the reject is the responder's own budget: {}",
                reject.message
            );
        }
    }
    assert_eq!(ports.calls(), vec!["compute"]);
    assert_eq!(ports.dropped(), 1);

    let errors = captured_frames_with(&received, "error");
    assert_eq!(errors.len(), 1, "exactly one error response: {errors:?}");
    assert_served_timeout_frame(&errors[0], 25);
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn serve_timeout_bounds_a_parked_extract_service_and_discards_the_late_completion() {
    // The whole extraction service (loader + extractor) is one bounded wait.
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, client, _sent, received) = ke_remote_dial(KeRemoteDial {
        client_capabilities: vec!["ke-extraction"],
        responder_capabilities: vec!["ke-extraction"],
        ports: Some(serve_wait_ports(&ports)),
        serve_budget_ms: Some(25),
        client_timeout_ms: Some(2000),
        ..KeRemoteDial::default()
    })
    .await;
    let request: ExtractRequest = serde_json::from_value(json!({
        "run_id": "run-serve-timeout",
        "sources": [{ "schema_version": 1, "source_id": "manuscript/ch1", "extensions": {} }],
    }))
    .expect("valid ExtractRequest");

    match client.extract(request).await {
        SpokeResult::Ok(_) => panic!("a parked extract service must hit the serve budget"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject_detail_kind(&reject), Some("timeout"));
            assert!(
                reject
                    .message
                    .contains("serve wait exceeded the local budget of 25ms"),
                "the reject is the responder's own budget: {}",
                reject.message
            );
        }
    }
    assert_eq!(ports.calls(), vec!["extract"]);
    assert_eq!(ports.dropped(), 1);
    let errors = captured_frames_with(&received, "error");
    assert_eq!(errors.len(), 1, "exactly one error response: {errors:?}");
    assert_served_timeout_frame(&errors[0], 25);

    // The late extract completion is discarded — never a second response.
    let payloads_before = captured_frames_with(&received, "payload").len();
    ports.release();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(captured_frames_with(&received, "error").len(), 1);
    assert_eq!(
        captured_frames_with(&received, "payload").len(),
        payloads_before
    );
    assert_eq!(ports.released(), 0);
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn serve_timeout_zero_budget_answers_before_any_provider_call() {
    // Zero means immediate timeout: the signed response is produced without
    // starting provider work, and the session is reusable for the ops that
    // do not need the bounded face.
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, client, _sent, received) = ke_remote_dial(KeRemoteDial {
        ports: Some(serve_wait_ports(&ports)),
        serve_budget_ms: Some(0),
        client_timeout_ms: Some(2000),
        ..KeRemoteDial::default()
    })
    .await;
    let scope: Scope =
        serde_json::from_value(json!({ "scope_id": "serve-timeout-zero" })).expect("valid Scope");

    match client.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(_) => panic!("a zero budget must answer the served timeout"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject_detail_kind(&reject), Some("timeout"));
            assert!(
                reject
                    .message
                    .contains("serve wait exceeded the local budget of 0ms"),
                "zero answers the responder's own budget: {}",
                reject.message
            );
        }
    }
    assert!(ports.calls().is_empty(), "zero provider calls for a zero budget");
    assert_eq!(ports.dropped(), 0);
    let errors = captured_frames_with(&received, "error");
    assert_eq!(errors.len(), 1, "exactly one error response: {errors:?}");
    assert_served_timeout_frame(&errors[0], 0);

    // The bounded wait never closes the session (no forced close, no counter
    // rollback): the state stays Established and a further invoke on the same
    // session is answered on the wire again.
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    match client.get_knowledge_entry("kb_tw_mira").await {
        SpokeResult::Ok(_) => panic!("a zero budget bounds every provider call"),
        SpokeResult::Reject(reject) => assert_eq!(reject_detail_kind(&reject), Some("timeout")),
    }
    assert!(ports.calls().is_empty());
    client.close();
    responder.close();
}

#[tokio::test]
async fn serve_timeout_zero_budget_keeps_gate_and_deny_precedence() {
    // Gate, payload-dependent ownership deny and optional-face probe all run
    // BEFORE the serve bound: at zero budget they still answer their own
    // codes and the provider stays untouched.
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        // `l5-fork` negotiated, `l2-computable` not: one dial covers the
        // capability deny, the probe deny and the ownership deny.
        client_capabilities: vec!["l5-fork"],
        responder_capabilities: vec!["l5-fork"],
        ports: Some(serve_wait_ports(&ports)),
        serve_budget_ms: Some(0),
        client_timeout_ms: Some(2000),
        ..KeRemoteDial::default()
    })
    .await;

    let request: ComputeRequest = serde_json::from_value(json!({
        "session_id": "sess_tw_dawn_arrival",
        "entry_id": "kb_tw_harbor",
        "computable": { "tide_level": 2.5 },
        "settle": true,
    }))
    .expect("valid ComputeRequest");
    match client.compute(request).await {
        SpokeResult::Ok(_) => panic!("unnegotiated l2-computable must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(reject_wire_code(&reject), Some("op_unsupported"));
            assert_eq!(
                reject_detail_kind(&reject),
                None,
                "a gate deny is not the serve timeout: {reject:?}"
            );
        }
    }

    let fork_scope: Scope = serde_json::from_value(json!({
        "scope_id": "serve-timeout-probe",
        "fork_id": "fork_tw_storm_branch",
    }))
    .expect("valid Scope");
    match client.list_fork_timeline_events(&fork_scope).await {
        SpokeResult::Ok(_) => panic!("an absent optional face must probe-deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(reject_wire_code(&reject), Some("op_unsupported"));
        }
    }

    match client.list_knowledge_entries(&viewpoint_scope()).await {
        SpokeResult::Ok(_) => panic!("a viewpoint without ke-ownership must deny"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(reject_wire_code(&reject), Some("op_unsupported"));
        }
    }

    assert!(
        ports.calls().is_empty(),
        "no denied request may reach the provider: {:?}",
        ports.calls()
    );
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}

#[tokio::test]
async fn serve_timeout_zero_budget_keeps_scope_validation_precedence() {
    // The payload-dependent Scope validation runs before the serve bound: a
    // malformed declared Scope stays an input failure at zero budget, while a
    // valid Scope on the same session answers the signed timeout.
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, pair, seed) =
        start_raw_ke_responder_with_budget(serve_wait_ports(&ports), &[], Some(0)).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &[]),
    )
    .await;

    let malformed = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        0,
        "serve-timeout-malformed-scope",
        "port.scope.list_knowledge_entries",
        json!({ "scope": { "scope_id": "s1", "viewpoint": "" } }),
    )
    .await;
    assert_eq!(
        malformed["error"]["code"], "INVALID_INPUT",
        "validation must win over the serve bound, got {malformed}"
    );

    let valid = tokio::time::timeout(
        Duration::from_secs(2),
        raw_invoke(
            &pair.client,
            seed,
            &session_id,
            1,
            "serve-timeout-zero-budget",
            "port.scope.list_knowledge_entries",
            json!({ "scope": { "scope_id": "s1" } }),
        ),
    )
    .await
    .expect("the served timeout must arrive without waiting out a client timer");
    assert_eq!(valid["error"]["code"], "INTERNAL_ERROR", "got {valid}");
    assert_eq!(valid["error"]["details"]["kind"], "timeout");
    assert_served_timeout_frame(&valid, 0);
    assert!(
        ports.calls().is_empty(),
        "a zero budget must not reach the provider"
    );
    assert_eq!(ports.dropped(), 0);
    responder.close();
}

#[tokio::test]
async fn serve_timeout_zero_budget_keeps_port_payload_validation_precedence() {
    // The D4 catalogue decode runs before the serve bound: a malformed
    // baseline port payload keeps its existing `INVALID_INPUT` reject at zero
    // budget, while a valid payload on the same session answers the signed
    // timeout — both with zero provider calls.
    let ports = Arc::new(ParkingServePorts::new());
    let (responder, pair, seed) =
        start_raw_ke_responder_with_budget(serve_wait_ports(&ports), &[], Some(0)).await;
    let session_id = raw_handshake(
        &pair.client,
        seed,
        &ke_remote_manifest("test-client", &[]),
    )
    .await;

    let malformed = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        0,
        "serve-timeout-malformed-put",
        "port.knowledge.put",
        json!({ "entry": { "entry_id": 7 } }),
    )
    .await;
    assert_eq!(
        malformed["error"]["code"], "INVALID_INPUT",
        "the catalogue decode must win over the serve bound, got {malformed}"
    );
    assert!(
        malformed["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid port.knowledge.put payload"),
        "the existing decode reject is preserved: {malformed}"
    );

    let non_string = raw_invoke(
        &pair.client,
        seed,
        &session_id,
        1,
        "serve-timeout-malformed-get",
        "port.knowledge.get",
        json!({ "entry_id": 7 }),
    )
    .await;
    assert_eq!(
        non_string["error"]["code"], "INVALID_INPUT",
        "a non-string entry_id is an input failure, not the serve timeout: {non_string}"
    );

    let valid = tokio::time::timeout(
        Duration::from_secs(2),
        raw_invoke(
            &pair.client,
            seed,
            &session_id,
            2,
            "serve-timeout-zero-budget-port",
            "port.knowledge.get",
            json!({ "entry_id": "kb_tw_mira" }),
        ),
    )
    .await
    .expect("the served timeout must arrive without waiting out a client timer");
    assert_eq!(valid["error"]["code"], "INTERNAL_ERROR", "got {valid}");
    assert_eq!(valid["error"]["details"]["kind"], "timeout");
    assert_served_timeout_frame(&valid, 0);
    assert!(
        ports.calls().is_empty(),
        "a zero budget must not reach the provider"
    );
    assert_eq!(ports.dropped(), 0);
    responder.close();
}

#[tokio::test]
async fn serve_timeout_close_aborts_a_parked_serve_dispatch_and_answers_nothing() {
    // A close releases the session: the in-flight serve dispatch — and the
    // local serve timer it armed — must not outlive it, so a provider that
    // settles afterwards answers neither a late success nor a late error.
    let ports = Arc::new(ParkingServePorts::new());
    let responder_sent: CapturedWire = Arc::new(Mutex::new(Vec::new()));
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        ports: Some(serve_wait_ports(&ports)),
        serve_budget_ms: Some(25),
        client_timeout_ms: Some(2000),
        responder_sent: Some(Arc::clone(&responder_sent)),
        ..KeRemoteDial::default()
    })
    .await;
    let scope: Scope =
        serde_json::from_value(json!({ "scope_id": "serve-timeout-close" })).expect("valid Scope");

    // The close fails the dialer's waiter; the witness asserts the
    // responder's own send attempts instead.
    let dialer = {
        let client = Arc::clone(&client);
        tokio::spawn(async move { client.list_knowledge_entries(&scope).await })
    };
    until_parked(&ports, 1).await;
    assert_eq!(ports.calls(), vec!["list_knowledge_entries"]);

    responder.close();
    let attempted_at_close = responder_sent.lock().expect("sent lock").len();
    // Past the local budget: a serve timer that survived the close would have
    // answered the signed timeout.
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        responder_sent.lock().expect("sent lock").len(),
        attempted_at_close,
        "no serve response may follow the close"
    );

    // A provider settling after the close is discarded too.
    ports.release();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        responder_sent.lock().expect("sent lock").len(),
        attempted_at_close
    );
    assert_eq!(
        ports.released(),
        0,
        "the aborted serve dispatch never resumes past its release gate"
    );
    assert_eq!(
        ports.dropped(),
        1,
        "the close drops the parked provider future"
    );
    assert_eq!(responder.state(), RemoteAdapterState::Closed);
    match dialer.await.expect("dialer task") {
        SpokeResult::Ok(_) => panic!("the close must fail the dialer's waiter"),
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            assert_eq!(reject_detail_kind(&reject), Some("session_closed"));
        }
    }
}

#[tokio::test]
async fn serve_timeout_does_not_affect_a_normal_serve_call() {
    // A provider that completes inside the budget is served unchanged.
    let (responder, client, _sent, _received) = ke_remote_dial(KeRemoteDial {
        serve_budget_ms: Some(25),
        client_timeout_ms: Some(2000),
        ..KeRemoteDial::default()
    })
    .await;
    let scope: Scope =
        serde_json::from_value(json!({ "scope_id": "toy-scope-001" })).expect("valid Scope");

    match client.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(entries) => assert!(
            entries
                .iter()
                .any(|entry| entry.entry_id.as_str() == "kb_tw_mira"),
            "a completing provider must be served unchanged"
        ),
        SpokeResult::Reject(reject) => {
            panic!("a completing provider must not be bounded away: {reject:?}")
        }
    }
    assert_eq!(responder.state(), RemoteAdapterState::Established);
    client.close();
    responder.close();
}
