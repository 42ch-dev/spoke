//! Loopback test — the memory family end-to-end over Connect (`memory.*`).
//!
//! The Rust **client** is the typed `MemoryPort` adapter on `RemoteAdapter`
//! (`crates/spoke-connect/src/remote/remote_adapter.rs`); the **host** is an
//! in-memory `MemoryPort` provider served through `MemoryPortService`. Both
//! directions therefore ride the typed P2 boundary — no JSON-face shortcut.
//!
//! The provider enforces its own duties, independently of the engine: grant
//! coverage (`spoke_operations::grant_covers`), idempotent replay with
//! `IDEMPOTENCY_CONFLICT` on a changed payload digest, space addressing on
//! review, and space / selection / principal filtering before the records
//! reach engine scoring.
//!
//! The dispatch-deny rows and the unknown-op fallback are asserted on the raw
//! wire (the typed client cannot express an unknown op).
//!
//! Gated on the `remote-adapter` feature; `cargo test -p spoke-connect`
//! (default features) does not build it.

#![cfg(feature = "remote-adapter")]

#[path = "common/loopback_oracle.rs"]
mod loopback_oracle;
use loopback_oracle::{
    connect_manifest, pubkey_client, seed_client, seed_host, sign_envelope,
};

#[path = "support/memory_host.rs"]
mod memory_host;
use memory_host::{
    not_served, reject, wire, MemoryHost, Store, DEFAULT_SPACE as SPACE, DEFAULT_SUBJECT as SUBJECT,
};

#[path = "support/sqlite_memory.rs"]
mod sqlite_memory;
use sqlite_memory::SqliteMemoryProvider;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use spoke_connect::core::{derive_peer_id_from_ed25519_pubkey, sign_hello_ed25519};
use spoke_connect::remote::{
    connect_remote_adapter, connect_responder, loopback_transport_pair, ConnectResponder,
    ConnectResponderOptions, LoopbackTransport, LoopbackTransportPair, MemoryAdmission,
    MemoryPortService, RemoteAdapter, RemoteAdapterOptions, RemoteIdentity, RemoteMemoryService,
    RemoteServePorts, RemoteServePortsComposite, Transport,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_fixture_memory_engine::ENGINE_POLICY_VERSION;
use spoke_operations::{
    cognition_record_matches_filter, jcs_sha256_hex, validate_cognition_record, BaselinePorts,
    FindingPort, HostManifestPort, KnowledgeEntryPort, MemoryPort, RelationPort, RuleQueryPort,
    ScopeQueryPort, SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::{ConnectHello, ConnectSession};
use spoke_schemas::{
    HostCapabilityManifest, MemoryCandidatesDismissRequest, MemoryCandidatesDismissResponse,
    MemoryCandidatesGetRequest, MemoryCandidatesGetResponse, MemoryCandidatesListRequest,
    MemoryCandidatesListResponse, MemoryCandidatesReviewRequest, MemoryCandidatesReviewResponse,
    MemoryCaptureRequest, MemoryCaptureResponse, MemoryDescribeRequest, MemoryDescribeResponse,
    MemoryExportsCreateRequest, MemoryExportsCreateResponse, MemoryExportsGetRequest,
    MemoryExportsGetResponse, MemoryGrantsGrantRequest, MemoryGrantsGrantResponse,
    MemoryGrantsListRequest, MemoryGrantsListResponse, MemoryGrantsRevokeRequest,
    MemoryGrantsRevokeResponse, MemoryImportsActivateRequest, MemoryImportsActivateResponse,
    MemoryImportsDiscardRequest, MemoryImportsDiscardResponse, MemoryImportsStageRequest,
    MemoryImportsStageResponse, MemoryProfileGetRequest, MemoryProfileGetResponse,
    MemoryProfileUpdateRequest, MemoryProfileUpdateResponse, MemoryRecallRequest,
    MemoryRecallResponse, MemoryReceiptsGetRequest, MemoryReceiptsGetResponse,
    MemoryRecordsCreateRequest, MemoryRecordsCreateResponse, MemoryRecordsDeleteRequest,
    MemoryRecordsDeleteResponse, MemoryRecordsGetRequest, MemoryRecordsGetResponse,
    MemoryRecordsListRequest, MemoryRecordsListResponse, MemoryRecordsPromoteRequest,
    MemoryRecordsPromoteResponse, MemoryRecordsUpdateRequest, MemoryRecordsUpdateResponse,
    MemoryReflectionsGenerateRequest, MemoryReflectionsGenerateResponse,
    MemoryReflectionsListRequest, MemoryReflectionsListResponse, MemoryReflectionsStateRequest,
    MemoryReflectionsStateResponse, MemorySpacesFreezeRequest, MemorySpacesFreezeResponse,
    MemorySpacesListRequest, MemorySpacesListResponse, MemorySpacesUnfreezeRequest,
    MemorySpacesUnfreezeResponse, MemorySubjectsEnsureRequest, MemorySubjectsEnsureResponse,
    MemorySubjectsFenceRequest, MemorySubjectsFenceResponse, MemorySubjectsGetRequest,
    MemorySubjectsGetResponse, Principal,
    Finding, KnowledgeEntry, Relation, Rule, Scope, TimelineEvent,
};

const OTHER_SPACE: &str = "spc_other";
const CONTEXT: &str = "bnd_mira_harbor_run_7";
const OTHER_CONTEXT: &str = "bnd_bo_market_run_2";

/// Memory capabilities negotiated by both ends of the loopback session.
const MEMORY_CAPABILITIES: [&str; 4] = [
    "spoke-baseline",
    "spoke-memory",
    "spoke-memory-reflection",
    "spoke-memory-transfer",
];

fn expect_ok<T>(result: SpokeResult<T>) -> T {
    match result {
        SpokeResult::Ok(value) => value,
        SpokeResult::Reject(reject) => panic!("expected success, got {reject:?}"),
    }
}

fn expect_reject<T: std::fmt::Debug>(result: SpokeResult<T>) -> SpokeReject {
    match result {
        SpokeResult::Ok(value) => panic!("expected a rejection, got {value:?}"),
        SpokeResult::Reject(reject) => reject,
    }
}

fn reject_wire_code(reject: &SpokeReject) -> Option<String> {
    reject
        .details
        .as_ref()
        .and_then(|details| details.get("wire_code"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ── Session plumbing ──────────────────────────────────────────────────────

fn principal_for(peer_id: &str) -> SpokeResult<Principal> {
    match serde_json::from_value(json!({
        "principal_id": format!("prn_{peer_id}"),
        "extensions": {},
    })) {
        Ok(principal) => SpokeResult::Ok(principal),
        Err(error) => reject(
            SpokeRejectCode::InternalError,
            format!("principal decode failed: {error}"),
        ),
    }
}

fn admission() -> MemoryAdmission {
    Arc::new(|peer_id: &str| principal_for(peer_id))
}

/// The serving face: the typed `MemoryPort` adapter (the P2 boundary).
fn memory_face(host: &Arc<MemoryHost>) -> Arc<dyn spoke_connect::remote::RemoteMemoryService> {
    let host: Arc<MemoryHost> = Arc::clone(host);
    let port: Arc<dyn MemoryPort + Send + Sync> = host;
    Arc::new(MemoryPortService::new(port, admission()))
}

fn memory_manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": capabilities,
        "namespaces": [],
        "extensions": {},
    }))
    .expect("valid manifest")
}

/// Start a responder whose ports face carries the memory service (when given)
/// and whose manifest advertises `capabilities`.
async fn start_host(
    memory: Option<Arc<dyn spoke_connect::remote::RemoteMemoryService>>,
    capabilities: &[&str],
) -> (Arc<ConnectResponder>, LoopbackTransportPair) {
    start_host_with_budget(memory, capabilities, None).await
}

async fn start_host_with_budget(
    memory: Option<Arc<dyn spoke_connect::remote::RemoteMemoryService>>,
    capabilities: &[&str],
    serve_budget_ms: Option<u64>,
) -> (Arc<ConnectResponder>, LoopbackTransportPair) {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let baseline: Arc<dyn BaselinePorts + Send + Sync> =
        Arc::new(ToyWorldAdapter::with_committed_fixtures());
    let mut ports = RemoteServePortsComposite::new(baseline, None, None);
    if let Some(memory) = memory {
        ports = ports.with_memory(memory);
    }
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(ports);
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity { seed: seed_host() },
        manifest: memory_manifest("memory-loopback-host", capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: serve_budget_ms,
    })
    .await;
    (responder, pair)
}

/// Dial the typed client over the loopback pair.
async fn dial(pair: LoopbackTransportPair, manifest: &HostCapabilityManifest) -> Arc<RemoteAdapter> {
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let _: Arc<dyn Transport> = Arc::new(pair.client.clone());
    connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest.clone(),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial")
}

fn pubkey_host() -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(&seed_host())
        .verifying_key()
        .to_bytes()
}

/// The local admission context of the dialing peer. Over the wire the host
/// derives the principal from the authenticated peer, so this value is never
/// transmitted (MEM-R003).
fn client_principal() -> Principal {
    expect_ok(principal_for(&derive_peer_id_from_ed25519_pubkey(
        &pubkey_client(),
    )))
}

// ── Typed request fixtures ────────────────────────────────────────────────

fn capture_request(idempotency_key: &str, context: &str, text: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": { "text": text },
        "capture_scope": { "visibility": "context", "context_id": context },
        "source_context_ids": [context],
        "kind_hint": "fact",
        "idempotency_key": idempotency_key,
    })
}

fn shared_capture_request(idempotency_key: &str, text: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": { "text": text },
        "capture_scope": { "visibility": "shared" },
        "kind_hint": "fact",
        "idempotency_key": idempotency_key,
    })
}

fn review_request(space_id: &str, candidate_id: &str, key: &str, text: &str) -> Value {
    json!({
        "space_id": space_id,
        "candidate_id": candidate_id,
        "expected_revision": 1,
        "decision": {
            "action": "fragment",
            "fragments": [{ "kind": "fact", "content": { "text": text } }],
        },
        "idempotency_key": key,
    })
}

fn fragmented_review_request(
    space_id: &str,
    candidate_id: &str,
    key: &str,
    fragments: &[(&str, &str)],
) -> Value {
    let fragments: Vec<Value> = fragments
        .iter()
        .map(|(kind, text)| json!({ "kind": kind, "content": { "text": text } }))
        .collect();
    json!({
        "space_id": space_id,
        "candidate_id": candidate_id,
        "expected_revision": 1,
        "decision": { "action": "fragment", "fragments": fragments },
        "idempotency_key": key,
    })
}

fn recall_request(context: &str, query: &str, max_items: u64) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [context] },
        "query": query,
        "budget": { "max_items": max_items },
    })
}

fn shared_recall_request(max_items: u64) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "shared_only" },
        "budget": { "max_items": max_items },
    })
}

fn typed_capture(request: Value) -> MemoryCaptureRequest {
    serde_json::from_value(request).expect("valid memory.capture request")
}

fn typed_review(request: Value) -> MemoryCandidatesReviewRequest {
    serde_json::from_value(request).expect("valid memory.candidates.review request")
}

fn typed_recall(request: Value) -> MemoryRecallRequest {
    serde_json::from_value(request).expect("valid memory.recall request")
}

// ── Raw-wire client (unknown-op fallback only) ────────────────────────────

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

/// Process-local handshake nonce source: one counter per test binary, so
/// every hello nonce is distinct without reading a wall clock.
fn next_nonce() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

async fn handshake(client: &LoopbackTransport, manifest: &HostCapabilityManifest) -> String {
    // A hello nonce is single-use by contract: derive it from a process-local
    // counter so every handshake in this binary is distinct and no
    // hard-coded cryptographic value sits in the test source.
    let nonce = format!("memory-loopback-nonce-{:x}", next_nonce());
    let hello = sign_hello_ed25519(&seed_client(), &nonce, &connect_manifest(manifest), None)
        .expect("initiator hello sign");
    client
        .send(&serde_json::to_vec(&hello).expect("hello bytes"))
        .await
        .expect("hello send");
    let _responder_hello: ConnectHello =
        serde_json::from_slice(&client.recv().await.expect("responder hello recv"))
            .expect("responder hello decode");
    let snapshot = client.recv().await.expect("snapshot recv");
    let session: ConnectSession = serde_json::from_slice(&snapshot).expect("session decode");
    session.session_id.to_string()
}

async fn raw_invoke(
    client: &LoopbackTransport,
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let request = sign_invoke_request(seed_client(), session_id, sequence, request_id, op, payload);
    client
        .send(&serde_json::to_vec(&request).expect("request bytes"))
        .await
        .expect("request send");
    serde_json::from_slice(&client.recv().await.expect("response recv")).expect("response decode")
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_loopback_round_trips_capture_review_and_recall_through_the_typed_port() {
    let host = granted_host();
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let request = capture_request(
        "cap-loopback-1",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    let captured = expect_ok(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    let receipt = wire(&captured);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["op"], "memory.capture");
    assert_eq!(receipt["idempotency_key"], "cap-loopback-1");
    // The provider admits the authenticated dialing peer as the requester; the
    // payload never carries a principal (MEM-R003).
    assert_eq!(
        receipt["requester_principal_id"],
        format!(
            "prn_{}",
            derive_peer_id_from_ed25519_pubkey(&pubkey_client())
        ),
        "the receipt names the authenticated peer, got {receipt}"
    );
    assert!(
        request.get("requester_principal_id").is_none(),
        "the capture payload carries no principal"
    );
    assert_eq!(receipt["result"]["classified"], 1, "{receipt}");
    // §6.1: the digest is SHA-256 over the JCS canonicalization of the whole
    // request object (idempotency_key included) — recomputed here
    // independently of the fixture's receipt builder.
    let expected_digest = match jcs_sha256_hex(&request) {
        SpokeResult::Ok(digest) => digest,
        SpokeResult::Reject(reject) => panic!("request digest: {reject:?}"),
    };
    assert_eq!(
        receipt["payload_digest"], expected_digest,
        "receipt payload_digest must be the §6.1 JCS digest of the request, got {receipt}"
    );
    let candidate_id = receipt["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {receipt}"))
        .to_owned();

    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(fragmented_review_request(
                    SPACE,
                    &candidate_id,
                    "review-cand-1",
                    &[
                        (
                            "fact",
                            "The gate key is hidden under the third barrel.",
                        ),
                        ("event", "Mira revealed the key location to Bo."),
                    ],
                )),
            )
            .await,
    );
    let review_receipt = wire(&reviewed);
    assert_eq!(review_receipt["status"], "committed", "{review_receipt}");
    assert_eq!(
        review_receipt["result"]["outcome"], "fragmented",
        "{review_receipt}"
    );
    assert_eq!(
        review_receipt["result"]["record_ids"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "{review_receipt}"
    );

    let recalled = expect_ok(
        client
            .recall(
                &principal,
                typed_recall(recall_request(CONTEXT, "Where is the gate key?", 8)),
            )
            .await,
    );
    let page = wire(&recalled);
    let records = page["records"].as_array().expect("records array");
    assert_eq!(records.len(), 2, "{page}");
    assert_eq!(page["total_authorized"], 2);
    assert_eq!(page["truncated"], false);
    assert_eq!(page["engine_policy_version"], ENGINE_POLICY_VERSION);
    // Ranking (§9 policy: query overlap then kind weight): the fact fragment
    // matches the query tokens and outranks the event fragment.
    assert_eq!(records[0]["kind"], "fact", "{page}");
    assert_eq!(records[1]["kind"], "event", "{page}");
    // Placement inheritance: a review decision carries no placement field, so
    // the recalled records carry the captured candidate's scope.
    for record in records {
        assert_eq!(
            record["scope"],
            json!({ "visibility": "context", "context_id": CONTEXT }),
            "review outcomes inherit the candidate placement, got {record}"
        );
    }

    // Budget: a one-item cap truncates the same ranking and keeps the totals.
    let truncated = expect_ok(
        client
            .recall(
                &principal,
                typed_recall(recall_request(CONTEXT, "Where is the gate key?", 1)),
            )
            .await,
    );
    let page = wire(&truncated);
    let records = page["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1, "{page}");
    assert_eq!(page["truncated"], true);
    assert_eq!(page["total_authorized"], 2);
    assert_eq!(records[0]["kind"], "fact", "{page}");

    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_enforces_grants_idempotency_and_selection_isolation() {
    // Denial: a provider state with no grant for the caller.
    let host = ungranted_host();
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let denied = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-denied",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied.code,
        SpokeRejectCode::NotAuthorized,
        "an ungranted capture must deny: {denied:?}"
    );
    let denied = expect_reject(
        client
            .recall(
                &principal,
                typed_recall(recall_request(CONTEXT, "Where is the gate key?", 8)),
            )
            .await,
    );
    assert_eq!(
        denied.code,
        SpokeRejectCode::NotAuthorized,
        "an ungranted recall must deny: {denied:?}"
    );
    {
        let state = host.store.state.lock().expect("state lock");
        assert_eq!(state.candidates.len(), 0, "denied captures write nothing");
        assert_eq!(state.receipts.len(), 0, "denied ops issue no receipt");
        assert_eq!(state.records.len(), 0);
    }
    responder.close();
    client.close();

    // Granted provider: idempotent replay, digest conflict, selection + space
    // isolation.
    let host = granted_host();
    host.store
        .seed_record(SPACE, OTHER_CONTEXT, "fact", "Bo sold rope at the market.");
    host.store.seed_record(
        OTHER_SPACE,
        CONTEXT,
        "fact",
        "A copy of the run stored in another space.",
    );
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let request = capture_request(
        "cap-replay",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    let first = expect_ok(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    let replay = expect_ok(client.capture(&principal, typed_capture(request.clone())).await);
    assert_eq!(
        wire(&replay)["receipt_id"],
        wire(&first)["receipt_id"],
        "same key + same payload replays the original receipt"
    );
    assert_eq!(
        host.store.state.lock().expect("state lock").candidates.len(),
        1,
        "replay must not double-apply"
    );

    let conflict = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-replay",
                    CONTEXT,
                    "A different payload under the same idempotency key.",
                )),
            )
            .await,
    );
    assert_eq!(
        conflict.code,
        SpokeRejectCode::IdempotencyConflict,
        "same key + different payload digest must conflict: {conflict:?}"
    );

    // One retained record inside the selected context.
    let candidate_id = wire(&first)["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {:?}", wire(&first)))
        .to_owned();
    expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &candidate_id,
                    "review-isolation",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );

    // Isolation: the seeded OTHER_CONTEXT and OTHER_SPACE records are
    // authorized state but outside the selection, so they never appear.
    let recalled = expect_ok(
        client
            .recall(&principal, typed_recall(recall_request(CONTEXT, "", 8)))
            .await,
    );
    let page = wire(&recalled);
    let records = page["records"]
        .as_array()
        .unwrap_or_else(|| panic!("recall must answer records, got {page}"));
    assert_eq!(
        records.len(),
        1,
        "only the selected context in this space is authorized input: {page}"
    );
    assert_eq!(
        records[0]["scope"],
        json!({ "visibility": "context", "context_id": CONTEXT })
    );
    assert_eq!(page["total_authorized"], 1);
    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_rejects_mismatched_review_space_and_rechecks_on_replay() {
    let host = Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    });
    let principal_id = format!("prn_{}", derive_peer_id_from_ed25519_pubkey(&pubkey_client()));
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["capture", "records_write"],
        &[CONTEXT],
        false,
    );
    host.store.add_grant(
        &principal_id,
        OTHER_SPACE,
        Some(SUBJECT),
        &["review", "records_write"],
        &[CONTEXT],
        false,
    );

    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let captured = expect_ok(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-cross-space",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_owned();
    let receipts_after_capture = host.store.state.lock().expect("state lock").receipts.len();

    // Addressing another space is a different operation (§7.0.1/§7.15).
    let cross_space = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    OTHER_SPACE,
                    &candidate_id,
                    "review-cross-space",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        cross_space.code,
        SpokeRejectCode::NotAuthorized,
        "a review addressing another space must be rejected: {cross_space:?}"
    );
    {
        let state = host.store.state.lock().expect("state lock");
        assert!(!state.candidates[0].consumed, "denied review consumes nothing");
        assert_eq!(state.records.len(), 0, "denied review writes no record");
        assert_eq!(
            state.receipts.len(),
            receipts_after_capture,
            "denied review issues no receipt"
        );
    }

    // The mismatch is rejected on addressing alone.
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["review"],
        &[CONTEXT],
        false,
    );
    let still_mismatched = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    OTHER_SPACE,
                    &candidate_id,
                    "review-space-mismatch",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        still_mismatched.code,
        SpokeRejectCode::NotAuthorized,
        "the addressed space is part of the operation identity: {still_mismatched:?}"
    );
    assert!(
        !host.store.state.lock().expect("state lock").candidates[0].consumed,
        "a mismatched address consumes nothing"
    );

    // Positive control: the request names the candidate's own space.
    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &candidate_id,
                    "review-in-space",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    let receipt = wire(&reviewed);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["space_id"], SPACE, "{receipt}");
    {
        let state = host.store.state.lock().expect("state lock");
        assert!(state.candidates[0].consumed, "the addressed review consumes");
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].space_id, SPACE);
    }

    // A second mutating op, issued before coverage is withdrawn.
    let capture_body = capture_request(
        "cap-revoked",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    expect_ok(
        client
            .capture(&principal, typed_capture(capture_body.clone()))
            .await,
    );

    // Replay after coverage is withdrawn: the original receipt is never
    // disclosed to a caller who no longer has coverage (MEM-R021).
    host.store.state.lock().expect("state lock").grants.clear();
    let issued_before_replay = host.store.state.lock().expect("state lock").receipts.len();
    let replayed_review = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &candidate_id,
                    "review-in-space",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        replayed_review.code,
        SpokeRejectCode::NotAuthorized,
        "review replay without current coverage must deny, not disclose: {replayed_review:?}"
    );
    let replayed_capture = expect_reject(
        client
            .capture(&principal, typed_capture(capture_body))
            .await,
    );
    assert_eq!(
        replayed_capture.code,
        SpokeRejectCode::NotAuthorized,
        "capture replay without current coverage must deny, not disclose: {replayed_capture:?}"
    );
    assert_eq!(
        host.store.state.lock().expect("state lock").receipts.len(),
        issued_before_replay,
        "denied replays issue no new receipt"
    );
    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_enforces_shared_placement_coverage() {
    let host = Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    });
    let principal_id = format!("prn_{}", derive_peer_id_from_ed25519_pubkey(&pubkey_client()));
    // Restricted grant: non-empty context coverage, no shared access.
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["capture", "review", "records_write", "recall"],
        &[CONTEXT],
        false,
    );
    let shared_candidate = host.store.seed_candidate(None);

    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    // Shared placement is denied on capture, review and read without
    // `shared_access`.
    let denied_capture = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(shared_capture_request(
                    "cap-shared",
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied_capture.code,
        SpokeRejectCode::NotAuthorized,
        "a shared placement needs shared access: {denied_capture:?}"
    );
    let denied_review = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &shared_candidate,
                    "review-shared-denied",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied_review.code,
        SpokeRejectCode::NotAuthorized,
        "reviewing shared placement needs shared access: {denied_review:?}"
    );
    assert!(
        !host.store.state.lock().expect("state lock").candidates[0].consumed,
        "denied shared review consumes nothing"
    );
    let denied_recall = expect_reject(
        client
            .recall(&principal, typed_recall(shared_recall_request(8)))
            .await,
    );
    assert_eq!(
        denied_recall.code,
        SpokeRejectCode::NotAuthorized,
        "a shared read needs shared access: {denied_recall:?}"
    );

    // Corrected positive control: a restricted grant with non-empty context
    // coverage AND shared access.
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["capture", "review", "records_write", "recall"],
        &[CONTEXT],
        true,
    );
    let captured = expect_ok(
        client
            .capture(
                &principal,
                typed_capture(shared_capture_request(
                    "cap-shared",
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(wire(&captured)["status"], "committed");
    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &shared_candidate,
                    "review-shared",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(wire(&reviewed)["result"]["outcome"], "fragmented");

    let recalled = expect_ok(
        client
            .recall(&principal, typed_recall(shared_recall_request(8)))
            .await,
    );
    let page = wire(&recalled);
    let records = page["records"]
        .as_array()
        .unwrap_or_else(|| panic!("shared recall must answer records, got {page}"));
    assert_eq!(records.len(), 1, "{page}");
    assert_eq!(records[0]["scope"], json!({ "visibility": "shared" }));
    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_denies_unnegotiated_unserved_and_unknown_memory_ops() {
    // (a) Known op, capability not negotiated: `capability_missing` before any
    // handler runs — zero side effects. The host outlives the denied invoke so
    // its state can be inspected.
    let host = granted_host();
    let (responder, pair) = start_host(Some(memory_face(&host)), &["spoke-baseline"]).await;
    let manifest = memory_manifest("test-client", &["spoke-baseline"]);
    let client = dial(pair, &manifest).await;
    let denied = expect_reject(
        client
            .capture(
                &client_principal(),
                typed_capture(capture_request(
                    "cap-denied",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied.code,
        SpokeRejectCode::CapabilityPortMissing,
        "an unnegotiated memory op is a capability deny: {denied:?}"
    );
    assert_eq!(
        reject_wire_code(&denied).as_deref(),
        Some("capability_missing"),
        "{denied:?}"
    );
    assert_eq!(
        denied
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("terminal"),
        "§14.2: a memory-boundary deny is classified terminal: {denied:?}"
    );
    {
        let state = host.store.state.lock().expect("state lock");
        assert_eq!(state.candidates.len(), 0, "a capability deny admits no candidate");
        assert_eq!(state.receipts.len(), 0, "a capability deny issues no receipt");
        assert_eq!(state.records.len(), 0, "a capability deny writes no record");
    }
    responder.close();
    client.close();

    // (b) Capability negotiated but the host provides no memory face: the
    // dispatch-deny branch, `op_unsupported`.
    let (responder, pair) = start_host(None, &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest_with(&MEMORY_CAPABILITIES)).await;
    let unserved = expect_reject(
        client
            .capture(
                &client_principal(),
                typed_capture(capture_request(
                    "cap-unserved",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(unserved.code, SpokeRejectCode::CapabilityPortMissing);
    assert_eq!(
        reject_wire_code(&unserved).as_deref(),
        Some("op_unsupported"),
        "{unserved:?}"
    );
    assert!(
        unserved.message.contains("no memory service configured"),
        "{unserved:?}"
    );
    assert_eq!(
        unserved
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("terminal"),
        "§14.2: an absent memory face is classified terminal: {unserved:?}"
    );
    responder.close();
    client.close();

    // (c) Unknown `memory.*` op: no dispatch row, fail-closed `op_unsupported`.
    // The typed client cannot express an unknown op, so this rides the raw
    // wire (the client above moved the transport).
    let (responder, pair) = start_host(Some(memory_face(&granted_host())), &MEMORY_CAPABILITIES).await;
    let raw_manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let session_id = handshake(&pair.client, &raw_manifest).await;
    let unknown = raw_invoke(
        &pair.client,
        &session_id,
        0,
        "memory-unknown",
        "memory.archive.purge",
        json!({}),
    )
    .await;
    assert_eq!(unknown["error"]["code"], "op_unsupported", "{unknown}");
    assert_eq!(
        unknown["error"]["details"]["retry_class"], "terminal",
        "§14.2: an unknown memory op denied before the service is still terminal: {unknown}"
    );
    // Control: the memory-boundary classification must not leak onto another
    // op family — an unknown non-memory op keeps its bare `op_unsupported`.
    let non_memory = raw_invoke(
        &pair.client,
        &session_id,
        1,
        "memory-unknown-control",
        "photosynthesis.derive",
        json!({}),
    )
    .await;
    assert_eq!(non_memory["error"]["code"], "op_unsupported", "{non_memory}");
    assert!(
        non_memory["error"].get("details").is_none(),
        "a non-memory deny carries no memory retry metadata: {non_memory}"
    );
    responder.close();
}

fn manifest_with(capabilities: &[&str]) -> HostCapabilityManifest {
    memory_manifest("test-client", capabilities)
}

fn granted_store() -> Arc<Store> {
    let peer_id = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    Store::with_grant(&format!("prn_{peer_id}"), &[CONTEXT, OTHER_CONTEXT])
}

fn granted_host() -> Arc<MemoryHost> {
    Arc::new(MemoryHost {
        store: granted_store(),
    })
}

fn ungranted_host() -> Arc<MemoryHost> {
    Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    })
}

// ── Golden vectors driven through the client adapter ──────────────────────

/// Echoes the shared golden vector for every `memory.*` op it is asked for and
/// asserts the client encoded exactly the vector's request object.
struct VectorHost {
    vectors: Arc<Vec<Value>>,
}

#[async_trait]
impl spoke_connect::remote::RemoteMemoryService for VectorHost {
    async fn invoke(&self, _peer_id: &str, op: &str, payload: Value) -> SpokeResult<Value> {
        let vector = self
            .vectors
            .iter()
            .find(|vector| vector["op"] == op)
            .unwrap_or_else(|| panic!("no golden vector for {op}"));
        assert_eq!(
            payload, vector["request"],
            "{op} client payload drifted from the golden vector"
        );
        SpokeResult::Ok(vector["response"].clone())
    }
}

fn req<T: serde::de::DeserializeOwned>(vector: &Value) -> T {
    serde_json::from_value(vector["request"].clone())
        .unwrap_or_else(|error| panic!("{} request does not decode: {error}", vector["op"]))
}

#[tokio::test]
async fn memory_loopback_golden_vectors_round_trip_through_the_client_adapter() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/memory-invoke-golden.json"))
        .expect("golden fixture parses");
    let vectors = fixture["vectors"]
        .as_array()
        .expect("fixture carries vectors")
        .clone();
    assert!(vectors.len() >= 8, "the shared vector set is representative");

    let host = Arc::new(VectorHost {
        vectors: Arc::new(vectors.clone()),
    });
    let (responder, pair) = start_host(Some(host), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let principal = client_principal();

    for vector in &vectors {
        let op = vector["op"].as_str().expect("vector op");
        let response = match op {
            "memory.describe" => wire(&expect_ok(
                client.describe(&principal, req::<MemoryDescribeRequest>(vector)).await,
            )),
            "memory.capture" => wire(&expect_ok(
                client.capture(&principal, req::<MemoryCaptureRequest>(vector)).await,
            )),
            "memory.candidates.review" => wire(&expect_ok(
                client
                    .review_candidate(&principal, req::<MemoryCandidatesReviewRequest>(vector))
                    .await,
            )),
            "memory.recall" => wire(&expect_ok(
                client.recall(&principal, req::<MemoryRecallRequest>(vector)).await,
            )),
            "memory.profile.update" => wire(&expect_ok(
                client
                    .update_profile(&principal, req::<MemoryProfileUpdateRequest>(vector))
                    .await,
            )),
            "memory.exports.create" => wire(&expect_ok(
                client
                    .create_export(&principal, req::<MemoryExportsCreateRequest>(vector))
                    .await,
            )),
            "memory.exports.get" => wire(&expect_ok(
                client
                    .get_export(&principal, req::<MemoryExportsGetRequest>(vector))
                    .await,
            )),
            "memory.imports.stage" => wire(&expect_ok(
                client
                    .stage_import(&principal, req::<MemoryImportsStageRequest>(vector))
                    .await,
            )),
            "memory.receipts.get" => wire(&expect_ok(
                client
                    .get_receipt(&principal, req::<MemoryReceiptsGetRequest>(vector))
                    .await,
            )),
            other => panic!("vector {other} has no adapter arm"),
        };
        assert_eq!(
            response, vector["response"],
            "{op} response drifted from the golden vector"
        );
    }

    responder.close();
    client.close();
}


#[test]
fn memory_port_service_classifies_a_malformed_payload_as_terminal() {
    let port: Arc<dyn MemoryPort + Send + Sync> = granted_host();
    let service = MemoryPortService::new(port, admission());
    let outcome = futures::executor::block_on(service.invoke(
        "peer-invalid",
        "memory.capture",
        json!({ "space_id": 1 }),
    ));
    let reject = expect_reject(outcome);
    assert_eq!(reject.code, SpokeRejectCode::InvalidInput, "{reject:?}");
    assert_eq!(
        reject
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("terminal"),
        "§14.2: a memory-boundary validation failure is terminal: {reject:?}"
    );
}

/// How the fixture's admitted capture settles (§14.3 shapes).
#[derive(Clone, Copy)]
enum DelayedCommit {
    /// The provider self-detaches the commit (the TS-path shape — the old
    /// fixture): the commit lands even if the responder drops the invoke.
    SelfDetached,
    /// The commit is part of the admitted invoke future itself, so it lands
    /// only if the responder keeps the task alive past its response deadline.
    InFlight,
    /// The admitted invoke panics, so the responder's detached task dies
    /// after admission with no result at all.
    Panics,
}

/// Provider fixture whose mutation commits asynchronously: the responder's
/// budget expires while the commit is still in flight, which is exactly the
/// uncertain-outcome case (the transport cannot know whether it committed).
struct DelayedHost {
    store: Arc<Store>,
    commit: DelayedCommit,
}

impl DelayedHost {
    fn commit_now(
        &self,
        principal: &Principal,
        payload: Value,
    ) -> SpokeResult<MemoryCaptureResponse> {
        match self.store.capture(principal, payload) {
            SpokeResult::Ok(value) => match serde_json::from_value(value) {
                Ok(response) => SpokeResult::Ok(response),
                Err(error) => reject(
                    SpokeRejectCode::InternalError,
                    format!("capture response decode failed: {error}"),
                ),
            },
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        }
    }
}

#[async_trait]
impl MemoryPort for DelayedHost {
    async fn describe(
        &self,
        _principal: &Principal,
        _request: MemoryDescribeRequest,
    ) -> SpokeResult<MemoryDescribeResponse> {
        not_served()
    }

    async fn list_spaces(
        &self,
        _principal: &Principal,
        _request: MemorySpacesListRequest,
    ) -> SpokeResult<MemorySpacesListResponse> {
        not_served()
    }

    async fn freeze_space(
        &self,
        _principal: &Principal,
        _request: MemorySpacesFreezeRequest,
    ) -> SpokeResult<MemorySpacesFreezeResponse> {
        not_served()
    }

    async fn unfreeze_space(
        &self,
        _principal: &Principal,
        _request: MemorySpacesUnfreezeRequest,
    ) -> SpokeResult<MemorySpacesUnfreezeResponse> {
        not_served()
    }

    async fn ensure_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsEnsureRequest,
    ) -> SpokeResult<MemorySubjectsEnsureResponse> {
        not_served()
    }

    async fn get_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsGetRequest,
    ) -> SpokeResult<MemorySubjectsGetResponse> {
        not_served()
    }

    async fn fence_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsFenceRequest,
    ) -> SpokeResult<MemorySubjectsFenceResponse> {
        not_served()
    }

    async fn list_grants(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsListRequest,
    ) -> SpokeResult<MemoryGrantsListResponse> {
        not_served()
    }

    async fn grant(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsGrantRequest,
    ) -> SpokeResult<MemoryGrantsGrantResponse> {
        not_served()
    }

    async fn revoke_grant(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsRevokeRequest,
    ) -> SpokeResult<MemoryGrantsRevokeResponse> {
        not_served()
    }

    async fn capture(
        &self,
        principal: &Principal,
        request: MemoryCaptureRequest,
    ) -> SpokeResult<MemoryCaptureResponse> {
        let payload = wire(&request);
        let key = payload["idempotency_key"].as_str().unwrap_or_default();
        // A replay of an already-committed key answers from provider state
        // immediately (MEM-R008: replay never re-executes).
        if self.store.has_receipt("memory.capture", key) {
            return self.commit_now(principal, payload);
        }
        match self.commit {
            DelayedCommit::InFlight => {
                // The commit is part of the admitted invoke: it only lands if
                // the responder keeps the provider task alive past the
                // deadline.
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                return self.commit_now(principal, payload);
            }
            DelayedCommit::Panics => {
                // The admitted task dies after invocation: the responder has
                // no result and cannot assert that nothing committed.
                panic!("memory provider panicked after admission");
            }
            DelayedCommit::SelfDetached => {}
        }
        let store = Arc::clone(&self.store);
        let principal = principal.clone();
        // Detached commit: the responder's budget may expire first, and the
        // transport has no way to know the outcome.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = store.capture(&principal, payload);
        });
        // Outlive the budget; this wait is dropped when the responder expires.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        not_served()
    }

    async fn list_candidates(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesListRequest,
    ) -> SpokeResult<MemoryCandidatesListResponse> {
        not_served()
    }

    async fn get_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesGetRequest,
    ) -> SpokeResult<MemoryCandidatesGetResponse> {
        not_served()
    }

    async fn dismiss_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesDismissRequest,
    ) -> SpokeResult<MemoryCandidatesDismissResponse> {
        not_served()
    }

    async fn review_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesReviewRequest,
    ) -> SpokeResult<MemoryCandidatesReviewResponse> {
        not_served()
    }

    async fn create_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsCreateRequest,
    ) -> SpokeResult<MemoryRecordsCreateResponse> {
        not_served()
    }

    async fn get_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsGetRequest,
    ) -> SpokeResult<MemoryRecordsGetResponse> {
        not_served()
    }

    async fn update_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsUpdateRequest,
    ) -> SpokeResult<MemoryRecordsUpdateResponse> {
        not_served()
    }

    async fn delete_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsDeleteRequest,
    ) -> SpokeResult<MemoryRecordsDeleteResponse> {
        not_served()
    }

    async fn list_records(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsListRequest,
    ) -> SpokeResult<MemoryRecordsListResponse> {
        not_served()
    }

    async fn promote_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsPromoteRequest,
    ) -> SpokeResult<MemoryRecordsPromoteResponse> {
        not_served()
    }

    async fn recall(
        &self,
        _principal: &Principal,
        _request: MemoryRecallRequest,
    ) -> SpokeResult<MemoryRecallResponse> {
        not_served()
    }

    async fn get_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileGetRequest,
    ) -> SpokeResult<MemoryProfileGetResponse> {
        not_served()
    }

    async fn update_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileUpdateRequest,
    ) -> SpokeResult<MemoryProfileUpdateResponse> {
        not_served()
    }

    async fn get_reflection_state(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsStateRequest,
    ) -> SpokeResult<MemoryReflectionsStateResponse> {
        not_served()
    }

    async fn list_reflections(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsListRequest,
    ) -> SpokeResult<MemoryReflectionsListResponse> {
        not_served()
    }

    async fn generate_reflection(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsGenerateRequest,
    ) -> SpokeResult<MemoryReflectionsGenerateResponse> {
        not_served()
    }

    async fn get_receipt(
        &self,
        _principal: &Principal,
        _request: MemoryReceiptsGetRequest,
    ) -> SpokeResult<MemoryReceiptsGetResponse> {
        not_served()
    }

    async fn create_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse> {
        not_served()
    }

    async fn get_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse> {
        not_served()
    }

    async fn stage_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse> {
        not_served()
    }

    async fn activate_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse> {
        not_served()
    }

    async fn discard_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse> {
        not_served()
    }
}

#[tokio::test]
async fn memory_loopback_reports_an_uncertain_outcome_and_recovers_on_replay() {
    let store = granted_store();
    let delayed_port: Arc<dyn MemoryPort + Send + Sync> = Arc::new(DelayedHost {
        store: Arc::clone(&store),
        commit: DelayedCommit::SelfDetached,
    });
    let delayed: Arc<dyn RemoteMemoryService> =
        Arc::new(MemoryPortService::new(delayed_port, admission()));
    // Short responder budget: it expires while the provider's commit is still
    // in flight.
    let (responder, pair) =
        start_host_with_budget(Some(delayed), &MEMORY_CAPABILITIES, Some(50)).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let principal = client_principal();
    let request = capture_request(
        "cap-uncertain",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );

    let timed_out = expect_reject(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    assert_eq!(
        timed_out.code,
        SpokeRejectCode::OutcomeUncertain,
        "§14.3: an expired memory budget after invocation is uncertain, not internal: {timed_out:?}"
    );
    assert_eq!(
        timed_out
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("uncertain"),
        "{timed_out:?}"
    );

    // Let the provider's detached commit land, then recover via the same key.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let recovered = expect_ok(client.capture(&principal, typed_capture(request)).await);
    let receipt = wire(&recovered);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["idempotency_key"], "cap-uncertain", "{receipt}");
    assert_eq!(
        store.state.lock().expect("state lock").receipts.len(),
        1,
        "the replay reuses the committed receipt instead of double-applying"
    );

    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_keeps_the_admitted_task_alive_past_the_budget() {
    // The provider does NOT self-detach: the commit is part of the admitted
    // invoke future, so it lands only if the responder spawns the invoke
    // instead of dropping it at the deadline (§14.3 — the responder neither
    // cancels nor rolls back admitted provider work).
    let store = granted_store();
    let delayed_port: Arc<dyn MemoryPort + Send + Sync> = Arc::new(DelayedHost {
        store: Arc::clone(&store),
        commit: DelayedCommit::InFlight,
    });
    let delayed: Arc<dyn RemoteMemoryService> =
        Arc::new(MemoryPortService::new(delayed_port, admission()));
    let (responder, pair) =
        start_host_with_budget(Some(delayed), &MEMORY_CAPABILITIES, Some(50)).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let principal = client_principal();
    let request = capture_request(
        "cap-detached",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );

    let timed_out = expect_reject(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    assert_eq!(
        timed_out.code,
        SpokeRejectCode::OutcomeUncertain,
        "§14.3: an expired memory budget after invocation is uncertain, not internal: {timed_out:?}"
    );

    // The detached invoke ran to its real completion: the committed receipt
    // is observable when the same idempotency key is replayed.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let recovered = expect_ok(client.capture(&principal, typed_capture(request)).await);
    let receipt = wire(&recovered);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["idempotency_key"], "cap-detached", "{receipt}");
    assert_eq!(
        store.state.lock().expect("state lock").receipts.len(),
        1,
        "the admitted task committed exactly once"
    );

    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_reports_an_uncertain_outcome_when_the_admitted_task_panics() {
    // §14.3: the detached invoke dies after admission with no result, so the
    // responder cannot assert that nothing committed — the answer is
    // OUTCOME_UNCERTAIN/uncertain, never a clean INTERNAL_ERROR.
    let store = granted_store();
    let panicking_port: Arc<dyn MemoryPort + Send + Sync> = Arc::new(DelayedHost {
        store: Arc::clone(&store),
        commit: DelayedCommit::Panics,
    });
    let panicking: Arc<dyn RemoteMemoryService> =
        Arc::new(MemoryPortService::new(panicking_port, admission()));
    let (responder, pair) =
        start_host_with_budget(Some(panicking), &MEMORY_CAPABILITIES, Some(500)).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let principal = client_principal();

    let failed = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-panic",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        failed.code,
        SpokeRejectCode::OutcomeUncertain,
        "§14.3: a panicked memory invoke is uncertain, not a clean failure: {failed:?}"
    );
    let details = failed.details.expect("uncertain carries details");
    assert_eq!(details["retry_class"], "uncertain", "{details:?}");
    assert_eq!(details["kind"], "panic", "{details:?}");

    responder.close();
    client.close();
}

// ── C8 combined-host no-KE boundary (P3/T1) ───────────────────────────────

/// Order-independent snapshot of the seeded ToyWorld KE authority as observed
/// through the very `RemoteAdapter` that drives the Memory lifecycle.
async fn knowledge_entry_state(client: &RemoteAdapter, scope: &Scope) -> Value {
    let mut entries: Vec<KnowledgeEntry> = expect_ok(client.list_knowledge_entries(scope).await);
    entries.sort_by(|left, right| left.entry_id.as_str().cmp(right.entry_id.as_str()));
    serde_json::to_value(entries).expect("KE snapshot serializes")
}

async fn assert_knowledge_entries_unchanged(
    client: &RemoteAdapter,
    scope: &Scope,
    expected: &Value,
    stage: &str,
) {
    let actual = knowledge_entry_state(client, scope).await;
    assert_eq!(
        &actual, expected,
        "Memory stage `{stage}` changed the consumer-visible KE authority"
    );
}

/// C8 state boundary: with the real `SqliteMemoryProvider` and the seeded
/// `ToyWorldAdapter` behind one combined host, every Memory lifecycle stage
/// must leave the complete KnowledgeEntry state exactly unchanged.
#[tokio::test]
async fn sqlite_cognition_lifecycle_leaves_knowledge_entries_unchanged() {
    // The reviewed C8 provider, provisioned with the loopback space/subject and
    // one unrestricted admin grant for the authenticated peer's principal.
    let provider = Arc::new(SqliteMemoryProvider::in_memory());
    provider.provision_space(SPACE, "memory-loopback-host", &[]);
    provider.provision_subject(SPACE, SUBJECT, None);
    let principal = client_principal();
    provider.provision_grant(
        SPACE,
        json!({
            "schema_version": 1,
            "grant_id": "grt_loopback_no_ke_admin",
            "space_id": SPACE,
            "principal_id": principal.principal_id,
            "capabilities": ["admin"],
            "created_at": "2026-07-23T05:36:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    let port: Arc<dyn MemoryPort + Send + Sync> = provider.clone();
    let memory: Arc<dyn RemoteMemoryService> = Arc::new(MemoryPortService::new(port, admission()));

    let (responder, pair) = start_host(Some(memory), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;

    // Cross-store oracle: the belief is attributed to the seeded ToyWorld
    // knowledge entry, so any Memory-driven KE synchronization is observable.
    let scope: Scope = serde_json::from_value(json!({ "scope_id": "no-ke-boundary" }))
        .expect("valid Scope");
    let expected = knowledge_entry_state(&client, &scope).await;
    let baseline = expected.as_array().expect("KE snapshot array");
    assert!(
        !baseline.is_empty(),
        "the seeded KE authority must be non-empty: {expected}"
    );
    assert!(
        baseline
            .iter()
            .any(|entry| entry["entry_id"] == json!("kb_tw_mira")),
        "the seeded holder entry must be part of the oracle: {expected}"
    );

    let vector_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cognition/memory-lifecycle-vectors.json"
    );
    let vectors: Value = serde_json::from_str(
        &std::fs::read_to_string(vector_path).expect("lifecycle vectors readable"),
    )
    .expect("lifecycle vectors parse");
    let admission_vector = vectors["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .find(|vector| vector["id"] == "lifecycle-admission-cognition-valid")
        .expect("valid cognition lifecycle vector present");
    let holder =
        json!({ "kind": "knowledge_entry", "namespace": "toy_world", "id": "kb_tw_mira" });
    let mut content = admission_vector["content"].clone();
    content["cognition"][0]["holder"] = holder.clone();
    content["cognition"][0]["assessor"] = holder.clone();
    let original_assessment = content["cognition"][0].clone();

    // Capture: classified and pending, never a KnowledgeEntry.
    let captured = expect_ok(
        client
            .capture(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "content": content.clone(),
                    "capture_scope": { "visibility": "context", "context_id": CONTEXT },
                    "source_context_ids": [CONTEXT],
                    "kind_hint": "fact",
                    "idempotency_key": "no-ke-cap-1",
                }))
                .expect("capture request"),
            )
            .await,
    );
    let captured = wire(&captured);
    assert_eq!(captured["status"], json!("committed"));
    let candidate_id = captured["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "capture").await;

    // Review: the only candidate → record path, still no KE.
    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                    "expected_revision": 1,
                    "decision": { "action": "promote", "kind": "fact" },
                    "idempotency_key": "no-ke-review-1",
                }))
                .expect("review request"),
            )
            .await,
    );
    let reviewed = wire(&reviewed);
    assert_eq!(reviewed["result"]["outcome"], json!("promoted"));
    let record_id = reviewed["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "review").await;

    // Recorded-at-time update: the original assessment is retained verbatim and
    // a second, identically attributed reassessment is appended.
    let mut reassessment = original_assessment.clone();
    reassessment["record_id"] = json!("rec_belief_mem_2");
    reassessment["assessed_at"] = json!("2026-07-24T06:05:00Z");
    reassessment["q"]["value"] = json!(0.4);
    let updated_content = json!({
        "text": content["text"].clone(),
        "cognition": [original_assessment.clone(), reassessment],
    });
    let updated = expect_ok(
        client
            .update_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": 1,
                    "content": updated_content.clone(),
                    "idempotency_key": "no-ke-update-1",
                }))
                .expect("update request"),
            )
            .await,
    );
    assert_eq!(wire(&updated)["result"]["revision"], json!(2));
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "recorded-at update").await;

    // Placement promotion: revision advances, content is untouched.
    let promoted = expect_ok(
        client
            .promote_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": 2,
                    "target_scope": { "visibility": "shared" },
                    "idempotency_key": "no-ke-promote-1",
                }))
                .expect("promote request"),
            )
            .await,
    );
    assert_eq!(wire(&promoted)["result"]["revision"], json!(3));
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "placement promote").await;

    // Retained read: the two-assessment content survives the placement move.
    let record = expect_ok(
        client
            .get_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let record = wire(&record)["record"].clone();
    assert_eq!(record["content"], updated_content);
    assert_eq!(record["revision"], json!(3));
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "retained read").await;

    // Full retirement scrubs Memory content and still leaves every KE intact.
    let deleted = expect_ok(
        client
            .delete_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": 3,
                    "idempotency_key": "no-ke-delete-1",
                }))
                .expect("delete request"),
            )
            .await,
    );
    assert_eq!(wire(&deleted)["result"]["tombstoned"], json!(true));
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "delete").await;

    // Export creation, then every export chunk read.
    let created = expect_ok(
        client
            .create_export(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "include_tombstoned": true,
                    "require_quiescent": false,
                    "idempotency_key": "no-ke-export-1",
                }))
                .expect("export request"),
            )
            .await,
    );
    let receipt_id = wire(&created)["receipt_id"]
        .as_str()
        .expect("export receipt id")
        .to_string();
    assert_knowledge_entries_unchanged(&client, &scope, &expected, "export creation").await;

    // Deferred receipt completion uses the provider's test-support worker, not
    // a read path (MEM-R011).
    let completed = provider
        .poll_receipt_value(SPACE, &receipt_id)
        .expect("export receipt present");
    assert_eq!(completed["status"], json!("committed"), "{completed}");
    let export_id = completed["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunk_count = completed["result"]["chunk_count"]
        .as_u64()
        .expect("chunk count");
    assert!(chunk_count >= 1, "the export must carry at least one chunk");
    for ordinal in 0..chunk_count {
        let chunk = expect_ok(
            client
                .get_export(
                    &principal,
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": ordinal,
                    }))
                    .expect("export chunk request"),
                )
                .await,
        );
        assert!(wire(&chunk)["chunk"].is_object(), "chunk {ordinal}");
        assert_knowledge_entries_unchanged(&client, &scope, &expected, "export chunk read").await;
    }

    responder.close();
    client.close();
}

// ── spoke-cognition public-session vectors (consumer contract §2) ─────────

/// The shared negotiation/requirement vectors — the SAME file the TypeScript
/// suite and the responder unit tests consume, so both languages agree on the
/// host manifests, payloads and exact rejection fields.
const COGNITION_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/spoke-connect-ts/tests/fixtures/cognition-negotiation.json"
));

const COGNITION_SCOPE_OP: &str = "port.scope.list_knowledge_entries";

fn cognition_manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": capabilities,
        "namespaces": ["world_a"],
        "extensions": {},
    }))
    .expect("valid manifest")
}

fn fixture_capabilities(host: &Value) -> Vec<String> {
    host.get("capabilities")
        .and_then(Value::as_array)
        .expect("host capabilities")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn capability_refs(capabilities: &[String]) -> Vec<&str> {
    capabilities.iter().map(String::as_str).collect()
}

fn expected_entry_ids(case: &Value) -> Vec<String> {
    case.get("expect_entry_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Counting KE-only source over the shared fixture entries. A denied
/// requirement must leave `scope_reads` at zero; a served selection reaches
/// it exactly once and answers the entries whose current-view standard
/// cognition satisfies the selection through the shared C6 matcher — a
/// discriminating result, never an unconditional echo.
struct CognitionKeSource {
    entries: Vec<Value>,
    scope_reads: AtomicUsize,
    returned_entry_ids: Mutex<Vec<String>>,
}

impl CognitionKeSource {
    fn new(entries: Vec<Value>) -> Self {
        Self {
            entries,
            scope_reads: AtomicUsize::new(0),
            returned_entry_ids: Mutex::new(Vec::new()),
        }
    }

    /// Apply the requested current-view selection with the shared C6 matcher.
    /// An absent selection returns the whole source (no cognitive refinement);
    /// absent optional record members are never read as zero.
    fn select(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        let scope_json = match serde_json::to_value(scope) {
            Ok(scope_json) => scope_json,
            Err(error) => {
                return reject(
                    SpokeRejectCode::InternalError,
                    format!("scope serialize failed: {error}"),
                )
            }
        };
        let Some(selection) = scope_json.get("cognition") else {
            return self.matching_entries(|_| true);
        };
        let current = selection.get("view").and_then(Value::as_str) == Some("current");
        let filter: spoke_schemas::data::cognition::CognitionFilter =
            match selection.get("filter").cloned() {
                Some(filter) => match serde_json::from_value(filter) {
                    Ok(filter) => filter,
                    Err(error) => {
                        return reject(
                            SpokeRejectCode::InvalidInput,
                            format!("fixture filter: {error}"),
                        )
                    }
                },
                None => return reject(SpokeRejectCode::InvalidInput, "missing filter"),
            };
        self.matching_entries(|entry| {
            current
                && fixture_entry_records(entry)
                    .iter()
                    .any(|record| cognition_record_matches_filter(record, &filter))
        })
    }

    fn matching_entries(
        &self,
        keep: impl Fn(&Value) -> bool,
    ) -> SpokeResult<Vec<KnowledgeEntry>> {
        let mut selected = Vec::new();
        let mut ids = Vec::new();
        for entry in &self.entries {
            if !keep(entry) {
                continue;
            }
            match serde_json::from_value::<KnowledgeEntry>(entry.clone()) {
                Ok(entry) => {
                    ids.push(entry.entry_id.clone());
                    selected.push(entry);
                }
                Err(error) => {
                    return reject(
                        SpokeRejectCode::InternalError,
                        format!("fixture entry decode failed: {error}"),
                    )
                }
            }
        }
        *self.returned_entry_ids.lock().expect("returned lock") = ids;
        SpokeResult::Ok(selected)
    }
}

/// A fixture entry's `modules.cognition.records`, each validated by the C5
/// record gate before it is read as a typed record.
fn fixture_entry_records(entry: &Value) -> Vec<spoke_schemas::CognitionRecord> {
    let Some(records) = entry
        .pointer("/modules/cognition/records")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    records
        .iter()
        .map(|record| {
            assert!(
                validate_cognition_record(record).is_ok(),
                "fixture record fails C5: {record}"
            );
            serde_json::from_value(record.clone()).expect("validated fixture record")
        })
        .collect()
}

#[async_trait]
impl KnowledgeEntryPort for CognitionKeSource {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        reject(
            SpokeRejectCode::KnowledgeEntryNotFound,
            format!("fixture has no entry {entry_id}"),
        )
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        _expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        SpokeResult::Ok(entry)
    }
}

#[async_trait]
impl RelationPort for CognitionKeSource {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        reject(
            SpokeRejectCode::RelationNotFound,
            format!("fixture has no relation {relation_id}"),
        )
    }

    async fn put_relation(
        &self,
        relation: Relation,
        _expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        SpokeResult::Ok(relation)
    }
}

#[async_trait]
impl ScopeQueryPort for CognitionKeSource {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.scope_reads.fetch_add(1, Ordering::SeqCst);
        self.select(scope)
    }

    async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        SpokeResult::Ok(Vec::new())
    }
}

#[async_trait]
impl FindingPort for CognitionKeSource {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        SpokeResult::Ok(findings)
    }
}

#[async_trait]
impl RuleQueryPort for CognitionKeSource {
    async fn list_rules(&self, _rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        SpokeResult::Ok(Vec::new())
    }
}

#[async_trait]
impl HostManifestPort for CognitionKeSource {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        SpokeResult::Ok(cognition_manifest("cognition-ke-source", &[]))
    }

    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        SpokeResult::Ok(Vec::new())
    }
}

/// Counting memory face with decodable success bodies. A denied requirement
/// must leave `invokes` at zero, so the Memory boundary itself is observable;
/// the served probe answers a decodable page or receipt.
#[derive(Default)]
struct CognitionMemoryService {
    invokes: AtomicUsize,
    last_payload: Mutex<Option<Value>>,
}

impl CognitionMemoryService {
    fn committed_receipt(op: &str, idempotency_key: &str) -> Value {
        json!({
            "schema_version": 1,
            "receipt_id": "rcpt_cognition_public_proof",
            "op": op,
            "space_id": SPACE,
            "requester_principal_id": "prn_cognition_public_proof",
            "idempotency_key": idempotency_key,
            "payload_digest": "0".repeat(64),
            "status": "committed",
            "result": { "record_id": "rec_cognition_public_proof", "revision": 1 },
            "created_at": "2026-10-28T09:20:00Z",
            "completed_at": "2026-10-28T09:20:00Z",
            "extensions": {},
        })
    }
}

#[async_trait]
impl RemoteMemoryService for CognitionMemoryService {
    async fn invoke(&self, _peer_id: &str, op: &str, payload: Value) -> SpokeResult<Value> {
        self.invokes.fetch_add(1, Ordering::SeqCst);
        *self.last_payload.lock().expect("payload lock") = Some(payload.clone());
        match op {
            "memory.records.create" => SpokeResult::Ok(Self::committed_receipt(
                op,
                payload
                    .get("idempotency_key")
                    .and_then(Value::as_str)
                    .unwrap_or("cognition-public-proof"),
            )),
            _ => SpokeResult::Ok(json!({ "records": [], "total_authorized": 0 })),
        }
    }
}

/// Start a responder whose ports face carries the counting KE source and the
/// counting memory service, over a real loopback pair.
async fn start_cognition_host(
    source: Arc<CognitionKeSource>,
    memory: Arc<CognitionMemoryService>,
    capabilities: &[&str],
) -> (Arc<ConnectResponder>, LoopbackTransportPair) {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let ports = RemoteServePortsComposite::new(source, None, None).with_memory(memory);
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(ports);
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity { seed: seed_host() },
        manifest: cognition_manifest("cognition-vector-host", capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: None,
    })
    .await;
    (responder, pair)
}

/// The typed client path sends exactly `{ "scope": <Scope> }`. A payload with
/// any other top-level member is not representable on that path (the typed
/// request would drop the member), so it MUST take the raw signed session
/// instead — the top-level `cognition` positive is the discriminating case.
fn typed_scope(payload: &Value) -> Option<Scope> {
    if payload.as_object()?.len() != 1 {
        return None;
    }
    let scope = payload.get("scope")?;
    let decoded: Scope = serde_json::from_value(scope.clone()).ok()?;
    (serde_json::to_value(&decoded).ok()? == *scope).then_some(decoded)
}

fn typed_memory_list(payload: &Value) -> Option<MemoryRecordsListRequest> {
    let request: MemoryRecordsListRequest = serde_json::from_value(payload.clone()).ok()?;
    (serde_json::to_value(&request).ok()? == *payload).then_some(request)
}

/// Every shared served vector through the REAL public boundary: a signed
/// loopback handshake between `connect_responder` (the independent Rust
/// responder) and a raw signed-session invoke carrying the fixture payload
/// EXACTLY — including the malformed Scope and Memory data the generated
/// request types cannot represent. Asserts the host's exact raw lowercase
/// wire code plus `reason`/`field`, and that the specific KE/Memory boundary
/// counters stay at their expected value (zero on every denial).
#[tokio::test]
async fn cognition_shared_vectors_answer_on_the_public_responder_session() {
    let fixture: Value = serde_json::from_str(COGNITION_FIXTURE).expect("fixture json");
    let hosts = fixture.get("hosts").expect("fixture hosts");
    let entries = fixture
        .get("ke_entries")
        .and_then(Value::as_array)
        .expect("fixture ke_entries")
        .clone();
    let cases = fixture
        .get("cases")
        .and_then(Value::as_array)
        .expect("fixture cases");
    // The two C7 field-path negatives are the only machine-checked proof of the
    // escaped §2.2 field contract; a dropped row must fail by case identity.
    for (case_id, field) in [
        (
            "scope_carrier_empty_member_keeps_its_path",
            "scope.cognition.filter[\"\"]",
        ),
        (
            "memory_carrier_delimiter_member_keeps_its_path",
            "cognition_filter.q[\"a.b\"]",
        ),
    ] {
        let case = cases
            .iter()
            .find(|case| {
                case.get("kind").and_then(Value::as_str) == Some("served")
                    && case.get("id").and_then(Value::as_str) == Some(case_id)
            })
            .unwrap_or_else(|| panic!("load-bearing served vector missing: {case_id}"));
        assert_eq!(
            case["expect"]["field"].as_str(),
            Some(field),
            "{case_id}: escaped wire field"
        );
        assert_eq!(
            case["expect_provider_calls"].as_u64(),
            Some(0),
            "{case_id}: a denial reaches no provider"
        );
    }
    for case in cases {
        if case.get("kind").and_then(Value::as_str) != Some("served") {
            continue;
        }
        let case_id = case.get("id").and_then(Value::as_str).expect("case id");
        let host = hosts
            .get(case.get("host").and_then(Value::as_str).expect("host"))
            .expect("host entry");
        let client = hosts
            .get(case.get("client").and_then(Value::as_str).expect("client"))
            .expect("client entry");
        let host_capabilities = fixture_capabilities(host);
        let client_capabilities = fixture_capabilities(client);
        let op = case.get("op").and_then(Value::as_str).expect("op");
        let payload = case.get("payload").expect("payload").clone();
        let expected = case.get("expect").expect("expect");
        let expected_calls = case
            .get("expect_provider_calls")
            .and_then(Value::as_u64)
            .expect("expect_provider_calls");

        let source = Arc::new(CognitionKeSource::new(entries.clone()));
        let memory = Arc::new(CognitionMemoryService::default());
        let (responder, pair) = start_cognition_host(
            Arc::clone(&source),
            Arc::clone(&memory),
            &capability_refs(&host_capabilities),
        )
        .await;
        let session_id = handshake(
            &pair.client,
            &cognition_manifest("cognition-vector-client", &capability_refs(&client_capabilities)),
        )
        .await;
        let response =
            raw_invoke(&pair.client, &session_id, 0, case_id, op, payload.clone()).await;

        let scope_reads = source.scope_reads.load(Ordering::SeqCst);
        let memory_invokes = memory.invokes.load(Ordering::SeqCst);
        let (expected_scope, expected_memory) = if op.starts_with("memory.") {
            (0, expected_calls)
        } else {
            (expected_calls, 0)
        };
        assert_eq!(
            u64::try_from(scope_reads).expect("reads fit"),
            expected_scope,
            "{case_id}: KE source reads"
        );
        assert_eq!(
            u64::try_from(memory_invokes).expect("invokes fit"),
            expected_memory,
            "{case_id}: memory service invokes"
        );

        if expected.get("status").and_then(Value::as_str) == Some("ok") {
            assert!(
                response.get("error").is_none(),
                "{case_id}: expected success, got {response}"
            );
            assert!(
                response.get("payload").is_some(),
                "{case_id}: the success envelope carries the served payload: {response}"
            );
            responder.close();
            continue;
        }
        let code = expected.get("code").and_then(Value::as_str).expect("code");
        // Exact wire parity (consumer contract §2.2) read from the RAW frame:
        // the lowercase dispatch spelling the responder must emit verbatim.
        assert_eq!(
            response["error"]["code"].as_str(),
            Some(code),
            "{case_id}: {response}"
        );
        assert!(
            response.get("payload").is_none(),
            "{case_id}: a denial carries no success payload: {response}"
        );
        if code == "invalid_input" {
            assert_eq!(
                response["error"]["details"]["reason"],
                expected
                    .get("reason")
                    .and_then(Value::as_str)
                    .expect("reason"),
                "{case_id}: {response}"
            );
            assert_eq!(
                response["error"]["details"]["field"],
                expected
                    .get("field")
                    .and_then(Value::as_str)
                    .expect("field"),
                "{case_id}: {response}"
            );
        }
        responder.close();
    }
}

/// Every shared SUCCESS vector decoded on the real client surface: the typed
/// `RemoteAdapter` result for payloads it can carry, and — for the raw-only
/// positives the generated request types cannot represent verbatim (the
/// top-level `cognition` member and the Memory `scope` member) — the wire
/// payload decoded with its own op response type exactly as the client
/// decodes it. Asserts the exact retained entry ids and the Memory page's
/// missing-versus-zero shape, with the source's own log as a supporting
/// control only.
#[tokio::test]
async fn cognition_shared_success_vectors_decode_on_the_real_client() {
    let fixture: Value = serde_json::from_str(COGNITION_FIXTURE).expect("fixture json");
    let hosts = fixture.get("hosts").expect("fixture hosts");
    let entries = fixture
        .get("ke_entries")
        .and_then(Value::as_array)
        .expect("fixture ke_entries")
        .clone();
    let cases = fixture
        .get("cases")
        .and_then(Value::as_array)
        .expect("fixture cases");
    let mut raw_only_ids: Vec<String> = Vec::new();
    for case in cases {
        if case.get("kind").and_then(Value::as_str) != Some("served")
            || case["expect"]["status"].as_str() != Some("ok")
        {
            continue;
        }
        let case_id = case.get("id").and_then(Value::as_str).expect("case id");
        let host = hosts
            .get(case.get("host").and_then(Value::as_str).expect("host"))
            .expect("host entry");
        let client_capabilities = fixture_capabilities(
            hosts
                .get(case.get("client").and_then(Value::as_str).expect("client"))
                .expect("client entry"),
        );
        let host_capabilities = fixture_capabilities(host);
        let op = case.get("op").and_then(Value::as_str).expect("op");
        let payload = case.get("payload").expect("payload");
        let expected_ids = expected_entry_ids(case);

        let source = Arc::new(CognitionKeSource::new(entries.clone()));
        let memory = Arc::new(CognitionMemoryService::default());
        let (responder, pair) = start_cognition_host(
            Arc::clone(&source),
            Arc::clone(&memory),
            &capability_refs(&host_capabilities),
        )
        .await;
        let client_manifest =
            cognition_manifest("cognition-vector-client", &capability_refs(&client_capabilities));

        let typed_scope_request = if op == COGNITION_SCOPE_OP {
            typed_scope(payload)
        } else {
            None
        };
        if let Some(scope) = typed_scope_request {
            let client = dial(pair, &client_manifest).await;
            let decoded = expect_ok(client.list_knowledge_entries(&scope).await);
            assert_eq!(
                decoded
                    .iter()
                    .map(|entry| entry.entry_id.clone())
                    .collect::<Vec<_>>(),
                expected_ids,
                "{case_id}: client-decoded entry ids"
            );
            assert_eq!(
                *source.returned_entry_ids.lock().expect("ids lock"),
                expected_ids,
                "{case_id}: source selection (control)"
            );
            assert_eq!(source.scope_reads.load(Ordering::SeqCst), 1);
            client.close();
        } else if let Some(request) = typed_memory_list(payload) {
            let client = dial(pair, &client_manifest).await;
            let page = expect_ok(client.list_records(&client_principal(), request).await);
            let page = wire(&page);
            assert_eq!(page["records"], json!([]), "{case_id}: {page}");
            assert_eq!(page["total_authorized"], 0, "{case_id}: {page}");
            assert!(
                page.get("next_cursor").is_none(),
                "{case_id}: an absent cursor is never invented: {page}"
            );
            assert_eq!(memory.invokes.load(Ordering::SeqCst), 1);
            client.close();
        } else {
            // Neither typed client path can forward this payload verbatim
            // (the scope request drops any second top-level member and the
            // generated Memory request type rejects the `scope` member), so
            // the raw public session is the real path. Each received payload
            // decodes with its own op response type — the same decode the
            // client performs.
            raw_only_ids.push(case_id.to_owned());
            let session_id = handshake(&pair.client, &client_manifest).await;
            let response = raw_invoke(
                &pair.client,
                &session_id,
                0,
                case_id,
                op,
                payload.clone(),
            )
            .await;
            let served = response
                .get("payload")
                .unwrap_or_else(|| panic!("{case_id}: expected a served payload, got {response}"));
            if op == COGNITION_SCOPE_OP {
                let decoded: Vec<KnowledgeEntry> = serde_json::from_value(served.clone())
                    .expect("the served entry page decodes");
                assert_eq!(
                    decoded
                        .iter()
                        .map(|entry| entry.entry_id.clone())
                        .collect::<Vec<_>>(),
                    expected_ids,
                    "{case_id}: decoded entry ids"
                );
                assert_eq!(source.scope_reads.load(Ordering::SeqCst), 1);
            } else {
                let decoded: MemoryRecordsListResponse = serde_json::from_value(served.clone())
                    .expect("the served page decodes as its op response");
                match decoded {
                    MemoryRecordsListResponse::Variant0 {
                        records,
                        total_authorized,
                        next_cursor,
                        ..
                    } => {
                        assert!(
                            records.is_empty(),
                            "{case_id}: decoded records {records:?}"
                        );
                        assert_eq!(total_authorized, 0, "{case_id}: decoded count");
                        assert!(
                            next_cursor.is_none(),
                            "{case_id}: an absent cursor is never invented"
                        );
                    }
                    MemoryRecordsListResponse::Variant1 { error, .. } => {
                        panic!("{case_id}: a success decoded to the error branch: {error:?}");
                    }
                }
                assert_eq!(memory.invokes.load(Ordering::SeqCst), 1);
            }
        }
        responder.close();
    }
    raw_only_ids.sort();
    assert_eq!(
        raw_only_ids,
        [
            "memory_scope_cognition_is_not_a_memory_carrier",
            "unrelated_top_level_cognition_is_ordinary_data",
        ],
        "both raw-only positives (Memory `scope` member, top-level `cognition` member) keep the raw public session"
    );
}

/// The decoded client result keeps an absent optional record member absent
/// and an explicit zero at zero — the source's return log is only a control.
#[tokio::test]
async fn cognition_success_records_keep_missing_and_zero_distinct_on_the_client() {
    let fixture: Value = serde_json::from_str(COGNITION_FIXTURE).expect("fixture json");
    let case = fixture
        .get("cases")
        .and_then(Value::as_array)
        .expect("fixture cases")
        .iter()
        .find(|case| {
            case.get("id").and_then(Value::as_str)
                == Some("ke_only_serves_current_selection_with_wildcard_fork")
        })
        .expect("wildcard fork vector");
    let hosts = fixture.get("hosts").expect("fixture hosts");
    let host_capabilities = fixture_capabilities(
        hosts
            .get(case.get("host").and_then(Value::as_str).expect("host"))
            .expect("host entry"),
    );
    let entries = fixture
        .get("ke_entries")
        .and_then(Value::as_array)
        .expect("fixture ke_entries")
        .clone();
    let payload = case.get("payload").expect("payload");
    let scope = typed_scope(payload).expect("the wildcard selection decodes");

    let source = Arc::new(CognitionKeSource::new(entries));
    let memory = Arc::new(CognitionMemoryService::default());
    let (responder, pair) = start_cognition_host(
        Arc::clone(&source),
        memory,
        &capability_refs(&host_capabilities),
    )
    .await;
    let client = dial(
        pair,
        &cognition_manifest(
            "cognition-vector-client",
            &capability_refs(&host_capabilities),
        ),
    )
    .await;
    let decoded = expect_ok(client.list_knowledge_entries(&scope).await);
    let ids: Vec<&str> = decoded
        .iter()
        .map(|entry| entry.entry_id.as_str())
        .collect();
    assert_eq!(ids, vec!["kb_mira_fork", "kb_mira_nofork"]);

    let forked = serde_json::to_value(&decoded[0]).expect("decoded entry serializes");
    let no_fork = serde_json::to_value(&decoded[1]).expect("decoded entry serializes");
    // Distinct assessment contexts are retained side by side.
    assert_eq!(
        forked["modules"]["cognition"]["records"][0]["assessment_context"],
        json!({"context_id": "ctx_main", "fork_id": "fork_alpha"})
    );
    assert_eq!(
        no_fork["modules"]["cognition"]["records"][0]["assessment_context"],
        json!({"context_id": "ctx_main"})
    );
    // Absent optional record data stays absent; an explicit zero stays zero.
    assert!(
        forked["modules"]["cognition"]["records"][0]
            .get("metrics")
            .is_none(),
        "the forked record carries no metrics: {forked}"
    );
    assert_eq!(
        no_fork["modules"]["cognition"]["records"][0]["metrics"]["b"]["value"].as_u64(),
        Some(0),
        "the explicit zero survives the client decode: {no_fork}"
    );
    // Supporting control: the source returned the same two entries.
    assert_eq!(
        *source.returned_entry_ids.lock().expect("ids lock"),
        vec!["kb_mira_fork".to_owned(), "kb_mira_nofork".to_owned()]
    );
    responder.close();
    client.close();
}

/// A `memory.records.create` payload whose Content carries cognition-shaped
/// data is ordinary written data: it is not a cognition requirement, it
/// reaches the service unchanged, and the client decodes the committed
/// receipt.
#[tokio::test]
async fn cognition_memory_create_content_data_is_ordinary_and_decodes_on_the_client() {
    let capabilities = ["spoke-baseline", "spoke-memory", "spoke-cognition"];
    let source = Arc::new(CognitionKeSource::new(Vec::new()));
    let memory = Arc::new(CognitionMemoryService::default());
    let (responder, pair) =
        start_cognition_host(Arc::clone(&source), Arc::clone(&memory), &capabilities).await;
    let client = dial(pair, &cognition_manifest("test-client", &capabilities)).await;
    let request: MemoryRecordsCreateRequest = serde_json::from_value(json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "kind": "fact",
        "content": {
            "text": "Mira records a belief-shaped note",
            "structured": { "cognition": { "records": [] } },
        },
        "scope": { "visibility": "context", "context_id": CONTEXT },
        "idempotency_key": "cognition-create-1",
    }))
    .expect("valid memory.records.create request");
    let decoded: MemoryRecordsCreateResponse =
        expect_ok(client.create_record(&client_principal(), request).await);
    let receipt = wire(&decoded);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["op"], "memory.records.create", "{receipt}");
    assert_eq!(receipt["result"]["record_id"], "rec_cognition_public_proof");
    assert_eq!(receipt["result"]["revision"], 1);
    assert_eq!(memory.invokes.load(Ordering::SeqCst), 1);
    assert_eq!(source.scope_reads.load(Ordering::SeqCst), 0);
    let delivered = memory
        .last_payload
        .lock()
        .expect("payload lock")
        .clone()
        .expect("the service received the payload");
    assert_eq!(
        delivered["content"]["structured"]["cognition"]["records"],
        json!([]),
        "Content data reaches the service unchanged: {delivered}"
    );
    assert_eq!(delivered["idempotency_key"], "cognition-create-1");
    responder.close();
    client.close();
}

/// §6.1/§6.4: an explicitly supplied `"cognition":[]` survives capture,
/// review override and fragment content, and every public read reply — raw
/// wire and typed — while a genuinely absent member stays omitted. The
/// candidate and the override differ in their supplied text and structured
/// fields while both intentionally carry `"cognition":[]`, and each reply
/// must equal the full supplied Content verbatim, so discarding the override
/// or the fragment fails. The SQLite reference provider answers reads by
/// decoding stored JSON into the generated typed response and re-serializing
/// it, so both views are covered.
#[tokio::test]
async fn sqlite_content_cognition_empty_stays_present_across_review_and_reads() {
    let provider = Arc::new(SqliteMemoryProvider::in_memory());
    provider.provision_space(SPACE, "memory-loopback-host", &[]);
    provider.provision_subject(SPACE, SUBJECT, None);
    let principal = client_principal();
    provider.provision_grant(
        SPACE,
        json!({
            "schema_version": 1,
            "grant_id": "grt_loopback_presence_admin",
            "space_id": SPACE,
            "principal_id": principal.principal_id,
            "capabilities": ["admin"],
            "created_at": "2026-07-23T05:36:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    let port: Arc<dyn MemoryPort + Send + Sync> = provider.clone();
    let memory: Arc<dyn RemoteMemoryService> = Arc::new(MemoryPortService::new(port, admission()));

    let (typed_responder, typed_pair) =
        start_host(Some(Arc::clone(&memory)), &MEMORY_CAPABILITIES).await;
    let (raw_responder, raw_pair) = start_host(Some(memory), &MEMORY_CAPABILITIES).await;
    let typed = dial(
        typed_pair,
        &memory_manifest("test-client", &MEMORY_CAPABILITIES),
    )
    .await;
    let raw_session = handshake(
        &raw_pair.client,
        &memory_manifest("test-client", &MEMORY_CAPABILITIES),
    )
    .await;

    // The candidate and the override are distinguishable in their supplied
    // text and structured fields (both intentionally carry `"cognition":[]`),
    // so an implementation that promotes the candidate and discards the
    // override cannot satisfy the retained-record assertions below.
    let candidate_content = json!({
        "text": "Bo described the harbour gate to Mira.",
        "structured": { "origin": "candidate" },
        "cognition": [],
    });
    let absent_content = json!({
        "text": "Bo counted three barrels along the harbour wall.",
    });
    let override_content = json!({
        "text": "Mira told Bo the gate key is hidden under the third barrel.",
        "structured": { "origin": "override" },
        "cognition": [],
    });
    let fragment_content = json!({
        "text": "Mira revealed the key location to Bo.",
        "structured": { "origin": "fragment" },
        "cognition": [],
    });
    let capture = |key: &str, content: &Value| {
        typed_capture(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "content": content.clone(),
            "capture_scope": { "visibility": "context", "context_id": CONTEXT },
            "source_context_ids": [CONTEXT],
            "kind_hint": "fact",
            "idempotency_key": key,
        }))
    };
    let candidate_id = |captured: &spoke_schemas::MemoryCaptureResponse| {
        wire(captured)["result"]["candidate_ids"][0]
            .as_str()
            .expect("candidate id")
            .to_string()
    };

    let override_candidate = candidate_id(&expect_ok(
        typed
            .capture(&principal, capture("presence-cap-override", &candidate_content))
            .await,
    ));
    let absent_candidate = candidate_id(&expect_ok(
        typed
            .capture(&principal, capture("presence-cap-absent", &absent_content))
            .await,
    ));
    let fragment_candidate = candidate_id(&expect_ok(
        typed
            .capture(&principal, capture("presence-cap-fragment", &absent_content))
            .await,
    ));

    // Candidates: the pending read replies keep the candidate Content verbatim
    // (explicit empty cognition included), while an absent member stays omitted.
    let candidate = wire(&expect_ok(
        typed
            .get_candidate(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": override_candidate,
                }))
                .expect("candidate get"),
            )
            .await,
    ));
    assert_eq!(
        candidate["candidate"]["proposed"]["content"], candidate_content,
        "candidates.get keeps the candidate Content verbatim: {candidate}"
    );
    let candidates = wire(&expect_ok(
        typed
            .list_candidates(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                }))
                .expect("candidate list"),
            )
            .await,
    ));
    let find_candidate = |id: &str| {
        candidates["candidates"]
            .as_array()
            .expect("candidates array")
            .iter()
            .find(|candidate| candidate["candidate_id"] == json!(id))
            .unwrap_or_else(|| panic!("candidate {id} is listed: {candidates}"))
            .clone()
    };
    assert_eq!(
        find_candidate(&override_candidate)["proposed"]["content"], candidate_content
    );
    assert_eq!(
        find_candidate(&absent_candidate)["proposed"]["content"], absent_content,
        "candidates.list keeps an absent Content without a cognition member: {candidates}"
    );

    let mut sequence = 0;
    let raw_candidate = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-candidate-get",
        "memory.candidates.get",
        json!({ "space_id": SPACE, "candidate_id": override_candidate }),
    )
    .await;
    sequence += 1;
    assert_eq!(
        raw_candidate["payload"]["candidate"]["proposed"]["content"], candidate_content,
        "raw candidates.get payload keeps the candidate Content verbatim: {raw_candidate}"
    );
    let raw_candidates = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-candidate-list",
        "memory.candidates.list",
        json!({ "space_id": SPACE, "subject_id": SUBJECT }),
    )
    .await;
    sequence += 1;
    let raw_list = raw_candidates["payload"]["candidates"]
        .as_array()
        .expect("raw candidates array");
    let find_raw_candidate = |id: &str| {
        raw_list
            .iter()
            .find(|candidate| candidate["candidate_id"] == json!(id))
            .unwrap_or_else(|| panic!("candidate {id} is listed: {raw_candidates}"))
            .clone()
    };
    assert_eq!(
        find_raw_candidate(&override_candidate)["proposed"]["content"], candidate_content,
        "raw candidates.list keeps the candidate Content verbatim: {raw_candidates}"
    );
    assert_eq!(
        find_raw_candidate(&absent_candidate)["proposed"]["content"], absent_content,
        "raw candidates.list keeps an absent Content without a cognition member: {raw_candidates}"
    );

    // Review: the override replaces the candidate Content and the fragment
    // Content supplies a new one; both must survive verbatim.
    let reviewed = wire(&expect_ok(
        typed
            .review_candidate(
                &principal,
                typed_review(json!({
                    "space_id": SPACE,
                    "candidate_id": override_candidate,
                    "expected_revision": 1,
                    "decision": {
                        "action": "promote",
                        "kind": "fact",
                        "content_override": override_content.clone(),
                    },
                    "idempotency_key": "presence-review-override",
                })),
            )
            .await,
    ));
    let override_record = reviewed["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    let reviewed = wire(&expect_ok(
        typed
            .review_candidate(
                &principal,
                typed_review(json!({
                    "space_id": SPACE,
                    "candidate_id": absent_candidate,
                    "expected_revision": 1,
                    "decision": { "action": "promote", "kind": "fact" },
                    "idempotency_key": "presence-review-absent",
                })),
            )
            .await,
    ));
    let absent_record = reviewed["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    let reviewed = wire(&expect_ok(
        typed
            .review_candidate(
                &principal,
                typed_review(json!({
                    "space_id": SPACE,
                    "candidate_id": fragment_candidate,
                    "expected_revision": 1,
                    "decision": {
                        "action": "fragment",
                        "fragments": [{ "kind": "fact", "content": fragment_content.clone() }],
                    },
                    "idempotency_key": "presence-review-fragment",
                })),
            )
            .await,
    ));
    let fragment_record = reviewed["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();

    // Typed reads: get, list, and recall all retain the explicit empty member.
    let record = wire(&expect_ok(
        typed
            .get_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": override_record,
                }))
                .expect("record get"),
            )
            .await,
    ));
    assert_eq!(
        record["record"]["content"], override_content,
        "records.get returns the supplied override Content verbatim: {record}"
    );
    let record = wire(&expect_ok(
        typed
            .get_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": fragment_record,
                }))
                .expect("fragment record get"),
            )
            .await,
    ));
    assert_eq!(
        record["record"]["content"], fragment_content,
        "records.get returns the supplied fragment Content verbatim: {record}"
    );
    let record = wire(&expect_ok(
        typed
            .get_record(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": absent_record,
                }))
                .expect("absent record get"),
            )
            .await,
    ));
    assert_eq!(
        record["record"]["content"], absent_content,
        "records.get keeps an absent Content without a cognition member: {record}"
    );

    let listed = wire(&expect_ok(
        typed
            .list_records(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "subject_all" },
                }))
                .expect("record list"),
            )
            .await,
    ));
    let find_record = |listed: &Value, id: &str| {
        listed["records"]
            .as_array()
            .expect("records array")
            .iter()
            .find(|record| record["record_id"] == json!(id))
            .unwrap_or_else(|| panic!("record {id} is listed: {listed}"))
            .clone()
    };
    assert_eq!(
        find_record(&listed, &override_record)["content"], override_content,
        "records.list returns the override Content verbatim: {listed}"
    );
    assert_eq!(
        find_record(&listed, &fragment_record)["content"], fragment_content,
        "records.list returns the fragment Content verbatim: {listed}"
    );
    assert_eq!(
        find_record(&listed, &absent_record)["content"], absent_content,
        "records.list keeps an absent Content without a cognition member: {listed}"
    );

    let recalled = wire(&expect_ok(
        typed
            .recall(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "subject_all" },
                    "budget": { "max_items": 8 },
                }))
                .expect("recall request"),
            )
            .await,
    ));
    assert_eq!(
        find_record(&recalled, &override_record)["content"], override_content,
        "recall returns the override Content verbatim: {recalled}"
    );
    assert_eq!(
        find_record(&recalled, &fragment_record)["content"], fragment_content,
        "recall returns the fragment Content verbatim: {recalled}"
    );
    assert_eq!(
        find_record(&recalled, &absent_record)["content"], absent_content,
        "recall keeps an absent Content without a cognition member: {recalled}"
    );

    // Raw reads: the emitted payload carries the same presence, so a lossy
    // client-side decode could not have produced this result.
    let raw_record = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-record-get",
        "memory.records.get",
        json!({ "space_id": SPACE, "record_id": override_record }),
    )
    .await;
    sequence += 1;
    assert_eq!(
        raw_record["payload"]["record"]["content"], override_content,
        "raw records.get payload carries the supplied override Content verbatim: {raw_record}"
    );
    let raw_fragment = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-record-get-fragment",
        "memory.records.get",
        json!({ "space_id": SPACE, "record_id": fragment_record }),
    )
    .await;
    sequence += 1;
    assert_eq!(
        raw_fragment["payload"]["record"]["content"], fragment_content,
        "raw records.get payload carries the supplied fragment Content verbatim: {raw_fragment}"
    );
    let raw_absent = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-record-get-absent",
        "memory.records.get",
        json!({ "space_id": SPACE, "record_id": absent_record }),
    )
    .await;
    sequence += 1;
    assert_eq!(
        raw_absent["payload"]["record"]["content"], absent_content,
        "raw records.get payload keeps an absent Content without a cognition member: {raw_absent}"
    );
    let raw_records = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-record-list",
        "memory.records.list",
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
        }),
    )
    .await;
    sequence += 1;
    assert_eq!(
        find_record(&raw_records["payload"], &override_record)["content"], override_content,
        "raw records.list carries the override Content verbatim: {raw_records}"
    );
    assert_eq!(
        find_record(&raw_records["payload"], &fragment_record)["content"], fragment_content,
        "raw records.list carries the fragment Content verbatim: {raw_records}"
    );
    assert_eq!(
        find_record(&raw_records["payload"], &absent_record)["content"], absent_content,
        "raw records.list keeps an absent Content without a cognition member: {raw_records}"
    );
    let raw_recall = raw_invoke(
        &raw_pair.client,
        &raw_session,
        sequence,
        "presence-raw-recall",
        "memory.recall",
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
            "budget": { "max_items": 8 },
        }),
    )
    .await;
    assert_eq!(
        find_record(&raw_recall["payload"], &override_record)["content"], override_content,
        "raw recall payload carries the override Content verbatim: {raw_recall}"
    );
    assert_eq!(
        find_record(&raw_recall["payload"], &fragment_record)["content"], fragment_content,
        "raw recall payload carries the fragment Content verbatim: {raw_recall}"
    );
    assert_eq!(
        find_record(&raw_recall["payload"], &absent_record)["content"], absent_content,
        "raw recall payload keeps an absent Content without a cognition member: {raw_recall}"
    );

    typed_responder.close();
    raw_responder.close();
    typed.close();
}

/// C9 public proof on the conformant Memory host: the real
/// `SqliteMemoryProvider` behind `MemoryPortService`, a client `RemoteAdapter`
/// and a session that explicitly negotiates `spoke-cognition`. The shared
/// memory-query vectors drive typed list/recall/reflection queries; the exact
/// eligible ids, page continuation, cursor invalidation and reflection basis
/// are asserted through the public client boundary.
#[tokio::test]
async fn cognition_query_vectors_answer_on_the_conformant_memory_session() {
    const C9_CAPABILITIES: [&str; 5] = [
        "spoke-baseline",
        "spoke-memory",
        "spoke-memory-reflection",
        "spoke-memory-transfer",
        "spoke-cognition",
    ];
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cognition/memory-query-vectors.json"
    );
    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(path).expect("memory-query-vectors.json readable"),
    )
    .expect("memory-query-vectors.json parses");

    let provider = Arc::new(SqliteMemoryProvider::in_memory());
    provider.provision_space(SPACE, "memory-loopback-host", &["personality"]);
    provider.provision_subject(SPACE, SUBJECT, None);
    let principal = client_principal();
    provider.provision_grant(
        SPACE,
        json!({
            "schema_version": 1,
            "grant_id": "grt_loopback_query_admin",
            "space_id": SPACE,
            "principal_id": principal.principal_id,
            "capabilities": ["admin"],
            "created_at": "2026-07-23T05:36:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    for record in file["records"].as_array().expect("records") {
        let mut stored = record.clone();
        stored["schema_version"] = json!(1);
        stored["space_id"] = json!(SPACE);
        stored["subject_id"] = json!(SUBJECT);
        stored["scope"] = json!({ "visibility": "shared" });
        stored["provenance"] = json!({
            "source_context_ids": ["ctx_vector"],
            "anchors": [],
            "captured_by": principal.principal_id,
            "captured_at": "2026-07-23T06:00:00Z",
        });
        stored["updated_at"] = stored["created_at"].clone();
        stored["extensions"] = json!({});
        provider.provision_record(SPACE, stored);
    }

    let port: Arc<dyn MemoryPort + Send + Sync> = provider.clone();
    let memory: Arc<dyn RemoteMemoryService> = Arc::new(MemoryPortService::new(port, admission()));
    let (responder, pair) = start_host(Some(memory), &C9_CAPABILITIES).await;
    let client = dial(pair, &memory_manifest("test-client", &C9_CAPABILITIES)).await;

    let page_filter = json!({
        "holder": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "target": { "target_id": "t_prop_1" },
        "q": { "min": 0.5 }
    });
    let other_filter = json!({
        "holder": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "target": { "target_id": "t_prop_1" },
        "category": "belief",
        "q": { "min": 0.5, "max": 1.0 }
    });

    // List: the one eligible source set before counts and pages.
    let listed = expect_ok(
        client
            .list_records(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "shared_only" },
                    "cognition_filter": page_filter.clone(),
                    "limit": 2,
                }))
                .expect("list request"),
            )
            .await,
    );
    let listed = wire(&listed);
    let list_ids: Vec<&str> = listed["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(
        json!(list_ids),
        json!(["rec_cog_alice_high", "rec_cog_forked"]),
        "list page 1"
    );
    assert_eq!(listed["total_authorized"], json!(3));
    let list_cursor = listed["next_cursor"]
        .as_str()
        .expect("list page 1 cursor")
        .to_string();

    // Recall page 1 and its continuation through the same cursor.
    let recalled = expect_ok(
        client
            .recall(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "shared_only" },
                    "cognition_filter": page_filter.clone(),
                    "budget": { "max_items": 2 },
                }))
                .expect("recall request"),
            )
            .await,
    );
    let recalled = wire(&recalled);
    let first_ids: Vec<&str> = recalled["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(
        json!(first_ids),
        json!(["rec_cog_alice_high", "rec_cog_forked"]),
        "recall page 1"
    );
    assert_eq!(recalled["total_authorized"], json!(3));
    let recall_cursor = recalled["next_cursor"]
        .as_str()
        .expect("recall page 1 cursor")
        .to_string();
    let continued = expect_ok(
        client
            .recall(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "shared_only" },
                    "cognition_filter": page_filter.clone(),
                    "budget": { "max_items": 2 },
                    "cursor": recall_cursor,
                }))
                .expect("recall request"),
            )
            .await,
    );
    let continued = wire(&continued);
    let continued_ids: Vec<&str> = continued["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(json!(continued_ids), json!(["rec_cog_full"]));

    // A changed filter invalidates both cursor identities.
    for request in [
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "shared_only" },
            "cognition_filter": other_filter.clone(),
            "budget": { "max_items": 2 },
            "cursor": recall_cursor,
        }),
    ] {
        let reject = expect_reject(
            client
                .recall(
                    &principal,
                    serde_json::from_value(request).expect("recall request"),
                )
                .await,
        );
        assert_eq!(
            reject.code,
            SpokeRejectCode::CursorInvalid,
            "a changed cognition filter must invalidate the cursor: {reject:?}"
        );
    }
    let reject = expect_reject(
        client
            .list_records(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "shared_only" },
                    "cognition_filter": other_filter.clone(),
                    "limit": 2,
                    "cursor": list_cursor,
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(reject.code, SpokeRejectCode::CursorInvalid);

    // Cursor identity carries the operation at the public boundary too: list
    // and recall order differently, so neither cursor may resume the other.
    let reject = expect_reject(
        client
            .list_records(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "shared_only" },
                    "cognition_filter": page_filter.clone(),
                    "limit": 2,
                    "cursor": recall_cursor.clone(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(
        reject.code,
        SpokeRejectCode::CursorInvalid,
        "a recall cursor must not resume a list page"
    );
    let reject = expect_reject(
        client
            .recall(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "shared_only" },
                    "cognition_filter": page_filter.clone(),
                    "budget": { "max_items": 2 },
                    "cursor": list_cursor.clone(),
                }))
                .expect("recall request"),
            )
            .await,
    );
    assert_eq!(
        reject.code,
        SpokeRejectCode::CursorInvalid,
        "a list cursor must not resume a recall page"
    );

    // Reflection: the state digest pins the filter, generation stores it, and
    // the listed reflection keeps the same basis.
    let state = expect_ok(
        client
            .get_reflection_state(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "subject_all" },
                    "cognition_filter": page_filter.clone(),
                }))
                .expect("state request"),
            )
            .await,
    );
    let state = wire(&state);
    assert_eq!(state["basis"]["record_count"], json!(3));
    let digest = state["state_digest"]
        .as_str()
        .expect("state digest")
        .to_string();
    let generated = expect_ok(
        client
            .generate_reflection(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "subject_all" },
                    "cognition_filter": page_filter.clone(),
                    "expected_state_digest": digest.clone(),
                    "idempotency_key": "vec-loopback-generate",
                }))
                .expect("generate request"),
            )
            .await,
    );
    assert_eq!(wire(&generated)["status"], json!("pending"));
    provider.run_worker();
    let reflections = expect_ok(
        client
            .list_reflections(
                &principal,
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "cognition_filter": page_filter.clone(),
                    "limit": 10,
                }))
                .expect("list reflections"),
            )
            .await,
    );
    let reflections = wire(&reflections);
    let pinned = reflections["reflections"]
        .as_array()
        .expect("reflections")
        .first()
        .expect("the generated reflection is listed");
    assert_eq!(pinned["cognition_filter"], page_filter);
    assert_eq!(pinned["basis"]["state_digest"], json!(digest));
    assert_eq!(pinned["basis"]["record_count"], json!(3));

    responder.close();
    client.close();
}
