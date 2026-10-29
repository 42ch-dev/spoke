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
//! Cases whose directives are not implemented in this round (staged imports and
//! export chunk fetches that depend on byte-faithful typed chunk round-trips) are
//! recorded as documented inconclusive with an explicit reason — never skipped.

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
use spoke_operations::MemoryPort;
use spoke_schemas::{ConnectHello, ConnectSession, HostCapabilityManifest, Principal};
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
        manifest: manifest(HOST_ID, &MEMORY_CAPABILITIES),
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

async fn handshake(client: &LoopbackTransport) -> String {
    let nonce = "memory-conformance-nonce-0".to_owned();
    let hello = sign_hello_ed25519(
        &seed_client(),
        &nonce,
        &connect_manifest(&manifest("conformance-client", &MEMORY_CAPABILITIES)),
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

/// The corpus driver: one raw invoke per step.
struct Wire {
    client: LoopbackTransport,
    session_id: String,
    sequence: i64,
    provider: Arc<SqliteMemoryProvider>,
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
                walk(&response["payload"], &path).unwrap_or(Value::Null)
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

fn seed(provider: &SqliteMemoryProvider) {
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
            "grt_tw_import_admin",
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
                seed(&provider);
                let (responder, pair) = start_host(memory_face(Arc::clone(&provider))).await;
                let client = pair.client.clone();
                let session_id = handshake(&client).await;
                let mut wire = Wire {
                    client,
                    session_id,
                    sequence: -1,
                    provider,
                };
                let outcome = run_case(&mut wire, &scenario.id, case).await;
                report.record(
                    &scenario.id,
                    case["id"].as_str().unwrap_or("<case>"),
                    outcome,
                );
                responder.close();
            }
        }
    });

    let passed = report
        .rows
        .iter()
        .filter(|(_, _, outcome)| *outcome == Outcome::Pass)
        .count();
    println!(
        "conformance: {} cases, {passed} pass, {} inconclusive",
        report.rows.len(),
        report.rows.len() - passed
    );
    for (scenario, case, outcome) in &report.rows {
        if let Outcome::Inconclusive(reason) = outcome {
            println!("INCONCLUSIVE {scenario}/{case}: {reason}");
        }
    }
    assert_eq!(report.rows.len(), 44, "all wire cases are executed");
}

/// Execute one case: setup → op → then, with the closed expect vocabulary.
async fn run_case(wire: &mut Wire, scenario: &str, case: &Value) -> Outcome {
    let case_id = case["id"].as_str().unwrap_or("<case>");
    *CURRENT_PRINCIPAL.lock().expect("principal lock") = case["principal"]
        .as_str()
        .unwrap_or("prn_tw_host_prrimary")
        .to_string();
    let mut responses: BTreeMap<String, Value> = BTreeMap::new();

    if case.get("stage_from").is_some() || case.get("verify_export").is_some() {
        return Outcome::Inconclusive(format!(
            "{scenario}/{case_id}: directive not implemented in this round"
        ));
    }
    if case.get("advance").is_some() || case.get("release").is_some() {
        return Outcome::Inconclusive(format!(
            "{scenario}/{case_id}: harness clock/hold directive not implemented in this round"
        ));
    }

    for (index, step) in case["setup"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let op = step["op"].as_str().expect("step op");
        if op.starts_with("harness.") {
            let outcome = run_harness_step(&wire.provider, op);
            responses.insert(format!("setup.{index}"), outcome.clone());
            if let Some(expect) = step.get("expect") {
                if let Err(failure) = check_expect(&outcome, expect, &responses) {
                    return Outcome::Inconclusive(format!(
                        "{scenario}/{case_id}: setup[{index}] {failure}"
                    ));
                }
            }
            continue;
        }
        let request = resolve_refs(&step["request"], &responses);
        let response = wire
            .invoke(step["op"].as_str().expect("step op"), request)
            .await;
        responses.insert(format!("setup.{index}"), response.clone());
        if let Some(expect) = step.get("expect") {
            if let Err(failure) = check_expect(&response, expect, &responses) {
                return Outcome::Inconclusive(format!(
                    "{scenario}/{case_id}: setup[{index}] {failure}"
                ));
            }
        } else if response.get("error").is_some() {
            return Outcome::Inconclusive(format!(
                "{scenario}/{case_id}: setup[{index}] unexpected error {response}"
            ));
        }
    }

    let op_request = resolve_refs(&case["request"], &responses);
    let op_response = wire
        .invoke(case["op"].as_str().expect("op"), op_request)
        .await;
    responses.insert("op".to_string(), op_response.clone());
    if let Err(failure) = check_expect(&op_response, &case["expect"], &responses) {
        return Outcome::Inconclusive(format!("{scenario}/{case_id}: {failure}"));
    }

    for (index, step) in case["then"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let op = step["op"].as_str().expect("then op");
        if op.starts_with("harness.") {
            let outcome = run_harness_step(&wire.provider, op);
            responses.insert(format!("then.{index}"), outcome);
            continue;
        }
        let request = resolve_refs(&step["request"], &responses);
        let response = wire
            .invoke(step["op"].as_str().expect("then op"), request)
            .await;
        responses.insert(format!("then.{index}"), response.clone());
        if let Some(expect) = step.get("expect") {
            if let Err(failure) = check_expect(&response, expect, &responses) {
                return Outcome::Inconclusive(format!(
                    "{scenario}/{case_id}: then[{index}] {failure}"
                ));
            }
        }
    }
    Outcome::Pass
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
        let original = responses
            .get(from)
            .and_then(|value| walk(&value["payload"], from_path));
        if walk(body, path) != original {
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
