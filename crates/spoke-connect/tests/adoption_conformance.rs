//! Adoption conformance over Connect — corpus interpreter for the
//! `spoke-knowledge-adoption` serving face.
//!
//! Loads the wire vectors from `fixtures/adoption-conformance/` and drives
//! them against the SHIPPED serving path: `surface: "wire"` cases run over a
//! real `connect_responder` ↔ `RemoteAdapter` loopback (the compound
//! capability gate, responder dispatch, `AdoptionPortService` decode /
//! admission / dedup and the served-response validation boundary);
//! `surface: "service"` cases run through `AdoptionPortService::invoke`
//! directly (requests the typed client cannot emit, like an absent
//! `idempotency_key`).
//!
//! The scripted `AdoptionPort` is deliberately dumb — it implements only
//! the provider-owned contract semantics over the seeded tables
//! (owner-private admission, the shared `resolve_retirement_chain`
//! helper, the copy rules, create-only OCC, anchor repeat classification)
//! — so every gate, dedup and validation assertion measures connect code,
//! never fixture policy. The fixture's scoped store applies the shared
//! `assert_adoption_anchor_preserved` gate before OCC, which is how the
//! `anchor-immutable` vectors measure the shipped helper over the wire.
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

use async_trait::async_trait;
use serde_json::{json, Value};
use spoke_connect::core::derive_peer_id_from_ed25519_pubkey;
use spoke_connect::remote::{
    connect_remote_adapter, connect_responder, loopback_transport_pair,
    AdoptionPortService, ConnectResponder, ConnectResponderOptions, HistoryAdmission,
    InMemoryHistoryDedupStore, KnowledgeAdmission, RemoteAdapter, RemoteAdapterOptions,
    RemoteAdoptionService, RemoteIdentity, RemoteKnowledgeFace, RemoteServePorts,
    RemoteServePortsComposite,
};
use spoke_operations::{
    assert_adoption_anchor_preserved, resolve_retirement_chain, spoke_ok, spoke_reject,
    AdoptionPort, KnowledgeEntryPort, RetirementChainEntry, ScopedKnowledgePort, SpokeReject,
    SpokeRejectCode, SpokeResult,
};
use spoke_schemas::{
    HostCapabilityManifest, KnowledgeAdoptRequest, KnowledgeAdoptResponse, KnowledgeEntry,
    Principal, Scope,
};

const HOST_CAPABILITIES: [&str; 2] = ["spoke-knowledge", "spoke-knowledge-adoption"];
const CLIENT_CAPABILITIES: [&str; 2] = ["spoke-knowledge", "spoke-knowledge-adoption"];
/// The destination scope the destination store and the knowledge face
/// admit (the fixture keeps one destination scope).
const DESTINATION_SCOPE: &str = "spc_local";
/// Every source entry is stored under this scope in the source table.
const SOURCE_SCOPE: &str = "spc_shared";
const DEFAULT_ADMITTED_SCOPES: [&str; 2] = [SOURCE_SCOPE, DESTINATION_SCOPE];

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adoption-conformance")
}

// ── Scripted provider ─────────────────────────────────────────────────────

/// Shared state between the scripted `AdoptionPort` and the fixture
/// scoped store: the source table (keyed `(scope_id, entry_id)`, each
/// seeded under `SOURCE_SCOPE` with a storage-owned successor map), the
/// destination map (the same map `RemoteKnowledgeFace` reads/writes, so
/// the written copy is observable through `port.knowledge.get`), the
/// provider call counter, and the destination governance the copy rules
/// reset to.
struct FixtureState {
    sources: Mutex<BTreeMap<String, SeededSource>>,
    destinations: Mutex<BTreeMap<String, Value>>,
    calls: AtomicUsize,
    admitted_scopes: Vec<String>,
    destination_governance: Value,
}

/// One seeded source: the `KnowledgeEntry` wire object plus the
/// storage-owned successor list `resolve_retirement_chain`'s
/// `successors_of` reads (absent = the provider cannot identify
/// successors → ambiguous).
#[derive(Clone)]
struct SeededSource {
    entry: Value,
    successors: Option<Vec<String>>,
}

impl FixtureState {
    fn load(input: &Value) -> Arc<Self> {
        let mut sources = BTreeMap::new();
        for source in input["sources"].as_array().into_iter().flatten() {
            let id = source["entry_id"].as_str().expect("source entry_id").to_string();
            let entry = source["entry"].clone();
            assert_eq!(
                entry["entry_id"].as_str(),
                Some(id.as_str()),
                "source entry_id must match its key"
            );
            let successors = source["successors"].as_array().map(|list| {
                list.iter()
                    .map(|s| s.as_str().expect("successor id").to_string())
                    .collect::<Vec<String>>()
            });
            sources.insert(format!("{SOURCE_SCOPE}|{id}"), SeededSource { entry, successors });
        }
        let mut destinations = BTreeMap::new();
        for entry in input["destinations"].as_array().into_iter().flatten() {
            let id = entry["entry_id"].as_str().expect("destination entry_id").to_string();
            destinations.insert(id, entry.clone());
        }
        let admitted_scopes = input
            .get("admitted_scopes")
            .and_then(Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .map(|s| s.as_str().expect("admitted scope").to_string())
                    .collect()
            })
            .unwrap_or_else(|| DEFAULT_ADMITTED_SCOPES.iter().map(|s| s.to_string()).collect());
        Arc::new(Self {
            sources: Mutex::new(sources),
            destinations: Mutex::new(destinations),
            calls: AtomicUsize::new(0),
            admitted_scopes,
            destination_governance: input
                .get("destination_governance")
                .cloned()
                .unwrap_or_else(|| json!({})),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The generic `NOT_FOUND` — one absence for missing, out-of-scope
    /// and owner-private sources, with no source-identifying details.
    fn generic_not_found<T>() -> SpokeResult<T> {
        spoke_reject(SpokeRejectCode::NotFound, "no such source", None)
    }
}

/// Scripted `AdoptionPort`: the provider-owned contract semantics over
/// the seeded tables — owner-private admission, retirement resolution
/// via the shared helper, the copy rules, create-only OCC and the
/// anchor repeat classification. `calls` is the observable "did the
/// port run" probe (`provider_calls`).
struct ScriptedAdoptionPort {
    state: Arc<FixtureState>,
}

impl ScriptedAdoptionPort {
    /// The destination copy per the contract copy rules: body /
    /// canonical_name / entry_type / status preserved; `source_anchor`
    /// cleared; `modules` minus `cognition`; `extensions` minus
    /// `mindcore_adoption`; owner/disclosure reset to the destination
    /// governance; create revision 1; `adoption_anchor` records the
    /// resolved terminal triple.
    fn copy_for_destination(
        &self,
        terminal_entry: &Value,
        terminal: &spoke_operations::RetirementTerminal,
        destination_id: &str,
        anchor_revision: u64,
    ) -> Value {
        let mut copy = serde_json::Map::new();
        for key in [
            "schema_version",
            "entry_type",
            "canonical_name",
            "status",
            "body",
        ] {
            if let Some(member) = terminal_entry.get(key) {
                copy.insert(key.to_string(), member.clone());
            }
        }
        copy.insert("entry_id".into(), json!(destination_id));
        copy.insert("revision".into(), json!(1));
        if let Some(modules) = terminal_entry.get("modules").and_then(Value::as_object) {
            let modules: serde_json::Map<String, Value> = modules
                .iter()
                .filter(|(key, _)| key.as_str() != "cognition")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            copy.insert("modules".into(), Value::Object(modules));
        }
        if let Some(extensions) = terminal_entry.get("extensions").and_then(Value::as_object) {
            let extensions: serde_json::Map<String, Value> = extensions
                .iter()
                .filter(|(key, _)| key.as_str() != "mindcore_adoption")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            copy.insert("extensions".into(), Value::Object(extensions));
        } else {
            copy.insert("extensions".into(), json!({}));
        }
        for key in ["owner", "disclosure"] {
            if let Some(value) = self.state.destination_governance.get(key) {
                copy.insert(key.to_string(), value.clone());
            }
        }
        copy.insert(
            "adoption_anchor".into(),
            json!({
                "schema_version": 1,
                "source_scope_id": terminal.source_scope_id,
                "source_entry_id": terminal.source_entry_id,
                "source_revision": anchor_revision,
                "extensions": {},
            }),
        );
        Value::Object(copy)
    }
}

#[async_trait]
impl AdoptionPort for ScriptedAdoptionPort {
    async fn adopt_knowledge_entry(
        &self,
        _principal: &Principal,
        scope: &Scope,
        request: &KnowledgeAdoptRequest,
    ) -> SpokeResult<KnowledgeAdoptResponse> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        let request_value = serde_json::to_value(request).expect("request serializes");
        let source_scope_id = request_value["source_scope_id"].as_str().unwrap();
        let source_entry_id = request_value["source_entry_id"].as_str().unwrap();
        let destination_scope_id = request_value["destination_scope_id"].as_str().unwrap();
        let source_revision = request_value["source_revision"].as_u64().unwrap();
        let destination_id = request_value["destination_entry_id"]
            .as_str()
            .map(str::to_string);

        // Admission: an unadmitted source scope is the same generic
        // absence as a missing entry.
        if !self.state.admitted_scopes.iter().any(|s| s == source_scope_id) {
            return FixtureState::generic_not_found();
        }
        // Source load: missing or owner-private to another viewpoint is
        // one generic NOT_FOUND with no source-identifying details.
        let requested = {
            let sources = self.state.sources.lock().expect("sources lock");
            sources
                .get(&format!("{source_scope_id}|{source_entry_id}"))
                .map(|seeded| seeded.entry.clone())
        };
        let Some(requested) = requested else {
            return FixtureState::generic_not_found();
        };
        if requested["disclosure"].as_str() == Some("owner-private") {
            let owner = requested["owner"].as_str();
            let viewpoint = scope
                .viewpoint
                .as_ref()
                .map(|viewpoint| viewpoint.as_str().to_string());
            if owner != viewpoint.as_deref() {
                return FixtureState::generic_not_found();
            }
        }

        // Retirement-chain resolution via the shared helper (cycle /
        // ambiguous / deleted terminal / 33rd edge answers
        // SOURCE_RETIRED with details.retirement_chain). The source table
        // is cloned out of the lock first — a std::sync MutexGuard is
        // !Send across the resolver's await, and the seeded table never
        // mutates during a case.
        let sources = self.state.sources.lock().expect("sources lock").clone();
        let chain_node = |entry: &Value| -> Option<RetirementChainEntry> {
            Some(RetirementChainEntry {
                entry_id: entry["entry_id"].as_str()?.to_string(),
                scope_id: source_scope_id.to_string(),
                status: entry["status"].as_str()?.to_string(),
                revision: entry["revision"].as_u64(),
            })
        };
        let requested_node = chain_node(&requested).expect("chain node");
        let terminal = match resolve_retirement_chain(
            requested_node,
            |node| {
                let seeded = sources.get(&format!("{source_scope_id}|{}", node.entry_id));
                std::future::ready(seeded.and_then(|s| s.successors.clone()))
            },
            |entry_id| {
                let loaded = sources
                    .get(&format!("{source_scope_id}|{entry_id}"))
                    .and_then(|s| chain_node(&s.entry));
                std::future::ready(loaded)
            },
        )
        .await
        {
            SpokeResult::Ok(terminal) => terminal,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let terminal_entry = sources
            .get(&format!("{source_scope_id}|{}", terminal.source_entry_id))
            .map(|seeded| seeded.entry.clone())
            .expect("resolved terminal is stored");

        // The entry adoption actually copies must satisfy the same
        // visibility predicate as the requested source (contract
        // §Admission and owner-private absence): an owner-private
        // resolved terminal invisible to the admitted viewpoint is the
        // same generic absence and no destination write occurs.
        if terminal_entry["disclosure"].as_str() == Some("owner-private") {
            let owner = terminal_entry["owner"].as_str();
            let viewpoint = scope
                .viewpoint
                .as_ref()
                .map(|viewpoint| viewpoint.as_str().to_string());
            if owner != viewpoint.as_deref() {
                return FixtureState::generic_not_found();
            }
        }

        // The destination scope must also be admitted.
        if !self
            .state
            .admitted_scopes
            .iter()
            .any(|s| s == destination_scope_id)
        {
            return FixtureState::generic_not_found();
        }

        let destination_id = destination_id.unwrap_or_else(|| terminal.source_entry_id.clone());
        let mut destinations = self.state.destinations.lock().expect("destinations lock");
        // The stored copy is classified by its anchor (contract §Repeat
        // and idempotency): the repeat comparator is the resolved source
        // tuple `(source_entry_id, resolved_terminal_revision)` — the
        // same tuple the create write records below. For a live source
        // the resolved terminal revision is the requested snapshot
        // revision; for a retirement-resolved source it is the
        // terminal's own revision. A differing resolved revision
        // refuses REVISION_CONFLICT with details.adopted_revision from
        // the stored anchor.
        let resolved_revision = if terminal.source_entry_id == source_entry_id {
            source_revision
        } else {
            terminal.source_revision
        };
        if let Some(occupant) = destinations.get(&destination_id) {
            if let Some(anchor) = occupant.get("adoption_anchor") {
                if anchor["source_scope_id"] == terminal.source_scope_id
                    && anchor["source_entry_id"] == terminal.source_entry_id
                {
                    if anchor["source_revision"].as_u64() == Some(resolved_revision) {
                        // `Variant0.entry` is the response module's own
                        // generated KnowledgeEntry — decode the whole
                        // response envelope instead of constructing it.
                        let response: KnowledgeAdoptResponse = serde_json::from_value(json!({
                            "outcome": "already_present",
                            "entry": occupant,
                        }))
                        .expect("adopt response decodes");
                        return spoke_ok(response);
                    }
                    let adopted_revision = anchor["source_revision"].as_u64().unwrap_or(0);
                    let mut details = serde_json::Map::new();
                    details.insert("adopted_revision".into(), json!(adopted_revision));
                    return spoke_reject(
                        SpokeRejectCode::RevisionConflict,
                        format!(
                            "destination {destination_id} already anchors source revision {adopted_revision}"
                        ),
                        Some(details),
                    );
                }
            }
            // An unrelated occupant retains the existing create-only put
            // refusal and is never overwritten.
            return spoke_reject(
                SpokeRejectCode::RevisionConflict,
                format!("destination {destination_id} is already occupied"),
                None,
            );
        }
        // The anchor records the resolved tuple — the requested
        // revision for a live source (the snapshot pinned), the
        // terminal's own revision for a retirement-resolved source
        // (contract §Repeat and idempotency).
        let anchor_revision = resolved_revision;
        let copy =
            self.copy_for_destination(&terminal_entry, &terminal, &destination_id, anchor_revision);
        destinations.insert(destination_id, copy.clone());
        let response: KnowledgeAdoptResponse = serde_json::from_value(json!({
            "outcome": "adopted",
            "entry": copy,
        }))
        .expect("adopt response decodes");
        spoke_ok(response)
    }
}

/// The fixture scoped store over the shared destination map: the
/// `RemoteKnowledgeFace` backing store. `put` applies the shared
/// `assert_adoption_anchor_preserved` gate BEFORE the OCC check — the
/// anchor-immutability vectors measure that shipped gate over the wire.
struct FixtureScopedStore {
    state: Arc<FixtureState>,
}

#[async_trait]
impl ScopedKnowledgePort for FixtureScopedStore {
    async fn get_scoped_knowledge_entry(
        &self,
        _scope: &Scope,
        entry_id: &str,
    ) -> SpokeResult<KnowledgeEntry> {
        let destinations = self.state.destinations.lock().expect("destinations lock");
        let Some(entry) = destinations.get(entry_id) else {
            return spoke_reject(
                SpokeRejectCode::KnowledgeEntryNotFound,
                format!("KnowledgeEntry not found: {entry_id}"),
                None,
            );
        };
        spoke_ok(serde_json::from_value(entry.clone()).expect("stored entry decodes"))
    }

    async fn put_scoped_knowledge_entry(
        &self,
        _scope: &Scope,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        let mut destinations = self.state.destinations.lock().expect("destinations lock");
        let stored: Option<KnowledgeEntry> = destinations
            .get(&entry.entry_id)
            .map(|stored| serde_json::from_value(stored.clone()).expect("stored entry decodes"));
        if let SpokeResult::Reject(reject) =
            assert_adoption_anchor_preserved(&entry, stored.as_ref())
        {
            return SpokeResult::Reject(reject);
        }
        match expected_base_revision {
            None => {
                if stored.is_some() {
                    let mut details = serde_json::Map::new();
                    details.insert("entry_id".into(), json!(entry.entry_id));
                    return spoke_reject(
                        SpokeRejectCode::RevisionConflict,
                        format!("Entry already exists: {}", entry.entry_id),
                        Some(details),
                    );
                }
            }
            Some(expected) => match stored.as_ref() {
                None => {
                    return spoke_reject(
                        SpokeRejectCode::StoredRevisionStale,
                        format!("KnowledgeEntry not found for update: {}", entry.entry_id),
                        None,
                    )
                }
                Some(stored) => {
                    let current = stored.revision.unwrap_or(0);
                    if current != expected {
                        return spoke_reject(
                            SpokeRejectCode::StoredRevisionStale,
                            format!("Store revision {current} does not match expected base {expected}"),
                            None,
                        );
                    }
                }
            },
        }
        destinations
            .insert(entry.entry_id.clone(), serde_json::to_value(&entry).expect("entry serializes"));
        spoke_ok(entry)
    }

    async fn list_scoped_knowledge_entries(
        &self,
        _scope: &Scope,
    ) -> SpokeResult<Vec<KnowledgeEntry>> {
        let destinations = self.state.destinations.lock().expect("destinations lock");
        let entries = destinations
            .values()
            .map(|entry| serde_json::from_value(entry.clone()).expect("stored entry decodes"))
            .collect();
        spoke_ok(entries)
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

/// The adoption admission context: every authenticated peer gets a
/// principal and the admitted scope — `viewpoint` set when the case
/// seeds `admitted_viewpoint` (owner-private admission).
fn adoption_admission(viewpoint: Option<&str>) -> HistoryAdmission {
    let viewpoint = viewpoint.map(str::to_string);
    Arc::new(move |peer_id: &str| {
        let mut scope_json = json!({ "scope_id": DESTINATION_SCOPE, "extensions": {} });
        if let Some(viewpoint) = &viewpoint {
            scope_json["viewpoint"] = json!(viewpoint);
        }
        spoke_ok((
            principal(peer_id),
            serde_json::from_value(scope_json).expect("valid Scope"),
        ))
    })
}

/// `AdoptionPortService` over the scripted port (+ optional bounded
/// dedup).
fn adoption_service(
    port: Arc<ScriptedAdoptionPort>,
    viewpoint: Option<&str>,
    dedup_capacity: Option<usize>,
) -> Arc<AdoptionPortService> {
    let service = AdoptionPortService::new(port, adoption_admission(viewpoint));
    match dedup_capacity {
        Some(capacity) => Arc::new(
            service.with_dedup_store(Arc::new(InMemoryHistoryDedupStore::with_capacity(capacity))),
        ),
        None => Arc::new(service),
    }
}

/// Knowledge face over the fixture destination store admitting
/// `DESTINATION_SCOPE` for every authenticated peer — the read/write
/// path exercised by the `port.knowledge.get`/`put` steps.
fn knowledge_face(state: Arc<FixtureState>) -> Arc<RemoteKnowledgeFace> {
    let admission: KnowledgeAdmission =
        Arc::new(|_peer| spoke_ok(scope(DESTINATION_SCOPE)));
    Arc::new(RemoteKnowledgeFace {
        store: Arc::new(FixtureScopedStore { state }),
        admission,
    })
}

struct AdoptionHost {
    responder: Arc<ConnectResponder>,
    client: Arc<RemoteAdapter>,
}

/// Start a responder serving the adoption face (or no adoption face)
/// plus the knowledge face over the fixture destination store, then
/// dial it.
async fn dial_host(
    service: Option<Arc<dyn RemoteAdoptionService>>,
    state: Arc<FixtureState>,
    host_capabilities: &[&str],
    client_capabilities: &[&str],
) -> AdoptionHost {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let mut ports = RemoteServePortsComposite::knowledge_only(knowledge_face(state));
    if let Some(service) = service {
        ports = ports.with_adoption(service);
    }
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(ports);
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity { seed: seed_host() },
        manifest: manifest("adoption-conformance-host", host_capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: None,
    })
    .await;
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("adoption-conformance-client", client_capabilities),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial");
    AdoptionHost { responder, client }
}

impl AdoptionHost {
    fn close(&self) {
        self.client.close();
        self.responder.close();
    }
}

// ── Step execution ────────────────────────────────────────────────────────

/// The observed step outcome in the corpus's normalized shape: adopt as
/// `{ "outcome": .., "entry": .. }`, knowledge get/put as the entry.
enum StepOutcome {
    Value(Value),
    Reject(SpokeReject),
}

/// One step's request fields, normalized: the op name plus its raw
/// request object (adopt requests and `port.knowledge.put`/`get`
/// payloads share the same `request` member).
struct StepRequest {
    op: String,
    request: Value,
}

fn parse_step(step: &Value, default_op: Option<&str>) -> StepRequest {
    StepRequest {
        op: step
            .get("op")
            .and_then(Value::as_str)
            .or(default_op)
            .expect("step op")
            .to_string(),
        request: step["request"].clone(),
    }
}

/// Dispatch a step. For `wire` the typed client is the boundary (adopt
/// and the knowledge ops); for `service` the raw request value goes
/// through `AdoptionPortService::invoke` verbatim.
async fn run_step(
    surface: &str,
    host: Option<&AdoptionHost>,
    service: &Arc<dyn RemoteAdoptionService>,
    client_peer: &str,
    step: &StepRequest,
) -> StepOutcome {
    if surface == "service" {
        assert!(
            step.op.starts_with("knowledge."),
            "service surface serves adoption ops only, got {}",
            step.op
        );
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
    let client_scope = scope(DESTINATION_SCOPE);
    match step.op.as_str() {
        "knowledge.adopt" => {
            let request: KnowledgeAdoptRequest = serde_json::from_value(step.request.clone())
                .expect("adopt step request decodes");
            match host
                .client
                .adopt_knowledge_entry(&client_principal, &client_scope, &request)
                .await
            {
                SpokeResult::Ok(response) => {
                    StepOutcome::Value(serde_json::to_value(response).expect("response serializes"))
                }
                SpokeResult::Reject(reject) => StepOutcome::Reject(reject),
            }
        }
        "port.knowledge.get" => {
            let entry_id = step.request["entry_id"].as_str().expect("get entry_id");
            match host.client.get_knowledge_entry(entry_id).await {
                SpokeResult::Ok(entry) => {
                    StepOutcome::Value(serde_json::to_value(entry).expect("entry serializes"))
                }
                SpokeResult::Reject(reject) => StepOutcome::Reject(reject),
            }
        }
        "port.knowledge.put" => {
            let entry: KnowledgeEntry = serde_json::from_value(step.request["entry"].clone())
                .expect("put step entry decodes");
            match host
                .client
                .put_knowledge_entry(entry, step.request["expected_base_revision"].as_u64())
                .await
            {
                SpokeResult::Ok(entry) => {
                    StepOutcome::Value(serde_json::to_value(entry).expect("entry serializes"))
                }
                SpokeResult::Reject(reject) => StepOutcome::Reject(reject),
            }
        }
        other => panic!("unknown adoption op {other}"),
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
        if let Some(absent) = expect.get("absent_details").and_then(Value::as_array) {
            for key in absent {
                let key = key.as_str().expect("absent_details member");
                let present = reject
                    .details
                    .as_ref()
                    .is_some_and(|d| d.contains_key(key));
                assert!(
                    !present,
                    "case {case_id}: details.{key} must be absent (generic absence), got {:?}",
                    reject.details
                );
            }
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
        if let Some(absent) = expect.get("absent").and_then(Value::as_array) {
            for path in absent {
                let path = path.as_str().expect("absent path");
                assert!(
                    path_get(value, path).is_none(),
                    "case {case_id}: member {path} must be absent, got {:?}",
                    path_get(value, path)
                );
            }
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

const REQUIRED_SCENARIOS: [&str; 6] = [
    "adopt-basic",
    "anchor-immutable",
    "idempotent-repeat",
    "private-source",
    "retirement-chain",
    "revision-conflict",
];

const EXPECT_KEYS: [&str; 8] = [
    "error_code",
    "details",
    "absent_details",
    "success",
    "state",
    "present",
    "absent",
    "provider_calls",
];

const CORPUS_OPS: [&str; 3] = ["knowledge.adopt", "port.knowledge.get", "port.knowledge.put"];

#[test]
fn corpus_integrity() {
    let scenarios = load_scenarios();
    let ids: Vec<&str> = scenarios.iter().map(|s| s.id.as_str()).collect();
    for required in REQUIRED_SCENARIOS {
        assert!(ids.contains(&required), "scenario {required} missing");
    }
    let mut seen_ids = std::collections::BTreeSet::new();
    for scenario in &scenarios {
        for case in &scenario.cases {
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
            assert!(CORPUS_OPS.contains(&op), "case {id} op {op}");
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
                    assert!(CORPUS_OPS.contains(&op), "case {id} step op {op}");
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
            for source in case["input"]["sources"].as_array().into_iter().flatten() {
                assert!(source["entry_id"].is_string(), "case {id} source entry_id");
                assert!(source["entry"].is_object(), "case {id} source entry");
                assert_eq!(
                    source["entry"]["entry_id"], source["entry_id"],
                    "case {id}: source entry_id must match its key"
                );
            }
        }
    }
    // No corpus-case count pin: adding a case must not break an aggregate
    // assertion — per-case id uniqueness and the per-case semantic checks
    // above carry the corpus contract.
}

// ── Corpus execution ──────────────────────────────────────────────────────

#[tokio::test]
async fn adoption_wire_conformance() {
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
    let state = FixtureState::load(input);
    let port = Arc::new(ScriptedAdoptionPort {
        state: Arc::clone(&state),
    });
    let dedup_capacity = input
        .get("dedup_capacity")
        .and_then(Value::as_u64)
        .map(|c| c as usize);
    let viewpoint = input.get("admitted_viewpoint").and_then(Value::as_str);
    let service: Arc<dyn RemoteAdoptionService> =
        adoption_service(port, viewpoint, dedup_capacity);
    let adoption_face = input
        .get("adoption_face")
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
    let host_caps: Vec<String> = input
        .get("host_capabilities")
        .and_then(Value::as_array)
        .map(|caps| {
            caps.iter()
                .map(|c| c.as_str().expect("capability").to_string())
                .collect()
        })
        .unwrap_or_else(|| HOST_CAPABILITIES.iter().map(|c| c.to_string()).collect());
    let client_peer = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    let body = std::panic::AssertUnwindSafe(async {
        // Wire surface: dial a responder whose composite carries the
        // service (or not); service surface talks to the service directly.
        let host = if surface == "wire" {
            let caps: Vec<&str> = client_caps.iter().map(String::as_str).collect();
            let host_caps: Vec<&str> = host_caps.iter().map(String::as_str).collect();
            Some(
                dial_host(
                    adoption_face.then(|| Arc::clone(&service)),
                    Arc::clone(&state),
                    &host_caps,
                    &caps,
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
                    check_expect(&case_id, expect, &outcome, state.calls());
                } else if let StepOutcome::Reject(reject) = &outcome {
                    panic!("setup step rejected: {reject:?}");
                }
            }
        }

        // The op under test.
        let parsed_op = parse_step(case, None);
        let outcome = execute(&parsed_op).await;
        check_expect(&case_id, &case["expect"], &outcome, state.calls());

        // then steps.
        if let Some(then) = case["then"].as_array() {
            for step in then {
                let parsed = parse_step(step, None);
                let outcome = execute(&parsed).await;
                if let Some(expect) = step.get("expect") {
                    check_expect(&case_id, expect, &outcome, state.calls());
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
