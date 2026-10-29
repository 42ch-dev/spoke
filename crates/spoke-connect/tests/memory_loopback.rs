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

use std::collections::HashMap;
use std::sync::Arc;

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
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{
    jcs_sha256_hex, BaselinePorts, MemoryPort, SpokeReject, SpokeRejectCode, SpokeResult,
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

async fn handshake(client: &LoopbackTransport, manifest: &HostCapabilityManifest) -> String {
    // A hello nonce is single-use by contract: derive it at runtime so no
    // fixed value can be replayed across runs (and no hard-coded
    // cryptographic value sits in the test source).
    let nonce = format!(
        "memory-loopback-nonce-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after the epoch")
            .as_nanos()
    );
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
