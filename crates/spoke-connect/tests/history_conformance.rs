//! History conformance over Connect — corpus interpreter for the
//! `spoke-history` serving face.
//!
//! Loads the wire vectors from `fixtures/history-conformance/` and drives
//! them against the SHIPPED serving path: `surface: "wire"` cases run over a
//! real `connect_responder` ↔ `RemoteAdapter` loopback (capability gate,
//! responder dispatch, serving decode/admission/scope/dedup and both
//! response-validation boundaries); `surface: "service"` cases run through
//! `HistoryPortService::invoke` directly (requests the typed client cannot
//! emit, like an absent `idempotency_key` or a non-integer revision).
//!
//! The scripted `HistoryPort` is deliberately dumb — it replays the seeded
//! snapshot re-stamped to the reconstructable revision and applies the
//! contract §Checkpoints pruning rule — so every gate, dedup and validation
//! assertion measures connect code, never fixture policy.
//!
//! Two responder-specific rows are appended after the corpus: a
//! wrong-target provider snapshot (transport decode error at the serving
//! boundary) and the serve-budget `OUTCOME_UNCERTAIN` row with recorded
//! recovery on same-key replay.
//!
//! Gated on the `remote-adapter` feature; `cargo test -p spoke-connect`
//! (default features) does not build it.

#![cfg(feature = "remote-adapter")]

#[path = "common/loopback_oracle.rs"]
mod loopback_oracle;
use loopback_oracle::{pubkey_client, pubkey_host, seed_client, seed_host};

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use spoke_connect::core::derive_peer_id_from_ed25519_pubkey;
use spoke_connect::remote::{
    connect_remote_adapter, connect_responder, loopback_transport_pair, ConnectResponder,
    ConnectResponderOptions, HistoryAdmission, HistoryPortService, InMemoryHistoryDedupStore,
    RemoteAdapter, RemoteAdapterOptions, RemoteHistoryService,
    RemoteIdentity, RemoteServePorts, RemoteServePortsComposite,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_operations::{
    jcs_sha256_hex, spoke_ok, spoke_reject, BaselinePorts, HistoryPort, SpokeReject,
    SpokeRejectCode, SpokeResult,
};
use spoke_operations::HistoryPruneResult;
use spoke_schemas::{HostCapabilityManifest, Principal, RevisionSnapshot, Scope};

const HOST_CAPABILITIES: [&str; 2] = ["spoke-baseline", "spoke-history"];
const CLIENT_CAPABILITIES: [&str; 2] = ["spoke-baseline", "spoke-history"];
const ADMITTED_SCOPE: &str = "spc_history";

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/history-conformance")
}

// ── Scripted provider ─────────────────────────────────────────────────────

/// One seeded entity: the retained checkpoint set plus the wire
/// `RevisionSnapshot` the provider replays. The stored snapshot carries the
/// entity at its oldest checkpoint revision; a reconstructable revision
/// re-stamps `revision` and the nested entity revision.
#[derive(Clone)]
struct SeededEntity {
    checkpoints: Vec<u64>,
    /// Retained floor accumulated by prior prunes (0 = nothing truncated).
    floor: u64,
    stored_snapshot: Value,
}

/// Scripted `HistoryPort`: checkpoint/floor semantics from the contract's
/// pruning rule, every refusal answered by the provider itself. Call counts
/// are the observable "did the port run" probe (`provider_calls`).
struct ScriptedHistoryPort {
    entities: Mutex<BTreeMap<(String, String), SeededEntity>>,
    calls: AtomicUsize,
    call_log: Mutex<Vec<String>>,
}

impl ScriptedHistoryPort {
    fn load(entities: &[Value]) -> Arc<Self> {
        let mut seeded = BTreeMap::new();
        for entity in entities {
            let kind = entity["entity_kind"].as_str().expect("entity_kind").to_string();
            let id = entity["entity_id"].as_str().expect("entity_id").to_string();
            let checkpoints = entity["checkpoints"]
                .as_array()
                .map(|cps| {
                    cps.iter()
                        .map(|cp| cp.as_u64().expect("checkpoint revision"))
                        .collect::<Vec<u64>>()
                })
                .unwrap_or_default();
            let stored_snapshot = entity["stored_snapshot"].clone();
            seeded.insert((kind, id), SeededEntity { checkpoints, floor: 0, stored_snapshot });
        }
        Arc::new(Self {
            entities: Mutex::new(seeded),
            calls: AtomicUsize::new(0),
            call_log: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

}

#[async_trait]
impl HistoryPort for ScriptedHistoryPort {
    async fn get_history_snapshot(
        &self,
        _principal: &Principal,
        _scope: &Scope,
        entity_kind: &str,
        entity_id: &str,
        revision: u64,
    ) -> SpokeResult<RevisionSnapshot> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.call_log
            .lock()
            .expect("call log")
            .push(format!("get:{entity_kind}:{entity_id}@{revision}"));
        let mut entities = self.entities.lock().expect("entities lock");
        let Some(entity) = entities.get_mut(&(entity_kind.to_string(), entity_id.to_string())) else {
            // The fixture serves the core kinds only; an unknown kind is
            // HISTORY_ENTITY_UNSUPPORTED even on an unseeded entity id.
            if entity_kind != "knowledge_entry" && entity_kind != "relation" {
                return spoke_reject(
                    SpokeRejectCode::HistoryEntityUnsupported,
                    format!("entity kind {entity_kind} is not served"),
                    Some(serde_json::Map::from_iter([(
                        "entity_kind".into(),
                        Value::String(entity_kind.into()),
                    )])),
                );
            }
            return spoke_reject(SpokeRejectCode::NotFound, "no such entity", None);
        };
        if entity_kind != "knowledge_entry" && entity_kind != "relation" {
            return spoke_reject(
                SpokeRejectCode::HistoryEntityUnsupported,
                format!("entity kind {entity_kind} is not served"),
                Some(serde_json::Map::from_iter([(
                    "entity_kind".into(),
                    Value::String(entity_kind.into()),
                )])),
            );
        }
        let floor = entity.floor_revision();
        if revision < floor {
            return spoke_reject(
                SpokeRejectCode::HistoryTruncated,
                "requested revision is below the retained floor",
                Some(serde_json::Map::from_iter([(
                    "floor_revision".into(),
                    json!(floor),
                )])),
            );
        }
        // Reconstructable from the nearest retained checkpoint at or above
        // the requested revision; above the newest checkpoint is a
        // declared retention gap.
        if entity
            .checkpoints
            .iter()
            .any(|cp| *cp >= revision)
        {
            let mut doc = entity.stored_snapshot.clone();
            doc["revision"] = json!(revision);
            doc["snapshot"]["revision"] = json!(revision);
            drop(entities);
            let snapshot = serde_json::from_value(doc).expect("seeded snapshot decodes");
            spoke_ok(snapshot)
        } else {
            spoke_reject(
                SpokeRejectCode::HistoryNotRetained,
                "revision cannot be reconstructed from retained checkpoints",
                Some(serde_json::Map::from_iter([(
                    "retention_reason".into(),
                    Value::String("retention gap above the newest checkpoint".into()),
                )])),
            )
        }
    }

    async fn prune_history(
        &self,
        _principal: &Principal,
        _scope: &Scope,
        entity_kind: &str,
        entity_id: &str,
        keep: u64,
    ) -> SpokeResult<HistoryPruneResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.call_log
            .lock()
            .expect("call log")
            .push(format!("prune:{entity_kind}:{entity_id}@{keep}"));
        let mut entities = self.entities.lock().expect("entities lock");
        let Some(entity) = entities.get_mut(&(entity_kind.to_string(), entity_id.to_string())) else {
            return spoke_reject(SpokeRejectCode::NotFound, "no such entity", None);
        };
        entity.prune(keep)
    }
}

impl SeededEntity {
    /// The retained floor after prior prunes. The fixture tracks the floor
    /// inside the checkpoint set itself: prunes remove the oldest members,
    /// so the floor is `oldest_survivor - gap` — computed as the highest
    /// revision below the oldest retained checkpoint + 1, i.e. simply the
    /// smallest retained checkpoint when the set started at contiguous
    /// history. For the {1,3,5} vectors the contract's floor is
    /// `newest_deleted + 1`; the fixture records it explicitly.
    fn floor_revision(&self) -> u64 {
        self.floor
    }

    fn prune(&mut self, keep: u64) -> SpokeResult<HistoryPruneResult> {
        let count = self.checkpoints.len() as u64;
        if keep >= count {
            return spoke_ok(HistoryPruneResult {
                deleted: 0,
                floor_revision: None,
            });
        }
        let delete_count = (count - keep) as usize;
        let deleted: Vec<u64> = self.checkpoints.drain(..delete_count).collect();
        let newest_deleted = *deleted.last().expect("non-empty delete set");
        self.floor = newest_deleted + 1;
        spoke_ok(HistoryPruneResult {
            deleted: deleted.len() as u64,
            floor_revision: Some(self.floor),
        })
    }
}

// ── Shared fixtures ───────────────────────────────────────────────────────

fn expect_ok<T>(result: SpokeResult<T>) -> T {
    match result {
        SpokeResult::Ok(value) => value,
        SpokeResult::Reject(reject) => panic!("expected success, got {reject:?}"),
    }
}

fn manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
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

fn principal(peer_id: &str) -> Principal {
    serde_json::from_value(json!({
        "principal_id": format!("prn_{peer_id}"),
        "extensions": {},
    }))
    .expect("valid Principal")
}

fn scope(id: &str) -> Scope {
    serde_json::from_value(json!({ "scope_id": id, "extensions": {} })).expect("valid Scope")
}

fn admission() -> HistoryAdmission {
    Arc::new(|peer_id: &str| {
        spoke_ok((principal(peer_id), scope(ADMITTED_SCOPE)))
    })
}

/// `HistoryPortService` over the scripted port (+ optional bounded dedup).
fn history_service(
    port: Arc<ScriptedHistoryPort>,
    dedup_capacity: Option<usize>,
) -> Arc<HistoryPortService> {
    let service = HistoryPortService::new(port, admission());
    match dedup_capacity {
        Some(capacity) => Arc::new(
            service.with_dedup_store(Arc::new(InMemoryHistoryDedupStore::with_capacity(capacity))),
        ),
        None => Arc::new(service),
    }
}

struct HistoryHost {
    responder: Arc<ConnectResponder>,
    client: Arc<RemoteAdapter>,
}

/// Start a responder serving `history` (or no history face) and dial it.
async fn dial_host(
    service: Option<Arc<dyn RemoteHistoryService>>,
    host_capabilities: &[&str],
    client_capabilities: &[&str],
    serve_budget_ms: Option<u64>,
    client_timeout_ms: Option<u64>,
) -> HistoryHost {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let baseline: Arc<dyn BaselinePorts + Send + Sync> =
        Arc::new(ToyWorldAdapter::with_committed_fixtures());
    let mut ports = RemoteServePortsComposite::new(baseline, None, None);
    if let Some(service) = service {
        ports = ports.with_history(service);
    }
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(ports);
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity { seed: seed_host() },
        manifest: manifest("history-conformance-host", host_capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: serve_budget_ms,
    })
    .await;
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("history-conformance-client", client_capabilities),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: client_timeout_ms,
        capability_token: None,
    })
    .await
    .expect("dial");
    HistoryHost { responder, client }
}

impl HistoryHost {
    fn close(&self) {
        self.client.close();
        self.responder.close();
    }
}

// ── Step execution ────────────────────────────────────────────────────────

/// The observed step outcome in the corpus's normalized shape: get as
/// `{ "snapshot": .. }`, prune as `{ "deleted": .., "floor_revision": .. }`.
enum StepOutcome {
    Value(Value),
    Reject(SpokeReject),
}

/// One step's request fields, normalized.
struct StepRequest {
    op: String,
    entity_kind: String,
    entity_id: String,
    revision: Option<u64>,
    keep: Option<u64>,
    idempotency_key: Option<String>,
    request: Value,
}

fn parse_step(step: &Value, default_op: Option<&str>) -> StepRequest {
    let op = step
        .get("op")
        .and_then(Value::as_str)
        .or(default_op)
        .expect("step op")
        .to_string();
    let request = step["request"].clone();
    StepRequest {
        entity_kind: request["entity_kind"].as_str().expect("entity_kind").to_string(),
        entity_id: request["entity_id"].as_str().expect("entity_id").to_string(),
        revision: request["revision"].as_u64(),
        keep: request["keep"].as_u64(),
        idempotency_key: request["idempotency_key"].as_str().map(str::to_string),
        request,
        op,
    }
}

/// Dispatch a step. For `wire` the typed client is the boundary (get and
/// prune with a caller-supplied or request-carried key); for `service` the
/// raw request value goes through `HistoryPortService::invoke` verbatim.
async fn run_step(
    surface: &str,
    host: Option<&HistoryHost>,
    service: &Arc<dyn RemoteHistoryService>,
    client_peer: &str,
    step: &StepRequest,
) -> StepOutcome {
    if surface == "service" {
        return match service
            .invoke(client_peer, &step.op, step.request.clone())
            .await
        {
            SpokeResult::Ok(value) => StepOutcome::Value(value),
            SpokeResult::Reject(reject) => StepOutcome::Reject(reject),
        };
    }
    let host = host.expect("wire step needs a dialed host");
    let client_principal = principal(client_peer);
    let client_scope = scope(ADMITTED_SCOPE);
    match step.op.as_str() {
        "history.revisions.get" => {
            match HistoryPort::get_history_snapshot(
                host.client.as_ref(),
                &client_principal,
                &client_scope,
                &step.entity_kind,
                &step.entity_id,
                step.revision.expect("get revision"),
            )
            .await
            {
                SpokeResult::Ok(snapshot) => StepOutcome::Value(json!({ "snapshot": snapshot })),
                SpokeResult::Reject(reject) => StepOutcome::Reject(reject),
            }
        }
        "history.revisions.prune" => {
            match host
                .client
                .prune_history(
                    &client_scope,
                    &step.entity_kind,
                    &step.entity_id,
                    step.keep.expect("prune keep"),
                    step.idempotency_key.as_deref(),
                )
                .await
            {
                SpokeResult::Ok(result) => StepOutcome::Value(serde_json::to_value(result).expect("prune result serializes")),
                SpokeResult::Reject(reject) => StepOutcome::Reject(reject),
            }
        }
        other => panic!("unknown history op {other}"),
    }
}

// ── Expect vocabulary ─────────────────────────────────────────────────────

/// `dotted.path[0]` lookup into a `Value`.
fn path_get<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if let Some((name, index)) = segment.split_once('[') {
            let index: usize = index.trim_end_matches(']').parse().expect("array index");
            current = current.get(name)?.get(index)?;
        } else {
            current = current.get(segment)?;
        }
    }
    Some(current)
}

/// Containment: every (nested) member of `expected` equals the
/// corresponding member of `actual`.
fn details_contain(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected
            .iter()
            .all(|(key, want)| actual.get(key).is_some_and(|have| details_contain(have, want))),
        _ => actual == expected,
    }
}

fn reject_code_str(reject: &SpokeReject) -> &str {
    reject.code.as_str()
}

/// Assert one expect object against an observed outcome. `provider_calls`
/// is the cumulative scripted-port call count at this point.
fn check_expect(
    case_id: &str,
    expect: &Value,
    outcome: &StepOutcome,
    provider_calls: usize,
    stored_snapshot: Option<&Value>,
) {
    if let Some(code) = expect.get("error_code").and_then(Value::as_str) {
        let StepOutcome::Reject(reject) = outcome else {
            panic!("case {case_id}: expected reject {code}, got success {outcome:?}");
        };
        match code {
            // Dispatch-layer wire codes arrive client-side mapped to
            // CAPABILITY_PORT_MISSING with details.wire_code preserved.
            "capability_missing" | "op_unsupported" => {
                assert_eq!(
                    reject.code,
                    SpokeRejectCode::CapabilityPortMissing,
                    "case {case_id}: expected CAPABILITY_PORT_MISSING for wire {code}, got {reject:?}"
                );
                let wire_code = reject
                    .details
                    .as_ref()
                    .and_then(|d| d.get("wire_code"))
                    .and_then(Value::as_str);
                assert_eq!(
                    wire_code,
                    Some(code),
                    "case {case_id}: details.wire_code"
                );
            }
            known => {
                assert_eq!(
                    reject_code_str(reject),
                    known,
                    "case {case_id}: reject code"
                );
            }
        }
        if let Some(want) = expect.get("details") {
            let have = reject.details.as_ref().map(|d| Value::Object(d.clone()));
            assert!(
                have.as_ref().is_some_and(|h| details_contain(h, want)),
                "case {case_id}: details {want} not contained in {have:?}"
            );
        }
    } else {
        assert!(
            expect.get("success").and_then(Value::as_bool) == Some(true),
            "case {case_id}: expect must carry error_code or success"
        );
        let StepOutcome::Value(value) = outcome else {
            panic!("case {case_id}: expected success, got {outcome:?}");
        };
        if let Some(state) = expect.get("state").and_then(Value::as_array) {
            for assertion in state {
                let path = assertion["path"].as_str().expect("state path");
                let want = &assertion["equals"];
                let have = path_get(value, path);
                assert_eq!(
                    have,
                    Some(want),
                    "case {case_id}: state.{path} expected {want}, got {have:?}"
                );
            }
        }
        if let Some(present) = expect.get("present").and_then(Value::as_array) {
            for path in present {
                let path = path.as_str().expect("present path");
                assert!(
                    path_get(value, path).is_some(),
                    "case {case_id}: member {path} must be present (explicit-empty survives)"
                );
            }
        }
        if let Some(equal) = expect.get("canonical_equal") {
            let path = equal["path"].as_str().expect("canonical_equal path");
            let member = path_get(value, path)
                .unwrap_or_else(|| panic!("case {case_id}: canonical_equal path {path}"));
            let stored = stored_snapshot.expect("canonical_equal needs a stored_snapshot");
            let left = expect_ok(jcs_sha256_hex(member));
            let right = expect_ok(jcs_sha256_hex(stored));
            assert_eq!(
                left, right,
                "case {case_id}: canonical bytes at {path} must equal the seeded snapshot's"
            );
        }
    }
    if let Some(want) = expect.get("provider_calls").and_then(Value::as_u64) {
        assert_eq!(
            provider_calls, want as usize,
            "case {case_id}: provider call count"
        );
    }
}

impl std::fmt::Debug for StepOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepOutcome::Value(value) => write!(f, "Value({value})"),
            StepOutcome::Reject(reject) => write!(f, "Reject({reject:?})"),
        }
    }
}

// ── Corpus integrity ──────────────────────────────────────────────────────

struct Scenario {
    id: String,
    cases: Vec<Value>,
}

fn load_scenarios() -> Vec<Scenario> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_root())
        .expect("corpus directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()?.to_str()? == "json").then_some(path)
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(&path).expect("read vector"))
                    .expect("json");
            Scenario {
                id: value["id"].as_str().expect("scenario id").to_string(),
                cases: value["cases"].as_array().cloned().unwrap_or_default(),
            }
        })
        .collect()
}

const REQUIRED_SCENARIOS: [&str; 4] = [
    "canonical-snapshot",
    "dedup-replay",
    "prune-floor",
    "refusals",
];

const EXPECT_KEYS: [&str; 7] = [
    "error_code",
    "details",
    "success",
    "state",
    "present",
    "canonical_equal",
    "provider_calls",
];

#[test]
fn corpus_integrity() {
    let scenarios = load_scenarios();
    let ids: Vec<&str> = scenarios.iter().map(|s| s.id.as_str()).collect();
    for required in REQUIRED_SCENARIOS {
        assert!(ids.contains(&required), "scenario {required} missing");
    }
    let mut case_count = 0;
    let mut seen_ids = std::collections::BTreeSet::new();
    for scenario in &scenarios {
        for case in &scenario.cases {
            case_count += 1;
            let id = case["id"].as_str().expect("case id");
            assert!(seen_ids.insert(id), "duplicate case id {id}");
            let surface = case["surface"].as_str().expect("surface");
            assert!(matches!(surface, "wire" | "service"), "case {id} surface");
            let outcome = case["outcome"].as_str().expect("outcome");
            assert_eq!(outcome, "identical", "case {id} outcome");
            assert!(case["invariant"].is_string(), "case {id} invariant");
            assert!(case["what"].is_string(), "case {id} what");
            assert!(case["request"].is_object(), "case {id} request");
            let op = case["op"].as_str().expect("case op");
            assert!(
                matches!(op, "history.revisions.get" | "history.revisions.prune"),
                "case {id} op {op}"
            );
            let expect = &case["expect"];
            assert!(expect.is_object() && !expect.as_object().unwrap().is_empty());
            for key in expect.as_object().unwrap().keys() {
                assert!(
                    EXPECT_KEYS.contains(&key.as_str()),
                    "case {id}: unknown expect key {key}"
                );
            }
            assert!(
                expect.get("error_code").is_some() || expect.get("success").is_some(),
                "case {id}: expect needs error_code or success"
            );
            for step_list in ["setup", "then"] {
                for step in case[step_list].as_array().into_iter().flatten() {
                    let op = step["op"].as_str().expect("step op");
                    assert!(
                        matches!(op, "history.revisions.get" | "history.revisions.prune"),
                        "case {id} step op {op}"
                    );
                    assert!(step["request"].is_object(), "case {id} step request");
                    if let Some(step_expect) = step.get("expect") {
                        for key in step_expect.as_object().unwrap().keys() {
                            assert!(
                                EXPECT_KEYS.contains(&key.as_str()),
                                "case {id}: unknown step expect key {key}"
                            );
                        }
                    }
                }
            }
            for entity in case["input"]["entities"].as_array().into_iter().flatten() {
                assert!(entity["entity_kind"].is_string(), "case {id} entity_kind");
                assert!(entity["entity_id"].is_string(), "case {id} entity_id");
                assert!(entity["checkpoints"].is_array(), "case {id} checkpoints");
                let stored = &entity["stored_snapshot"];
                assert_eq!(
                    stored["entity_kind"], entity["entity_kind"],
                    "case {id}: stored snapshot entity_kind must match"
                );
                assert_eq!(
                    stored["entity_id"], entity["entity_id"],
                    "case {id}: stored snapshot entity_id must match"
                );
            }
        }
    }
    assert_eq!(case_count, 24, "corpus case total drifted");
}

// ── Corpus execution ──────────────────────────────────────────────────────

fn stored_snapshot_for(case: &Value, request: &StepRequest) -> Option<Value> {
    case["input"]["entities"]
        .as_array()?
        .iter()
        .find(|e| {
            e["entity_kind"].as_str() == Some(request.entity_kind.as_str())
                && e["entity_id"].as_str() == Some(request.entity_id.as_str())
        })
        .map(|e| e["stored_snapshot"].clone())
}

#[tokio::test]
async fn history_wire_conformance() {
    let scenarios = load_scenarios();
    let mut failures: Vec<String> = Vec::new();
    for scenario in &scenarios {
        for case in &scenario.cases {
            let case_id = format!("{}/{}", scenario.id, case["id"].as_str().unwrap());
            if let Err(error) = run_case(case).await {
                failures.push(format!("{case_id}: {error}"));
            }
        }
    }
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
}

/// Run one case; a step panic is captured as the case's failure line so one
/// broken vector cannot mask the rest of the corpus.
async fn run_case(case: &Value) -> Result<(), String> {
    use futures::FutureExt;
    let case_id = case["id"].as_str().unwrap_or("?").to_string();
    let surface = case["surface"].as_str().unwrap_or("wire");
    let input = &case["input"];
    let entities = input["entities"].as_array().cloned().unwrap_or_default();
    let port = ScriptedHistoryPort::load(&entities);
    let dedup_capacity = input
        .get("dedup_capacity")
        .and_then(Value::as_u64)
        .map(|c| c as usize);
    let service: Arc<dyn RemoteHistoryService> =
        history_service(Arc::clone(&port), dedup_capacity);
    let history_face = input
        .get("history_face")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let client_caps: Vec<String> = input
        .get("client_capabilities")
        .and_then(Value::as_array)
        .map(|caps| {
            caps.iter()
                .map(|c| c.as_str().expect("capability").to_string())
                .collect()
        })
        .unwrap_or_else(|| CLIENT_CAPABILITIES.iter().map(|c| c.to_string()).collect());
    let client_peer = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    let body = std::panic::AssertUnwindSafe(async {
        // Wire surface: dial a responder whose composite carries the
        // service (or not); service surface talks to the service directly.
        let host = if surface == "wire" {
            let caps: Vec<&str> = client_caps.iter().map(String::as_str).collect();
            Some(
                dial_host(
                    history_face.then(|| Arc::clone(&service)),
                    &HOST_CAPABILITIES,
                    &caps,
                    None,
                    None,
                )
                .await,
            )
        } else {
            None
        };

        let execute = async |step: &StepRequest| -> StepOutcome {
            run_step(surface, host.as_ref(), &service, &client_peer, step).await
        };

        // setup steps: execute, honoring an optional per-step expect.
        if let Some(setup) = case["setup"].as_array() {
            for step in setup {
                let parsed = parse_step(step, None);
                let outcome = execute(&parsed).await;
                if let Some(expect) = step.get("expect") {
                    check_expect(&case_id, expect, &outcome, port.calls(), None);
                } else if let StepOutcome::Reject(reject) = &outcome {
                    panic!("setup step rejected: {reject:?}");
                }
            }
        }

        // The op under test.
        let parsed_op = parse_step(case, None);
        let stored = stored_snapshot_for(case, &parsed_op);
        let outcome = execute(&parsed_op).await;
        check_expect(
            &case_id,
            &case["expect"],
            &outcome,
            port.calls(),
            stored.as_ref(),
        );

        // then steps.
        if let Some(then) = case["then"].as_array() {
            for step in then {
                let parsed = parse_step(step, None);
                let outcome = execute(&parsed).await;
                if let Some(expect) = step.get("expect") {
                    let stored = stored_snapshot_for(case, &parsed);
                    check_expect(&case_id, expect, &outcome, port.calls(), stored.as_ref());
                }
            }
        }

        if let Some(host) = host {
            host.close();
        }
    })
    .catch_unwind()
    .await;
    body.map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "panic".to_string())
    })
}

// ── Responder-specific rows (beyond the corpus) ───────────────────────────

/// The serving boundary rejects a provider snapshot targeting different
/// selectors — the client sees the transport decode error, never a
/// wrong-target `Ok`.
#[tokio::test]
async fn wrong_target_snapshot_is_transport_decode_error() {
    struct WrongTargetPort;
    #[async_trait]
    impl HistoryPort for WrongTargetPort {
        async fn get_history_snapshot(
            &self,
            _principal: &Principal,
            _scope: &Scope,
            _kind: &str,
            _id: &str,
            revision: u64,
        ) -> SpokeResult<RevisionSnapshot> {
            serde_json::from_value(json!({
                "schema_version": 1,
                "entity_kind": "knowledge_entry",
                "entity_id": "ke_OTHER",
                "revision": revision,
                "snapshot": {
                    "schema_version": 1,
                    "entry_id": "ke_OTHER",
                    "entry_type": "note",
                    "canonical_name": "wrong-target",
                    "status": "confirmed",
                    "revision": revision,
                    "body": {},
                    "extensions": {}
                },
                "extensions": {}
            }))
            .map(SpokeResult::Ok)
            .unwrap_or_else(|e| spoke_reject(SpokeRejectCode::InternalError, e.to_string(), None))
        }

        async fn prune_history(
            &self,
            _principal: &Principal,
            _scope: &Scope,
            _kind: &str,
            _id: &str,
            _keep: u64,
        ) -> SpokeResult<HistoryPruneResult> {
            spoke_ok(HistoryPruneResult {
                deleted: 0,
                floor_revision: None,
            })
        }
    }

    let port: Arc<dyn HistoryPort + Send + Sync> = Arc::new(WrongTargetPort);
    let service: Arc<dyn RemoteHistoryService> =
        Arc::new(HistoryPortService::new(port, admission()));
    let host = dial_host(
        Some(service),
        &HOST_CAPABILITIES,
        &CLIENT_CAPABILITIES,
        None,
        None,
    )
    .await;
    let client_peer = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let result = HistoryPort::get_history_snapshot(
        host.client.as_ref(),
        &principal(&client_peer),
        &scope(ADMITTED_SCOPE),
        "knowledge_entry",
        "ke_requested",
        3,
    )
    .await;
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InternalError);
            let kind = reject
                .details
                .as_ref()
                .and_then(|d| d.get("kind"))
                .and_then(Value::as_str);
            assert_eq!(kind, Some("transport"), "wrong-target is a transport decode error");
        }
        SpokeResult::Ok(_) => panic!("wrong-target snapshot must never decode to Ok"),
    }
    host.close();
}

/// §14.2/§14.3: a serve budget that expires AFTER the history service was
/// invoked answers `OUTCOME_UNCERTAIN`/`uncertain` — the detached prune
/// still settles and records, so a same-key replay recovers the outcome.
#[tokio::test]
async fn expired_serve_budget_is_outcome_uncertain_then_replay_recovers() {
    let port = ScriptedHistoryPort::load(&[json!({
        "entity_kind": "knowledge_entry",
        "entity_id": "ke_slow",
        "checkpoints": [1, 3, 5],
        "stored_snapshot": {
            "schema_version": 1,
            "entity_kind": "knowledge_entry",
            "entity_id": "ke_slow",
            "revision": 1,
            "snapshot": {
                "schema_version": 1,
                "entry_id": "ke_slow",
                "entry_type": "note",
                "canonical_name": "slow-fixture",
                "status": "confirmed",
                "revision": 1,
                "body": {},
                "extensions": {}
            },
            "extensions": {}
        }
    })]);

    /// Prune takes longer than the responder's serve budget.
    struct SlowPrunePort {
        inner: Arc<ScriptedHistoryPort>,
    }
    #[async_trait]
    impl HistoryPort for SlowPrunePort {
        async fn get_history_snapshot(
            &self,
            principal: &Principal,
            scope: &Scope,
            kind: &str,
            id: &str,
            revision: u64,
        ) -> SpokeResult<RevisionSnapshot> {
            self.inner
                .get_history_snapshot(principal, scope, kind, id, revision)
                .await
        }
        async fn prune_history(
            &self,
            principal: &Principal,
            scope: &Scope,
            kind: &str,
            id: &str,
            keep: u64,
        ) -> SpokeResult<HistoryPruneResult> {
            tokio::time::sleep(Duration::from_millis(200)).await;
            self.inner
                .prune_history(principal, scope, kind, id, keep)
                .await
        }
    }

    let slow: Arc<dyn HistoryPort + Send + Sync> = Arc::new(SlowPrunePort {
        inner: Arc::clone(&port),
    });
    let service: Arc<dyn RemoteHistoryService> =
        Arc::new(HistoryPortService::new(slow, admission()));
    // Responder serve budget 60ms < the 200ms port call < client budget.
    let host = dial_host(
        Some(service),
        &HOST_CAPABILITIES,
        &CLIENT_CAPABILITIES,
        Some(60),
        Some(5000),
    )
    .await;
    let first = host
        .client
        .prune_history(
            &scope(ADMITTED_SCOPE),
            "knowledge_entry",
            "ke_slow",
            2,
            Some("uncertain-key"),
        )
        .await;
    match first {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::OutcomeUncertain);
            let retry_class = reject
                .details
                .as_ref()
                .and_then(|d| d.get("retry_class"))
                .and_then(Value::as_str);
            assert_eq!(retry_class, Some("uncertain"));
        }
        SpokeResult::Ok(_) => panic!("expired serve budget must answer OUTCOME_UNCERTAIN"),
    }
    // The detached invoke keeps running: let it settle and record, then a
    // same-key replay returns the recorded outcome without re-executing.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let replay = host
        .client
        .prune_history(
            &scope(ADMITTED_SCOPE),
            "knowledge_entry",
            "ke_slow",
            2,
            Some("uncertain-key"),
        )
        .await;
    let result = expect_ok(replay);
    assert_eq!(result.deleted, 1);
    assert_eq!(result.floor_revision, Some(2));
    assert_eq!(port.calls(), 1, "replay must not re-execute the port");
    host.close();
}
