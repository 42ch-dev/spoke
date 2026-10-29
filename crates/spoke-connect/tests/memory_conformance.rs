//! Memory conformance over Connect — corpus interpreter and SQLite-provider run.
//!
//! Loads the wire vectors from `fixtures/memory-conformance/`, drives them over a
//! Connect loopback against the SQLite reference provider, and records one
//! terminal outcome per case (pass / documented inconclusive / fail). The
//! loopback harness provider joins in Task 6.
//!
//! One `invoke(op, payload) -> Value` driver: over Connect an invoke carries JSON,
//! so the interpreter needs no typed per-op dispatch. The expect vocabulary,
//! `$ref` namespaces and directives follow `fixtures/memory-conformance/README.md`.
//!
//! The `stage_from` directive partitions a sealed export asset into §6.13 chunks
//! and stages them in ordinal order; `verify_export` fetches every remaining
//! ordinal and recomputes the advertised chunk/bundle digests. Cooperative-class
//! policy directives (`remap_policy_check`, `pinned_basis_check`,
//! `policy_version_check`) are verified in-case: a violation fails the case,
//! never a silent pass. Cases whose expectations are genuinely unreachable on
//! this provider are recorded as documented inconclusive — never skipped.

#![cfg(feature = "remote-adapter")]

#[path = "common/loopback_oracle.rs"]
mod loopback_oracle;
use loopback_oracle::{connect_manifest, pubkey_client, seed_client, seed_host, sign_envelope};

#[path = "support/sqlite_memory.rs"]
mod sqlite_memory;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use spoke_connect::core::derive_peer_id_from_ed25519_pubkey;
use spoke_connect::remote::{
    connect_responder, loopback_transport_pair, ConnectResponder, ConnectResponderOptions,
    LoopbackTransport, LoopbackTransportPair, MemoryAdmission, MemoryPortService, RemoteIdentity,
    RemoteMemoryService, RemoteServePorts, RemoteServePortsComposite, Transport,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_operations::{MemoryPort, SpokeResult};
use spoke_schemas::{
    ConnectHello, ConnectSession, HostCapabilityManifest, MemoryReflectionsStateRequest,
    Principal,
};
use sqlite_memory::SqliteMemoryProvider;

const HOST_ID: &str = "memory-conformance-host";
const MEMORY_CAPABILITIES: [&str; 4] = [
    "spoke-baseline",
    "spoke-memory",
    "spoke-memory-reflection",
    "spoke-memory-transfer",
];
const SPACE: &str = "spc_tw_harbor";
const TARGET_SPACE: &str = "spc_tw_import_target";
const MIRA: &str = "toy:character:mira";
const BO: &str = "toy:character:bo";
const GATE: &str = "ctx_tw_customs_gate";
const MARKET: &str = "ctx_tw_market_square";

// ── Corpus loading ────────────────────────────────────────────────────────

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/memory-conformance")
}

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

fn load_asset(relative: &str) -> Value {
    let path = corpus_root().join(relative);
    serde_json::from_str(&fs::read_to_string(&path).expect("read asset")).expect("asset json")
}

// ── Loopback plumbing (copied from tests/memory_loopback.rs) ──────────────

type SharedPrincipal = Arc<Mutex<String>>;

fn principal_for(peer_id: &str) -> Principal {
    serde_json::from_value(json!({ "principal_id": format!("prn_{peer_id}"), "extensions": {} }))
        .expect("valid principal")
}

fn memory_face(provider: Arc<SqliteMemoryProvider>) -> Arc<dyn RemoteMemoryService> {
    let port: Arc<dyn MemoryPort + Send + Sync> = provider;
    let admission: MemoryAdmission = Arc::new(move |_peer_id: &str| {
        // The harness sets the corpus principal for the case being executed.
        let principal_id = CURRENT_PRINCIPAL.lock().expect("principal lock").clone();
        match serde_json::from_value::<Principal>(json!({
            "principal_id": principal_id,
            "extensions": {},
        })) {
            Ok(principal) => spoke_operations::SpokeResult::Ok(principal),
            Err(error) => spoke_operations::spoke_reject(
                spoke_operations::SpokeRejectCode::InternalError,
                format!("principal decode: {error}"),
                None,
            ),
        }
    });
    Arc::new(MemoryPortService::new(port, admission))
}

static CURRENT_PRINCIPAL: Mutex<String> = Mutex::new(String::new());

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

async fn start_host(
    memory: Arc<dyn RemoteMemoryService>,
    capabilities: &[&str],
) -> (Arc<ConnectResponder>, LoopbackTransportPair) {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let baseline: Arc<dyn spoke_operations::BaselinePorts + Send + Sync> =
        Arc::new(ToyWorldAdapter::with_committed_fixtures());
    let ports = RemoteServePortsComposite::new(baseline, None, None).with_memory(memory);
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(ports);
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity { seed: seed_host() },
        manifest: manifest(HOST_ID, capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: std::collections::HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: None,
    })
    .await;
    (responder, pair)
}

fn sign_invoke_request(
    seed: [u8; 32],
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let signed = json!({
        "session_id": session_id,
        "sequence": sequence,
        "request_id": request_id,
        "op": op,
        "payload": payload,
    });
    let signature = sign_envelope(&seed, &signed);
    let mut wire = signed.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    Value::Object(wire)
}

async fn handshake(client: &LoopbackTransport, capabilities: &[&str]) -> String {
    let nonce = "memory-conformance-nonce-0".to_owned();
    let hello = sign_hello_ed25519(
        &seed_client(),
        &nonce,
        &connect_manifest(&manifest("conformance-client", capabilities)),
        None,
    )
    .expect("hello sign");
    client
        .send(&serde_json::to_vec(&hello).expect("hello bytes"))
        .await
        .expect("hello send");
    let _responder_hello: ConnectHello =
        serde_json::from_slice(&client.recv().await.expect("hello recv")).expect("hello decode");
    let snapshot = client.recv().await.expect("snapshot recv");
    let session: ConnectSession = serde_json::from_slice(&snapshot).expect("session decode");
    session.session_id.to_string()
}

use spoke_connect::core::sign_hello_ed25519;

/// The provider variant a Wire drives: the conformance matrix runs the same
/// interpreter against both providers, and directives may scope assertions
/// to one leg (`provider_scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Sqlite,
    Loopback,
}

impl ProviderKind {
    fn name(self) -> &'static str {
        match self {
            ProviderKind::Sqlite => "sqlite",
            ProviderKind::Loopback => "loopback",
        }
    }
}

/// The corpus driver: one raw invoke per step.
struct Wire {
    client: LoopbackTransport,
    session_id: String,
    sequence: i64,
    provider: Arc<SqliteMemoryProvider>,
    /// The active provider variant: `provider_scope` predicates compare
    /// against THIS, never against a constant.
    kind: ProviderKind,
}

/// Harness capabilities are local by definition: they drive the provider's
/// snapshot surface, not the wire.
fn run_harness_step(provider: &SqliteMemoryProvider, op: &str) -> Value {
    match op {
        "harness.save_snapshot" => {
            provider.save_snapshot();
            json!({ "success": true })
        }
        "harness.restore_snapshot" => {
            let restored = provider.restore_snapshot();
            json!({ "success": restored })
        }
        other => panic!("unknown harness capability {other}"),
    }
}

impl Wire {
    async fn invoke(&mut self, op: &str, payload: Value) -> Value {
        self.sequence += 1;
        let request = sign_invoke_request(
            seed_client(),
            &self.session_id,
            self.sequence,
            &format!("req-conformance-{}", self.sequence),
            op,
            payload,
        );
        self.client
            .send(&serde_json::to_vec(&request).expect("request bytes"))
            .await
            .expect("request send");
        serde_json::from_slice(&self.client.recv().await.expect("response recv"))
            .expect("response decode")
    }
}

// ── Interpreter helpers ───────────────────────────────────────────────────

fn text(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or_default().to_string()
}

/// Resolve `$ref:setup.<n>.<path>` / `$ref:op.<path>` / `$ref:then.<n>.<path>`.
fn resolve_refs(value: &Value, responses: &BTreeMap<String, Value>) -> Value {
    match value {
        Value::String(text_value) => match text_value.strip_prefix("$ref:") {
            None => value.clone(),
            Some(reference) => {
                let (step, path) = split_reference(reference);
                let response = responses
                    .get(&step)
                    .unwrap_or_else(|| panic!("unknown step ref {step}"));
                let payload = &response["payload"];
                walk(payload, &path)
                    // Polled receipts nest the op payload under `receipt`.
                    .or_else(|| walk(&payload["receipt"], &path))
                    .unwrap_or(Value::Null)
            }
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| resolve_refs(item, responses))
                .collect(),
        ),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), resolve_refs(item, responses)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Split `setup.<n>.<path>` / `op.<path>` / `then.<n>.<path>` into the response
/// key and the path inside that response.
fn split_reference(reference: &str) -> (String, String) {
    if let Some(path) = reference.strip_prefix("op.") {
        return ("op".to_string(), path.to_string());
    }
    let mut parts = reference.splitn(3, '.');
    let namespace = parts.next().unwrap_or_default();
    let index = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    (format!("{namespace}.{index}"), path.to_string())
}

/// Dotted path walk with array indexes (`a.b.0.c`).
fn walk(value: &Value, path: &str) -> Option<Value> {
    let mut current = value.clone();
    for segment in path.split('.') {
        current = match current {
            Value::Object(entries) => entries.get(segment)?.clone(),
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?.clone(),
            _ => return None,
        };
    }
    Some(current)
}

fn has_path(value: &Value, path: &str) -> bool {
    walk(value, path).is_some()
}

// ── Outcome recording ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Pass,
    Inconclusive(String),
    /// A cooperative-class policy violation or expectation mismatch: the case
    /// is proven non-compliant and fails the run.
    Fail(String),
}

struct Report {
    rows: Vec<(String, String, Outcome)>,
}

impl Report {
    fn record(&mut self, scenario: &str, case: &str, outcome: Outcome) {
        self.rows
            .push((scenario.to_string(), case.to_string(), outcome));
    }
}

// ── Seeding ───────────────────────────────────────────────────────────────

fn grant(
    space: &str,
    id: &str,
    principal: &str,
    capabilities: &[&str],
    subject: Option<&str>,
    contexts: Option<&[&str]>,
    shared: Option<bool>,
) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "grant_id": id,
        "space_id": space,
        "principal_id": principal,
        "capabilities": capabilities,
        "created_at": "2026-07-23T05:35:00Z",
        "revision": 1,
        "extensions": {},
    });
    if let Some(subject) = subject {
        value["subject_id"] = json!(subject);
    }
    if let Some(contexts) = contexts {
        value["context_ids"] = json!(contexts);
        value["shared_access"] = json!(shared.unwrap_or(false));
    }
    value
}

fn record(
    space: &str,
    subject: &str,
    id: &str,
    kind: &str,
    text: &str,
    context: Option<&str>,
) -> Value {
    let scope = match context {
        Some(context) => json!({ "visibility": "context", "context_id": context }),
        None => json!({ "visibility": "shared" }),
    };
    json!({
        "schema_version": 1,
        "record_id": id,
        "space_id": space,
        "subject_id": subject,
        "kind": kind,
        "content": { "text": text },
        "scope": scope,
        "provenance": {
            "source_context_ids": [context.unwrap_or("ctx_tw_dawn_arrival")],
            "anchors": [],
            "captured_by": "prn_tw_host_primary",
            "captured_at": "2026-07-23T06:05:00Z",
        },
        "created_at": "2026-07-23T06:06:12Z",
        "updated_at": "2026-07-23T06:06:12Z",
        "revision": 1,
        "extensions": {},
    })
}

async fn seed(provider: &SqliteMemoryProvider) {
    provider.provision_space(SPACE, "host_tw_primary", &["personality"]);
    provider.provision_space(TARGET_SPACE, "host_tw_primary", &["personality"]);
    provider.provision_subject(SPACE, MIRA, Some(("toy", "character")));
    provider.provision_subject(SPACE, BO, Some(("toy", "character")));

    provider.provision_grant(
        SPACE,
        grant(
            SPACE,
            "grt_tw_mira_context_0001",
            "prn_tw_host_primary",
            &[
                "capture",
                "review",
                "records_read",
                "records_write",
                "recall",
            ],
            Some(MIRA),
            Some(&[GATE]),
            Some(true),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            SPACE,
            "grt_tw_operator_admin",
            "prn_tw_cli_operator",
            &["admin"],
            None,
            None,
            None,
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            SPACE,
            "grt_tw_peer_reader",
            "prn_tw_peer_reader",
            &[
                "capture",
                "review",
                "records_read",
                "records_write",
                "recall",
            ],
            Some(MIRA),
            Some(&[GATE]),
            Some(false),
        ),
    );
    let mut expired = grant(
        SPACE,
        "grt_tw_expired_reader",
        "prn_tw_expired_reader",
        &["recall"],
        Some(MIRA),
        Some(&[GATE]),
        Some(true),
    );
    expired["expires_at"] = json!("2026-07-22T00:00:00Z");
    provider.provision_grant(SPACE, expired);
    provider.provision_grant(
        TARGET_SPACE,
        grant(
            TARGET_SPACE,
            "grt_tw_import_admin_0001",
            "prn_tw_cli_operator",
            &["admin"],
            None,
            None,
            None,
        ),
    );

    provider.provision_record(
        SPACE,
        record(
            SPACE,
            MIRA,
            "rec_tw_gate_key_fact",
            "fact",
            "The gate key is hidden under the third barrel.",
            Some(GATE),
        ),
    );
    provider.provision_record(
        SPACE,
        record(
            SPACE,
            MIRA,
            "rec_tw_gate_key_event",
            "event",
            "Mira revealed the key location to Bo.",
            Some(GATE),
        ),
    );
    provider.provision_record(
        SPACE,
        record(
            SPACE,
            MIRA,
            "rec_tw_bo_promise",
            "event",
            "Mira promised the harbor master she would return before dawn.",
            None,
        ),
    );

    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": MIRA,
            "profile_kind": "personality",
            "document": { "format": "markdown", "body": "# Mira\n\nGuarded, loyal, keeps promises.\n" },
            "updated_by": "prn_tw_cli_operator",
            "created_at": "2026-07-23T05:50:00Z",
            "updated_at": "2026-07-23T05:50:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );

    // The source-space control reflection: pinned to the provider's own
    // current basis (via `memory.reflections.state`), so it reads `stale:
    // false` until its basis moves — the imported-staleness case's control
    // assertion requires exactly that.
    let operator: Principal = serde_json::from_value(json!({
        "principal_id": "prn_tw_cli_operator",
        "extensions": {},
    }))
    .expect("valid principal");
    let state_request: MemoryReflectionsStateRequest = serde_json::from_value(json!({
        "space_id": SPACE,
        "subject_id": MIRA,
        "selection": {
            "mode": "selected_contexts",
            "context_ids": [GATE],
            "include_shared": true,
        },
    }))
    .expect("valid reflections.state request");
    let state = match provider.get_reflection_state(&operator, state_request).await {
        SpokeResult::Ok(response) => serde_json::to_value(response).expect("state response"),
        SpokeResult::Reject(reject) => panic!("seed reflections.state: {reject:?}"),
    };
    let mut control = load_asset("assets/tw-minimal-export.json")["reflections"][0].clone();
    control["basis"]["state_digest"] = state["state_digest"].clone();
    provider.provision_reflection(SPACE, control);
}

#[test]
fn corpus_interpreter_runs_against_the_sqlite_provider() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let mut report = Report { rows: Vec::new() };
    runtime.block_on(async {
        // One provider (and one session) per case: the corpus cases assume a
        // seeded, unfrozen space, so they must not inherit each other's state.
        for scenario in load_scenarios() {
            for case in scenario
                .cases
                .iter()
                .filter(|case| case["surface"] == "wire")
            {
                let provider = Arc::new(SqliteMemoryProvider::in_memory());
                seed(&provider).await;
                let (responder, pair) =
                    start_host(memory_face(Arc::clone(&provider)), &MEMORY_CAPABILITIES).await;
                let client = pair.client.clone();
                let session_id = handshake(&client, &MEMORY_CAPABILITIES).await;
                let mut wire = Wire {
                    client,
                    session_id,
                    sequence: -1,
                    provider: Arc::clone(&provider),
                    kind: ProviderKind::Sqlite,
                };
                // `capability_missing` is unreachable on a fully-negotiated
                // session: a case that expects it runs its op on a second
                // session negotiated without `spoke-memory-reflection`.
                let mut reduced = None;
                let mut reduced_responder = None;
                if expects_capability_missing(case) {
                    let caps: Vec<&str> = MEMORY_CAPABILITIES
                        .iter()
                        .copied()
                        .filter(|cap| *cap != "spoke-memory-reflection")
                        .collect();
                    let (responder, pair) =
                        start_host(memory_face(Arc::clone(&provider)), &caps).await;
                    let client = pair.client.clone();
                    let session_id = handshake(&client, &caps).await;
                    reduced = Some(Wire {
                        client,
                        session_id,
                        sequence: -1,
                        provider: Arc::clone(&provider),
                        kind: ProviderKind::Sqlite,
                    });
                    reduced_responder = Some(responder);
                }
                let outcome = run_case(&mut wire, reduced.as_mut(), &scenario.id, case).await;
                report.record(
                    &scenario.id,
                    case["id"].as_str().unwrap_or("<case>"),
                    outcome,
                );
                responder.close();
                if let Some(responder) = reduced_responder {
                    responder.close();
                }
            }
        }
    });

    let passed = report
        .rows
        .iter()
        .filter(|(_, _, outcome)| *outcome == Outcome::Pass)
        .count();
    println!(
        "conformance: {} cases, {passed} pass, {} not passing",
        report.rows.len(),
        report.rows.len() - passed
    );
    for (scenario, case, outcome) in &report.rows {
        match outcome {
            Outcome::Inconclusive(reason) => println!("INCONCLUSIVE {scenario}/{case}: {reason}"),
            Outcome::Fail(reason) => println!("FAIL {scenario}/{case}: {reason}"),
            Outcome::Pass => {}
        }
    }
    assert_eq!(report.rows.len(), 44, "all wire cases are executed");
    let failed = report
        .rows
        .iter()
        .filter(|(_, _, outcome)| matches!(outcome, Outcome::Fail(_)))
        .count();
    assert_eq!(failed, 0, "cooperative-class policy violations fail their case");
}

/// Execute one case: setup → op → then, with the closed expect vocabulary.
async fn run_case(
    wire: &mut Wire,
    mut reduced: Option<&mut Wire>,
    scenario: &str,
    case: &Value,
) -> Outcome {
    let case_id = case["id"].as_str().unwrap_or("<case>");
    *CURRENT_PRINCIPAL.lock().expect("principal lock") = case["principal"]
        .as_str()
        .unwrap_or("prn_tw_host_primary")
        .to_string();
    let mut responses: BTreeMap<String, Value> = BTreeMap::new();

    for (index, step) in case["setup"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let outcome = match invoke_step(wire, step, &responses).await {
            Ok(value) => value,
            Err(failure) => {
                return Outcome::Inconclusive(format!(
                    "{scenario}/{case_id}: setup[{index}] {failure}"
                ))
            }
        };
        responses.insert(format!("setup.{index}"), outcome.clone());
        if let Some(expect) = step.get("expect") {
            if let Err(failure) = check_expect(&outcome, expect, &responses) {
                return Outcome::Inconclusive(format!(
                    "{scenario}/{case_id}: setup[{index}] {failure}"
                ));
            }
        } else if outcome.get("error").is_some() {
            return Outcome::Inconclusive(format!(
                "{scenario}/{case_id}: setup[{index}] unexpected error {outcome}"
            ));
        }
    }

    if let Some(advance) = case.get("advance") {
        let mut instants: Vec<i64> = Vec::new();
        for instant in advance["instants"].as_array().cloned().unwrap_or_default() {
            let step = instant["from"].as_str().unwrap_or_default();
            let path = instant["path"].as_str().unwrap_or_default();
            let offset = instant["offset_seconds"].as_i64().unwrap_or(0);
            let base = responses
                .get(step)
                .and_then(|value| walk(&value["payload"], path))
                .and_then(|value| value.as_i64());
            match base {
                Some(base) => instants.push(base + offset),
                None => {
                    return Outcome::Inconclusive(format!(
                        "{scenario}/{case_id}: advance instant {step}.{path} unreadable"
                    ))
                }
            }
        }
        if instants.is_empty() {
            return Outcome::Inconclusive(format!("{scenario}/{case_id}: empty advance"));
        }
        let chosen = match advance["choose"].as_str() {
            Some("min") => *instants.iter().min().expect("non-empty"),
            _ => instants[0],
        };
        wire.provider.advance_clock(chosen);
    }

    // `release`: complete the held setup receipts (poll them to terminal).
    for reference in case["release"].as_array().cloned().unwrap_or_default() {
        let step = reference.as_str().unwrap_or_default().to_string();
        let receipt_id = responses
            .get(&step)
            .map(|value| value["payload"]["receipt_id"].clone())
            .unwrap_or(Value::Null);
        let _ = wire
            .invoke(
                "memory.receipts.get",
                json!({ "space_id": SPACE, "receipt_id": receipt_id }),
            )
            .await;
    }

    let op_request = resolve_refs(&case["request"], &responses);
    let op = case["op"].as_str().expect("op");
    let mut op_response = if op == "memory.imports.stage" && case.get("stage_from").is_some() {
        // The staging directive also appears on the op-under-test itself.
        let mut op_step = serde_json::Map::new();
        op_step.insert("op".into(), json!(op));
        op_step.insert("request".into(), op_request.clone());
        op_step.insert("stage_from".into(), case["stage_from"].clone());
        match stage_from_asset(wire, &Value::Object(op_step), &responses).await {
            Ok(response) => response,
            Err(failure) => {
                return Outcome::Inconclusive(format!("{scenario}/{case_id}: {failure}"))
            }
        }
    } else {
        match reduced.as_deref_mut() {
            Some(reduced_wire) => reduced_wire.invoke(op, op_request.clone()).await,
            None => wire.invoke(op, op_request.clone()).await,
        }
    };
    if let Some(directive) = case.get("verify_export") {
        match verify_export_fetches(wire, wire.kind, &directive, &responses, &op_request, &op_response).await
        {
            Ok(verified) => op_response["payload"]["verified"] = verified,
            Err(failure) => {
                return Outcome::Inconclusive(format!("{scenario}/{case_id}: {failure}"))
            }
        }
    }
    responses.insert("op".to_string(), op_response.clone());
    if let Err(failure) = check_expect(&op_response, &case["expect"], &responses) {
        return Outcome::Inconclusive(format!("{scenario}/{case_id}: {failure}"));
    }
    if let Some(outcome) = check_policy_directives(wire, case, &responses, &op_request).await {
        return outcome;
    }

    for (index, step) in case["then"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let response = match invoke_step(wire, step, &responses).await {
            Ok(value) => value,
            Err(failure) => {
                return Outcome::Inconclusive(format!(
                    "{scenario}/{case_id}: then[{index}] {failure}"
                ))
            }
        };
        responses.insert(format!("then.{index}"), response.clone());
        if let Some(expect) = step.get("expect") {
            if let Err(failure) = check_expect(&response, expect, &responses) {
                // A then step's expect is an assertion, not a probe: a
                // differing observable FAILS the case (for example the
                // recall-page stability repeat).
                return Outcome::Fail(format!("{scenario}/{case_id}: then[{index}] {failure}"));
            }
        }
    }
    Outcome::Pass
}

/// A case expecting `capability_missing` must run on a session negotiated
/// without the reflection sub-capability — the deny is unreachable on the
/// fully-negotiated session.
fn expects_capability_missing(case: &Value) -> bool {
    case["expect"]
        .get("error_code")
        .and_then(Value::as_str)
        .map(|code| code == "capability_missing")
        .unwrap_or(false)
}

/// Invoke one case step. Harness capabilities drive the provider's snapshot
/// surface; a `memory.imports.stage` step carrying `stage_from` partitions the
/// asset and stages every chunk; anything else is one raw invoke.
async fn invoke_step(
    wire: &mut Wire,
    step: &Value,
    responses: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    let op = step["op"].as_str().expect("step op");
    if op.starts_with("harness.") {
        return Ok(run_harness_step(&wire.provider, op));
    }
    if op == "memory.imports.stage" && step.get("stage_from").is_some() {
        return stage_from_asset(wire, step, responses).await;
    }
    let request = resolve_refs(&step["request"], responses);
    Ok(wire.invoke(op, request).await)
}

/// `stage_from` directive: partition the sealed asset into §6.13 chunks of at
/// most `chunk_limit` bytes and stage them in ordinal order over the wire,
/// expanding `idempotency_key_pattern` per chunk. `replay.chunk` re-sends that
/// chunk with the identical key and asserts `result.chunks_received` does not
/// double-count. The step's response is the final chunk's receipt.
async fn stage_from_asset(
    wire: &mut Wire,
    step: &Value,
    responses: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    let directive = &step["stage_from"];
    let asset = directive["asset"].as_str().unwrap_or_default();
    let bundle = load_asset(asset);
    let chunk_limit = directive["chunk_limit"].as_u64().unwrap_or(4096) as usize;
    let pattern = directive["idempotency_key_pattern"]
        .as_str()
        .expect("stage_from idempotency_key_pattern");
    let export_id = text(&bundle, "export_id");
    let chunks = spoke_memory::transfer::partition_bundle(&bundle, chunk_limit)
        .map_err(|error| format!("partition {asset}: {error}"))?;
    let template = resolve_refs(&step["request"], responses);
    let replay_chunk = directive["replay"]["chunk"].as_u64();
    let mut last = Value::Null;
    for chunk in &chunks {
        let ordinal = chunk["chunk_ordinal"].as_u64().unwrap_or(0);
        let mut request = template.clone();
        request["chunk"] = chunk.clone();
        request["idempotency_key"] = json!(pattern
            .replace("{export_id}", &export_id)
            .replace("{ordinal}", &ordinal.to_string()));
        if ordinal > 0 {
            // §7.31: tail chunks carry no op-level conflict_policy/adopt_grants.
            let object = request.as_object_mut().expect("stage request object");
            object.remove("conflict_policy");
            object.remove("adopt_grants");
        }
        let response = wire.invoke("memory.imports.stage", request.clone()).await;
        if let Some(error) = response["payload"].get("error").or_else(|| response.get("error")) {
            return Err(format!("stage chunk {ordinal}: {error}"));
        }
        if replay_chunk == Some(ordinal) {
            let replayed = wire.invoke("memory.imports.stage", request).await;
            let first = response["payload"]["result"]["chunks_received"].clone();
            let second = replayed["payload"]["result"]["chunks_received"].clone();
            if first != second {
                return Err(format!(
                    "replay chunk {ordinal} double-counted: {first} then {second}"
                ));
            }
        }
        last = response;
    }
    Ok(last)
}

/// `verify_export` directive: the op response carries chunk ordinal 0; fetch
/// every remaining ordinal from the referenced receipt's `result.export_id` /
/// `result.chunk_count`, then recompute each `chunk_digest` (JCS of the chunk
/// minus `chunk_digest`) and the reassembled bundle digest (JCS minus
/// `digest`). Fabricated digests fail the case.
async fn verify_export_fetches(
    wire: &mut Wire,
    kind: ProviderKind,
    directive: &Value,
    responses: &BTreeMap<String, Value>,
    request: &Value,
    op_response: &Value,
) -> Result<Value, String> {
    let reference = directive["receipt"].as_str().unwrap_or_default();
    let receipt_response = responses
        .get(reference)
        .ok_or_else(|| format!("verify_export: unknown receipt step {reference}"))?;
    let payload = &receipt_response["payload"];
    let lookup = |path: &str| walk(payload, path).or_else(|| walk(&payload["receipt"], path));
    let export_id = lookup("result.export_id")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or("verify_export: receipt result.export_id unreadable")?;
    let chunk_count = lookup("result.chunk_count")
        .and_then(|value| value.as_u64())
        .ok_or("verify_export: receipt result.chunk_count unreadable")? as usize;
    let advertised = lookup("result.bundle_digest")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or("verify_export: receipt result.bundle_digest unreadable")?;
    let mut chunks = vec![op_response["payload"]["chunk"].clone()];
    for ordinal in 1..chunk_count {
        let response = wire
            .invoke(
                "memory.exports.get",
                json!({
                    "space_id": request["space_id"],
                    "export_id": export_id,
                    "chunk_ordinal": ordinal,
                }),
            )
            .await;
        if let Some(error) = response["payload"].get("error").or_else(|| response.get("error")) {
            return Err(format!("fetch chunk {ordinal}: {error}"));
        }
        chunks.push(response["payload"]["chunk"].clone());
    }
    let all_chunk_digests = chunks.iter().all(|chunk| {
        spoke_memory::transfer::chunk_digest(chunk)
            .map(|digest| Some(digest.as_str()) == chunk["chunk_digest"].as_str())
            .unwrap_or(false)
    });
    let metadata = |field: &str| chunks[0][field].clone();
    let chunk_metadata_consistent = chunks.iter().enumerate().all(|(index, chunk)| {
        chunk["chunk_ordinal"].as_u64() == Some(index as u64)
            && chunk["export_id"] == metadata("export_id")
            && chunk["chunk_count"] == metadata("chunk_count")
            && chunk["bundle_digest"] == metadata("bundle_digest")
    });
    let reassembled = spoke_memory::transfer::reassemble(&chunks)
        .map_err(|error| format!("reassemble: {error}"))?;
    let recomputed = spoke_memory::transfer::bundle_digest(&reassembled)
        .map_err(|error| format!("bundle digest: {error}"))?;
    // The vector may pin an independently generated digest constant (Python
    // JCS+sha256 of the deterministic sealed bundle). `provider_scope`
    // restricts the assertion to one provider leg, compared against the
    // ACTIVE provider variant — a differently scoped leg bypasses it
    // (documented skip), never an Inconclusive. A mismatch on the active
    // scoped leg is a hard failure.
    if foreign_constant_applies(directive, kind) {
        let foreign = directive["foreign_bundle_digest"]
            .as_str()
            .expect("foreign_bundle_digest checked by foreign_constant_applies");
        if recomputed != foreign {
            return Err(format!(
                "foreign bundle digest mismatch: recomputed {recomputed}, pinned {foreign}"
            ));
        }
    }
    let bundle_digest_matches_receipt = recomputed == advertised;
    let reassembled_bundle_verifies =
        reassembled.get("digest").and_then(Value::as_str) == Some(recomputed.as_str());
    Ok(json!({
        "all_chunk_digests": all_chunk_digests,
        "chunk_metadata_consistent": chunk_metadata_consistent,
        "bundle_digest_matches_receipt": bundle_digest_matches_receipt,
        "reassembled_bundle_verifies": reassembled_bundle_verifies,
    }))
}

/// Resolve a `<step>.<dotted.path>` reference against the recorded responses'
/// payloads (polled receipts nest the op payload under `receipt`).
fn reference_value(reference: &Value, responses: &BTreeMap<String, Value>) -> Value {
    let reference = reference.as_str().unwrap_or_default();
    let (step, path) = split_reference(reference);
    responses
        .get(&step)
        .and_then(|response| {
            walk(&response["payload"], &path)
                .or_else(|| walk(&response["payload"]["receipt"], &path))
        })
        .unwrap_or(Value::Null)
}

/// Cooperative-class policy directives declared on the case. A violation
/// returns `Some(Outcome::Fail)` (or `Inconclusive` when the case declares
/// `inconclusive_on_*: true` for a third-party provider).
async fn check_policy_directives(
    wire: &mut Wire,
    case: &Value,
    responses: &BTreeMap<String, Value>,
    op_request: &Value,
) -> Option<Outcome> {
    let case_id = case["id"].as_str().unwrap_or("<case>");
    let verdict = |inconclusive: bool, reason: String| {
        if inconclusive {
            Outcome::Inconclusive(reason)
        } else {
            Outcome::Fail(reason)
        }
    };
    if let Some(directive) = case.get("remap_policy_check") {
        if let Err(violation) = check_remap_policy(&directive, responses) {
            let inconclusive = directive["inconclusive_on_violation"].as_bool() == Some(true);
            return Some(verdict(
                inconclusive,
                format!("{case_id}: remap policy: {violation}"),
            ));
        }
    }
    if let Some(directive) = case.get("policy_version_check") {
        let observed = reference_value(&directive["observed"], responses);
        let pinned = reference_value(&directive["pinned"], responses);
        if observed != pinned {
            let inconclusive = directive["inconclusive_on_mismatch"].as_bool() == Some(true);
            return Some(verdict(
                inconclusive,
                format!("{case_id}: policy version mismatch: observed {observed}, pinned {pinned}"),
            ));
        }
    }
    if let Some(directive) = case.get("pinned_basis_check") {
        let reflection = reference_value(&directive["reflection"], responses);
        let mut request = json!({
            "space_id": op_request["space_id"],
            "subject_id": reflection["subject_id"],
            "selection": reflection["selection"],
        });
        if let Some(filter) = reflection.get("filter").filter(|value| !value.is_null()) {
            request["filter"] = filter.clone();
        }
        let state = wire
            .invoke("memory.reflections.state", request)
            .await;
        let recomputed = state["payload"]["state_digest"].clone();
        let pinned = reflection["basis"]["state_digest"].clone();
        if recomputed != pinned {
            let inconclusive = directive["inconclusive_on_mismatch"].as_bool() == Some(true);
            return Some(verdict(
                inconclusive,
                format!("{case_id}: pinned basis mismatch: recomputed {recomputed}, pinned {pinned}"),
            ));
        }
    }
    None
}

/// `remap_policy_check`: every named translation map holds exactly the declared
/// source ids, each mapped to itself (§7.31 reassigns, never renames).
fn check_remap_policy(
    directive: &Value,
    responses: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let report = reference_value(&directive["report"], responses);
    for map_name in directive["maps"].as_array().cloned().unwrap_or_default() {
        let map_name = map_name.as_str().unwrap_or_default();
        let expected: BTreeSet<&str> = directive["expect_keys"][map_name]
            .as_array()
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let actual = report[map_name]
            .as_object()
            .ok_or_else(|| format!("{map_name}: missing from the staged report"))?;
        let actual_keys: BTreeSet<&str> = actual.keys().map(String::as_str).collect();
        if actual_keys != expected {
            return Err(format!(
                "{map_name}: keys {actual_keys:?} != declared {expected:?}"
            ));
        }
        for (key, value) in actual {
            if value.as_str() != Some(key) {
                return Err(format!("{map_name}: {key} maps to {value}, not itself"));
            }
        }
    }
    Ok(())
}

/// Whether the `foreign_bundle_digest` assertion applies on the active
/// provider leg: the directive must pin a constant and either declare no
/// `provider_scope` or scope it to the ACTIVE variant (`wire.kind`) — never
/// a constant string.
fn foreign_constant_applies(directive: &Value, kind: ProviderKind) -> bool {
    directive["foreign_bundle_digest"].is_string()
        && directive["provider_scope"]
            .as_str()
            .map(|scope| scope == kind.name())
            .unwrap_or(true)
}

#[test]
fn foreign_constant_scopes_to_the_active_provider_leg() {
    // QC1-005 regression: the scoped assertion runs on the sqlite leg and is
    // bypassed (documented skip, not Inconclusive) on the loopback leg;
    // ordinary export verification stays active everywhere.
    let scoped = json!({
        "receipt": "setup.1",
        "foreign_bundle_digest": "ab".repeat(32),
        "provider_scope": "sqlite",
    });
    assert!(foreign_constant_applies(&scoped, ProviderKind::Sqlite));
    assert!(
        !foreign_constant_applies(&scoped, ProviderKind::Loopback),
        "sqlite-scoped oracle must be bypassed on the loopback leg"
    );

    let unscoped = json!({
        "receipt": "setup.1",
        "foreign_bundle_digest": "ab".repeat(32),
    });
    assert!(foreign_constant_applies(&unscoped, ProviderKind::Sqlite));
    assert!(foreign_constant_applies(&unscoped, ProviderKind::Loopback));

    let plain = json!({ "receipt": "setup.1" });
    assert!(!foreign_constant_applies(&plain, ProviderKind::Sqlite));
    assert!(!foreign_constant_applies(&plain, ProviderKind::Loopback));
}

/// Evaluate the closed expect vocabulary against a response.
fn check_expect(
    response: &Value,
    expect: &Value,
    responses: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let payload = &response["payload"];
    let body = if payload.is_null() { response } else { payload };
    let failed = |message: String| Err(message);

    if let Some(code) = expect.get("error_code").and_then(Value::as_str) {
        let actual = body["error"]["code"].as_str().unwrap_or_default();
        if actual != code {
            return failed(format!("expected error {code}, got {body}"));
        }
    }
    if expect["success"].as_bool() == Some(true) && body.get("error").is_some() {
        return failed(format!("expected success, got {body}"));
    }
    if let Some(verified) = expect.get("verified") {
        // Harness-computed booleans (the `verify_export` directive injects
        // them into the op payload before this check).
        if body.get("verified") != Some(verified) {
            return failed(format!(
                "verified: expected {verified}, got {:?}",
                body.get("verified")
            ));
        }
    }
    if let Some(status) = expect.get("receipt_status").and_then(Value::as_str) {
        let receipt = if body.get("receipt").is_some() {
            &body["receipt"]
        } else {
            body
        };
        if receipt["status"].as_str() != Some(status) {
            return failed(format!("expected receipt status {status}, got {body}"));
        }
    }
    if let Some(target) = expect.get("same_receipt_as").and_then(Value::as_str) {
        let receipt = if body.get("receipt").is_some() {
            &body["receipt"]
        } else {
            body
        };
        let original = responses
            .get(target)
            .map(|value| value["payload"]["receipt_id"].clone())
            .unwrap_or(Value::Null);
        if receipt["receipt_id"] != original {
            return failed(format!("expected receipt {original}, got {body}"));
        }
    }
    if let Some(details) = expect.get("details").and_then(Value::as_object) {
        let actual = &body["error"]["details"];
        for (key, expected) in details {
            if actual.get(key) != Some(expected) {
                return failed(format!("details.{key} mismatch: {body}"));
            }
        }
    }
    for entry in expect
        .get("state")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        match walk(body, path) {
            Some(actual) if actual == entry["equals"] => {}
            other => {
                return failed(format!(
                    "state {path}: expected {}, got {other:?}",
                    entry["equals"]
                ))
            }
        }
    }
    for entry in expect
        .get("sizes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let actual = walk(body, path).and_then(|value| value.as_array().map(Vec::len));
        if actual != entry["equals"].as_u64().map(|value| value as usize) {
            return failed(format!(
                "sizes {path}: expected {}, got {actual:?}",
                entry["equals"]
            ));
        }
    }
    for entry in expect
        .get("minimums")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let actual = walk(body, path).and_then(|value| value.as_array().map(Vec::len));
        let minimum = entry["equals"].as_u64().unwrap_or(0) as usize;
        if actual.map(|len| len < minimum).unwrap_or(true) {
            return failed(format!(
                "minimums {path}: expected >= {minimum}, got {actual:?}"
            ));
        }
    }
    for entry in expect
        .get("atLeast")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let actual = walk(body, path).and_then(|value| value.as_f64());
        let minimum = entry["equals"].as_f64().unwrap_or(0.0);
        if actual.map(|value| value < minimum).unwrap_or(true) {
            return failed(format!(
                "atLeast {path}: expected >= {minimum}, got {actual:?}"
            ));
        }
    }
    for entry in expect
        .get("absent")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry.as_str().unwrap_or_default();
        if has_path(body, path) {
            return failed(format!("absent {path}: present in {body}"));
        }
    }
    for entry in expect
        .get("present")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry.as_str().unwrap_or_default();
        if !has_path(body, path) {
            return failed(format!("present {path}: missing in {body}"));
        }
    }
    for entry in expect
        .get("contains")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let items = walk(body, path)
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let matched = items.iter().any(|item| {
            entry["match"]
                .as_object()
                .map(|expected| {
                    expected
                        .iter()
                        .all(|(key, value)| item.get(key) == Some(value))
                })
                .unwrap_or(false)
        });
        if !matched {
            return failed(format!(
                "contains {path}: no element matches {}",
                entry["match"]
            ));
        }
    }
    for entry in expect
        .get("excludes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let items = walk(body, path)
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let matched = items.iter().any(|item| {
            entry["match"]
                .as_object()
                .map(|expected| {
                    expected
                        .iter()
                        .all(|(key, value)| item.get(key) == Some(value))
                })
                .unwrap_or(false)
        });
        if matched {
            return failed(format!(
                "excludes {path}: an element matches {}",
                entry["match"]
            ));
        }
    }
    for entry in expect
        .get("unchanged")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let from = entry["from"].as_str().unwrap_or_default();
        let from_path = entry["from_path"].as_str().unwrap_or_default();
        // Both compared values must EXIST: a missing path is a failure, never
        // a vacuous None == None pass.
        let original = responses
            .get(from)
            .and_then(|value| walk(&value["payload"], from_path));
        let Some(original) = original else {
            return failed(format!("unchanged {path}: {from}.{from_path} does not exist"));
        };
        let Some(actual) = walk(body, path) else {
            return failed(format!("unchanged {path}: does not exist in the response"));
        };
        if actual != original {
            return failed(format!("unchanged {path}: differs from {from}.{from_path}"));
        }
    }
    for entry in expect
        .get("uniform")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = entry["path"].as_str().unwrap_or_default();
        let field = entry["field"].as_str().unwrap_or_default();
        let items = walk(body, path)
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let minimum = entry["min_items"].as_u64().unwrap_or(2) as usize;
        if items.len() < minimum {
            return failed(format!("uniform {path}: {} items < {minimum}", items.len()));
        }
        let first = walk(&items[0], field);
        if items.iter().any(|item| walk(item, field) != first) {
            return failed(format!("uniform {path}.{field}: values differ"));
        }
    }
    Ok(())
}
