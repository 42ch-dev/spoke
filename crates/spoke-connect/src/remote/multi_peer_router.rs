//! `MultiPeerRouter` — capability-selected multi-peer async `BaselinePorts`
//! over N registered per-peer `RemoteAdapter` instances (frozen multi-peer
//! routing contract: `.mstar/specs/spoke-remote-adapter.md (Multi-peer section)`;
//! normative design intent in the RemoteAdapter spec "Multi-peer
//! registry/composer" staged follow-on). Behavioral twin of
//! `packages/spoke-connect-ts/src/remote/multi-peer-router.ts`.
//!
//! The router sits ABOVE per-peer `RemoteAdapter`s and BELOW `orchestrate*`:
//! consumers dial and register established adapters, then keep calling
//! `orchestrate_upsert(&router, req)` etc. with no per-op `peer_id`. Each
//! `BaselinePorts` call selects exactly one peer by the locked §3 algorithm —
//! capability → namespace → authority hard filters, role soft partition,
//! deterministic lexicographic UTF-8 `peer_id` tie-break (§4) — and
//! delegates. The selected peer's underlying `SpokeResult` reject is
//! returned as-is (§7.2: no automatic alternate-retry, no remap).
//! `get_host_capability_manifest` / `list_peer_host_capability_manifests`
//! are aggregated locally from the cached per-peer manifests (§6: composed
//! view / per-peer array — no round-trip).
//!
//! Public surface (§8): `connect_multi_peer_router(options)`,
//! `register_peer`, `unregister_peer`, `list_peers`, the async
//! `BaselinePorts` six families, and the tool-invoke face
//! `invoke_tool(capability_id, arguments)` (D14 — exact-capability hard
//! filter, lowest-`peer_id` tie-break, terminal `no_capable_peer` reject,
//! composed-manifest `tools[]` union). Per-peer adapters are encapsulated —
//! the router never exposes them after registration; consumers hold only the
//! adapter `Arc` they dialed themselves.
//!
//! The router stores `Arc<dyn RoutedRemoteAdapter>` (the parity shape of
//! the TS `RoutedRemoteAdapter` interface, which composes `BaselinePorts`
//! plus the three read-only session getters). `RemoteAdapter` satisfies it
//! by forwarding each port method to its concrete `BaselinePorts` impl.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use spoke_operations::{
    spoke_ok, FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort,
    ScopeQueryPort, SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::host_capability_manifest::{
    HostCapabilityManifestExtensionsKey, HostCapabilityManifestHostId, ToolDescriptor,
};
use spoke_schemas::{
    Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Rule, Scope, TimelineEvent,
};

use super::remote_adapter::{RemoteAdapter, RemoteAdapterState};

/// The router's own identity when the consumer configures none (contract §8).
const DEFAULT_ROUTER_HOST_ID: &str = "multi-peer-router";

// ── Locked op → selection-input tables (contract §2 / §3) ────────────────

/// Required capability per op family (contract §2 — locked) plus the frozen
/// §6 `tools.` prefix rule (the selection table gains the same widening as
/// core dispatch): a `tools.<ns>.<tool_id>` op is self-describing — the
/// required capability IS the op string itself (no registry, no umbrella
/// flag). Orchestrated baseline families and the `port.*` baseline ops
/// require `spoke-baseline`; the computable families require
/// `l2-computable`. Product-defined ops are product-documented and have no
/// row here; selection REJECTS ops outside this table (`no_capable_peer`) —
/// an op with no gate must not fall through ungated (QC2 S-1). The router's
/// fixed six-family surface only ever queries the `port.*` rows (mirrors the
/// RemoteAdapter `PORT_OPS` catalogue). The output lifetime is tied to `op`
/// (`Option<&str>`); static rows coerce.
fn required_capability(op: &str) -> Option<&str> {
    if op.starts_with("tools.") {
        return Some(op);
    }
    match op {
        // Orchestrated op families.
        "upsert" | "promote" | "relate" | "check" | "assemble" => Some("spoke-baseline"),
        "project" | "compute" => Some("l2-computable"),
        // port.* baseline ops.
        "port.knowledge.get"
        | "port.knowledge.put"
        | "port.relation.get"
        | "port.relation.put"
        | "port.scope.list_knowledge_entries"
        | "port.scope.list_timeline_events"
        | "port.finding.put"
        | "port.rule.list" => Some("spoke-baseline"),
        // Optional l2-computable port ops (not delegated by the baseline surface).
        "port.computable.project" | "port.computable.compute" => Some("l2-computable"),
        _ => None,
    }
}

/// Preferred role per op (contract §3 — locked, SOFT preference only: it
/// reorders candidates, never rejects a capable peer for lacking the role).
/// `upsert` / `promote` / `relate` and `port.*` baseline ops carry no role
/// preference — capability + namespace are the discriminators.
fn preferred_role(op: &str) -> Option<&'static str> {
    match op {
        "check" => Some("checker"),
        "assemble" => Some("assembler"),
        "project" | "compute" => Some("l2-computable"),
        _ => None,
    }
}

// ── Payload-derived selection inputs (contract §2 / §3) ──────────────────

/// The request's collaboration namespace, derived from the op payload when
/// it carries one (contract §2 — "derived from `Scope` when the op payload
/// carries one"). Products surface the namespace either at the payload top
/// level (`namespace`) or on the payload's `Scope` (`scope.namespace`). When
/// neither is present the namespace filter is skipped (§3 step 3). No
/// wildcard in v1: a literal `"*"` is the literal string, never a match-all.
fn request_namespace(payload: &Value) -> Option<String> {
    if let Some(Value::String(top_level)) = payload.get("namespace") {
        return Some(top_level.clone());
    }
    if let Some(Value::Object(scope)) = payload.get("scope") {
        if let Some(Value::String(nested)) = scope.get("namespace") {
            return Some(nested.clone());
        }
    }
    None
}

/// The request's authority scope key, when the payload carries one (contract
/// §3 step 4 — the authority filter applies when the peer manifest AND the
/// request both declare a scope key). Same carrier shapes as the namespace.
fn request_scope_key(payload: &Value) -> Option<String> {
    if let Some(Value::String(top_level)) = payload.get("scope_key") {
        return Some(top_level.clone());
    }
    if let Some(Value::Object(scope)) = payload.get("scope") {
        if let Some(Value::String(nested)) = scope.get("scope_key") {
            return Some(nested.clone());
        }
    }
    None
}

/// The locked §5 no-capable-peer reject: terminal, no retry, no fallback.
fn no_capable_peer(op: &str, reason: String) -> SpokeReject {
    let mut details = Map::new();
    details.insert(
        "wire_code".to_string(),
        Value::String("no_capable_peer".to_string()),
    );
    details.insert(
        "kind".to_string(),
        Value::String("no_capable_peer".to_string()),
    );
    details.insert("op".to_string(), Value::String(op.to_string()));
    SpokeReject {
        code: SpokeRejectCode::CapabilityPortMissing,
        message: format!("no capable peer for {op}: {reason}"),
        details: Some(details),
    }
}

/// Set-union preserving first-seen order across the inputs (contract §6).
/// Rust `Vec`s carry no `minItems` constraint, so the empty union (a router
/// with zero connected peers) is expressed directly.
fn union_of_strings<'a>(arrays: impl IntoIterator<Item = &'a [String]>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for array in arrays {
        for item in array {
            if seen.insert(item.as_str()) {
                out.push(item.clone());
            }
        }
    }
    out
}

// ── Pure selection (contract §3 + §4 + §5) ───────────────────────────────

/// One selection candidate: a registered peer's id + cached manifest (§2).
#[derive(Debug, Clone)]
pub struct SelectablePeer {
    pub peer_id: String,
    pub manifest: HostCapabilityManifest,
}

/// The locked §3 selection algorithm over an explicit candidate set:
///
/// 1. Capability filter (hard): the peer's `capabilities` MUST include the
///    required capability for the op (§2 mapping table). Ops outside the
///    mapping table are rejected outright — no gate to run, never an
///    ungated fall-through (QC2 S-1).
/// 2. Namespace filter (hard): when the request payload carries a namespace,
///    the peer's `namespaces` MUST include it (exact match; skipped when the
///    request carries none; no wildcard).
/// 3. Authority filter (hard when both sides declare): a peer whose
///    `authority.scope_key` is present AND mismatches the request's scope key
///    is excluded; when only one side (or neither) declares, the filter is
///    skipped for that peer.
/// 4. Role preference (soft): peers with the op's preferred role in `roles[]`
///    are preferred over peers without it; never rejects for lacking it.
/// 5. Deterministic tie-break: lowest `peer_id` in lexicographic UTF-8 byte
///    order (§4) — no clock, no random, no health score. Rust `String`
///    ordering IS UTF-8 byte order, so a plain sort is the faithful sort.
///
/// Returns a `no_capable_peer` reject (§5) when the op has no capability
/// mapping or no candidate survives the hard gates — terminal, stable, no
/// wrong-peer fallback.
pub fn select_peer_for_op(
    candidates: &[SelectablePeer],
    op: &str,
    payload: &Value,
) -> SpokeResult<SelectablePeer> {
    if candidates.is_empty() {
        return SpokeResult::Reject(no_capable_peer(
            op,
            "no established peer registered".to_string(),
        ));
    }

    // Unknown ops are rejected outright: the §3 capability gate is step 1 of
    // every selection, and an op outside the locked mapping table has no gate
    // to run — falling through would select an arbitrary Established peer
    // (QC2 S-1). Same terminal §5 reject shape as a no-match.
    let Some(required_capability) = required_capability(op) else {
        return SpokeResult::Reject(no_capable_peer(
            op,
            format!("no capability mapping for unknown op \"{op}\""),
        ));
    };
    let mut survivors: Vec<&SelectablePeer> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .manifest
                .capabilities
                .iter()
                .any(|c| c == required_capability)
        })
        .collect();
    if survivors.is_empty() {
        return SpokeResult::Reject(no_capable_peer(
            op,
            format!("no peer advertises capability \"{required_capability}\""),
        ));
    }

    let namespace = request_namespace(payload);
    if let Some(namespace) = namespace {
        survivors.retain(|candidate| {
            candidate
                .manifest
                .namespaces
                .iter()
                .any(|declared| declared.as_str() == namespace)
        });
        if survivors.is_empty() {
            return SpokeResult::Reject(no_capable_peer(
                op,
                format!("no peer advertises namespace \"{namespace}\""),
            ));
        }
    }

    let scope_key = request_scope_key(payload);
    if let Some(scope_key) = scope_key {
        survivors.retain(|candidate| {
            let declared = candidate
                .manifest
                .authority
                .as_ref()
                .map(|authority| authority.scope_key.as_str());
            // Both sides declare → exact match required; only one side
            // declares → filter skipped for this peer.
            declared.is_none() || declared == Some(scope_key.as_str())
        });
        if survivors.is_empty() {
            return SpokeResult::Reject(no_capable_peer(
                op,
                format!("no peer authority scope key matches \"{scope_key}\""),
            ));
        }
    }

    let preferred_role = preferred_role(op);
    if let Some(preferred_role) = preferred_role {
        let role_matched: Vec<&SelectablePeer> = survivors
            .iter()
            .copied()
            .filter(|candidate| {
                candidate
                    .manifest
                    .roles
                    .iter()
                    .any(|role| role == preferred_role)
            })
            .collect();
        if !role_matched.is_empty() {
            survivors = role_matched;
        }
    }

    survivors.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    let winner = survivors[0];
    SpokeResult::Ok(SelectablePeer {
        peer_id: winner.peer_id.clone(),
        manifest: winner.manifest.clone(),
    })
}

// ── Router ───────────────────────────────────────────────────────────────

/// The per-peer adapter surface the router composes — satisfied by
/// `RemoteAdapter` (its read-only session getters + async `BaselinePorts`).
/// The router reads `state` (candidate gate), `remote_peer_id` (registry key
/// + tie-break), and `remote_manifest` (cached at registration); it delegates
/// `BaselinePorts` calls to the selected adapter. Envelope auth stays
/// enforced inside each per-peer session — selection is a routing decision
/// above the adapters and adds no bypass.
///
/// The ten port methods mirror the six `BaselinePorts` families so the trait
/// is object-safe: `Arc<dyn RoutedRemoteAdapter>` dispatches through the
/// trait object's own vtable (a trait object implements its trait by
/// definition), which a supertrait-bound trait object would not — see the
/// module docs.
#[async_trait]
pub trait RoutedRemoteAdapter: Send + Sync {
    fn state(&self) -> RemoteAdapterState;
    fn remote_peer_id(&self) -> Option<String>;
    fn remote_manifest(&self) -> Option<HostCapabilityManifest>;

    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry>;
    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry>;
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation>;
    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation>;
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>>;
    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>>;
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>>;
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest>;
    async fn list_peer_host_capability_manifests(&self)
        -> SpokeResult<Vec<HostCapabilityManifest>>;

    /// Forward tool-invoke face (frozen §6) — satisfied by
    /// `RemoteAdapter::invoke_tool`; the router delegates tool invokes to
    /// the selected peer's adapter (the router never crafts envelopes
    /// itself).
    async fn invoke_tool(&self, capability_id: &str, arguments: Value) -> SpokeResult<Value>;
}

/// Forward the per-peer adapter surface to `RemoteAdapter`'s concrete
/// `BaselinePorts` impls (the ten port methods) plus its read-only session
/// getters.
#[async_trait]
impl RoutedRemoteAdapter for RemoteAdapter {
    fn state(&self) -> RemoteAdapterState {
        RemoteAdapter::state(self)
    }

    fn remote_peer_id(&self) -> Option<String> {
        RemoteAdapter::remote_peer_id(self)
    }

    fn remote_manifest(&self) -> Option<HostCapabilityManifest> {
        RemoteAdapter::remote_manifest(self)
    }

    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        <Self as KnowledgeEntryPort>::get_knowledge_entry(self, entry_id).await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        <Self as KnowledgeEntryPort>::put_knowledge_entry(self, entry, expected_base_revision).await
    }

    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        <Self as RelationPort>::get_relation(self, relation_id).await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        <Self as RelationPort>::put_relation(self, relation, expected_base_revision).await
    }

    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        <Self as ScopeQueryPort>::list_knowledge_entries(self, scope).await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        <Self as ScopeQueryPort>::list_timeline_events(self, scope).await
    }

    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        <Self as FindingPort>::put_findings(self, findings).await
    }

    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        <Self as RuleQueryPort>::list_rules(self, rule_refs).await
    }

    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        <Self as HostManifestPort>::get_host_capability_manifest(self).await
    }

    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        <Self as HostManifestPort>::list_peer_host_capability_manifests(self).await
    }

    async fn invoke_tool(&self, capability_id: &str, arguments: Value) -> SpokeResult<Value> {
        RemoteAdapter::invoke_tool(self, capability_id, arguments).await
    }
}

/// Constructor options (contract §8 — none required; the router starts with
/// zero peers; consumers dial each peer's `RemoteAdapter` themselves and
/// register the established adapter).
#[derive(Debug, Clone, Default)]
pub struct MultiPeerRouterOptions {
    /// The router's own host identity — the composed view's `host_id`
    /// (contract §6: the local node's identity, NOT a peer's). Optional:
    /// defaults to `"multi-peer-router"`.
    pub host_id: Option<String>,
}

/// Registration failure — the adapter has no established connect session
/// (no verified remote peer id / no cached manifest). Consumers dial first
/// (§8); mirrors the TS `registerPeer` throw.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MultiPeerRouterError {
    #[error("registerPeer requires an adapter with an established connect session (verified remote peer id unavailable)")]
    NoPeerId,
    #[error("registerPeer requires an adapter with an established connect session (remote capability manifest unavailable)")]
    NoManifest,
}

/// A registered peer's stored adapter + manifest cache (contract §1/§7.4).
#[derive(Clone)]
struct RegisteredPeer {
    adapter: Arc<dyn RoutedRemoteAdapter>,
    /// Per-peer manifest cache, captured at registration.
    manifest: HostCapabilityManifest,
}

/// One selection outcome: the chosen peer's adapter.
struct SelectedPeer {
    adapter: Arc<dyn RoutedRemoteAdapter>,
}

/// A connected (Established) peer's id + cached manifest.
struct ConnectedPeer {
    peer_id: String,
    manifest: HostCapabilityManifest,
}

/// Multi-peer capability router: registry (§7.4) + locked §3 selection +
/// async `BaselinePorts` delegation (six families) + §6 HostManifest
/// aggregation. Construct via `connect_multi_peer_router` — the router
/// starts with zero peers; consumers dial each peer's `RemoteAdapter` and
/// register the established adapter.
pub struct MultiPeerRouter {
    host_id: String,
    /// Registered peers by `peer_id` (dynamic insert/delete).
    peers: Mutex<HashMap<String, RegisteredPeer>>,
    /// Registered peer ids in registration order (the registry, not
    /// selection — parity with the TS `Map` insertion order).
    registration_order: Mutex<Vec<String>>,
}

impl MultiPeerRouter {
    fn new(options: MultiPeerRouterOptions) -> Self {
        // An empty configured host id is treated as unset (the schema's
        // `host_id` requires a non-empty string) — same defaulting as TS
        // `options.hostId ?? DEFAULT_ROUTER_HOST_ID`.
        let host_id = options
            .host_id
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| DEFAULT_ROUTER_HOST_ID.to_string());
        Self {
            host_id,
            peers: Mutex::new(HashMap::new()),
            registration_order: Mutex::new(Vec::new()),
        }
    }

    // ── Registry (contract §7.4) ───────────────────────────────────────────

    /// Register an adapter under its verified remote peer id. Idempotent on
    /// `peer_id`: re-registering the same id replaces the stored adapter and
    /// re-caches its manifest. Returns the `peer_id`. Errors when the
    /// adapter has no established session (no verified peer id / no cached
    /// manifest) — consumers dial first (§8). Does NOT close the adapter. A
    /// `Closed` adapter may remain registered — it is simply excluded from
    /// the candidate set; the consumer MAY call `unregister_peer` to evict.
    pub fn register_peer(
        &self,
        adapter: Arc<dyn RoutedRemoteAdapter>,
    ) -> Result<String, MultiPeerRouterError> {
        let peer_id = adapter
            .remote_peer_id()
            .filter(|id| !id.is_empty())
            .ok_or(MultiPeerRouterError::NoPeerId)?;
        let manifest = adapter
            .remote_manifest()
            .ok_or(MultiPeerRouterError::NoManifest)?;
        let mut peers = self.peers.lock().expect("peers lock");
        if !peers.contains_key(&peer_id) {
            self.registration_order
                .lock()
                .expect("order lock")
                .push(peer_id.clone());
        }
        peers.insert(peer_id.clone(), RegisteredPeer { adapter, manifest });
        Ok(peer_id)
    }

    /// Remove a peer from the registry. No-op if not registered. Does NOT
    /// close the adapter (the consumer owns the adapter lifecycle, §7.4).
    pub fn unregister_peer(&self, peer_id: &str) {
        self.peers.lock().expect("peers lock").remove(peer_id);
        self.registration_order
            .lock()
            .expect("order lock")
            .retain(|registered| registered != peer_id);
    }

    /// Registered peer ids, in registration order (the registry, not
    /// selection).
    pub fn list_peers(&self) -> Vec<String> {
        self.registration_order.lock().expect("order lock").clone()
    }

    /// Tool-invoke face (frozen §6): select the peer whose cached hello
    /// manifest `capabilities[]` contains the EXACT tool capability string
    /// (the selection table's `tools.` prefix rule resolves the required
    /// capability to the op itself), then delegate to the selected peer's
    /// adapter `invoke_tool` — the router never crafts envelopes itself. No
    /// namespace/authority/role filters for tools (the capability string is
    /// ns-scoped; tool payloads carry no `Scope`); deterministic tie-break
    /// = lowest `peer_id`; none → the existing terminal `no_capable_peer`
    /// reject (`details.op = capability_id`). The selected peer's underlying
    /// `SpokeResult` reject is returned as-is (§7.2 — no alternate-retry).
    pub async fn invoke_tool(&self, capability_id: &str, arguments: Value) -> SpokeResult<Value> {
        let selected = match self.select_for_op(capability_id, &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.invoke_tool(capability_id, arguments).await
    }

    // ── Selection + delegation internals ───────────────────────────────────

    /// Connected (Established) peers with their cached manifests,
    /// peer_id-sorted (lexicographic UTF-8 byte order — contract §4).
    fn connected_peers(&self) -> Vec<ConnectedPeer> {
        let mut connected: Vec<ConnectedPeer> = self
            .peers
            .lock()
            .expect("peers lock")
            .iter()
            .filter(|(_, entry)| entry.adapter.state() == RemoteAdapterState::Established)
            .map(|(peer_id, entry)| ConnectedPeer {
                peer_id: peer_id.clone(),
                manifest: entry.manifest.clone(),
            })
            .collect();
        connected.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
        connected
    }

    /// Select one peer for `op` + selection payload, then resolve its stored
    /// adapter. `no_capable_peer` (§5) when no Established peer survives the
    /// hard gates.
    fn select_for_op(&self, op: &str, payload: &Value) -> Result<SelectedPeer, SpokeReject> {
        // Candidate set: Established peers only (Closed / Disconnected /
        // Handshaking excluded, contract §3 step 1 / §7.4).
        let candidates: Vec<SelectablePeer> = self
            .connected_peers()
            .into_iter()
            .map(|peer| SelectablePeer {
                peer_id: peer.peer_id,
                manifest: peer.manifest,
            })
            .collect();
        let selected = select_peer_for_op(&candidates, op, payload);
        let selected = match selected {
            SpokeResult::Ok(selected) => selected,
            SpokeResult::Reject(reject) => return Err(reject),
        };
        let entry = self
            .peers
            .lock()
            .expect("peers lock")
            .get(&selected.peer_id)
            .cloned()
            .ok_or_else(|| {
                // Unreachable: candidates are built from the registry keys above.
                no_capable_peer(
                    op,
                    format!("selected peer {} is no longer registered", selected.peer_id),
                )
            })?;
        Ok(SelectedPeer {
            adapter: entry.adapter,
        })
    }
}

// ── BaselinePorts (async) — select a peer per call, then delegate (§1/§8) ─

#[async_trait]
impl KnowledgeEntryPort for MultiPeerRouter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        let selected = match self.select_for_op("port.knowledge.get", &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.get_knowledge_entry(entry_id).await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        let selected = match self.select_for_op("port.knowledge.put", &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected
            .adapter
            .put_knowledge_entry(entry, expected_base_revision)
            .await
    }
}

#[async_trait]
impl RelationPort for MultiPeerRouter {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        let selected = match self.select_for_op("port.relation.get", &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.get_relation(relation_id).await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        let selected = match self.select_for_op("port.relation.put", &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected
            .adapter
            .put_relation(relation, expected_base_revision)
            .await
    }
}

#[async_trait]
impl ScopeQueryPort for MultiPeerRouter {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        // The selection payload carries the request Scope so the §2
        // namespace / §3 authority derivations see it (the same Scope the
        // peer receives).
        let selected = match self.select_for_op(
            "port.scope.list_knowledge_entries",
            &json!({ "scope": scope }),
        ) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.list_knowledge_entries(scope).await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        let selected = match self.select_for_op(
            "port.scope.list_timeline_events",
            &json!({ "scope": scope }),
        ) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.list_timeline_events(scope).await
    }
}

#[async_trait]
impl FindingPort for MultiPeerRouter {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        let selected = match self.select_for_op("port.finding.put", &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.put_findings(findings).await
    }
}

#[async_trait]
impl RuleQueryPort for MultiPeerRouter {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        let selected = match self.select_for_op("port.rule.list", &json!({})) {
            Ok(selected) => selected,
            Err(reject) => return SpokeResult::Reject(reject),
        };
        selected.adapter.list_rules(rule_refs).await
    }
}

// ── HostManifestPort — aggregated locally (contract §6, no round-trip) ────

#[async_trait]
impl HostManifestPort for MultiPeerRouter {
    /// Composed view: set-union of all CONNECTED (Established) peers'
    /// capabilities / roles / namespaces, the router's own `host_id`, NO
    /// `authority` (a composed view does not synthesize an authority scope),
    /// and `extensions.router.peers` listing the contributing peer ids in
    /// lexicographic UTF-8 byte order. For consumer introspection ONLY —
    /// never used for routing (§6 aggregation vs routing non-conflation).
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        let connected = self.connected_peers();
        let capabilities = union_of_strings(
            connected
                .iter()
                .map(|peer| peer.manifest.capabilities.as_slice()),
        );
        let roles = union_of_strings(connected.iter().map(|peer| peer.manifest.roles.as_slice()));
        let mut seen_namespaces: HashSet<&str> = HashSet::new();
        let namespaces: Vec<_> = connected
            .iter()
            .flat_map(|peer| peer.manifest.namespaces.iter())
            .filter(|declared| seen_namespaces.insert(declared.as_str()))
            .cloned()
            .collect();
        // Frozen §6: `tools[]` unions across connected peers, dedup by
        // `capability_id` (first occurrence wins), then lexicographic
        // `capability_id` order for stability — NOT first-seen order.
        let mut seen_tools: HashSet<&str> = HashSet::new();
        let mut tools: Vec<ToolDescriptor> = Vec::new();
        for peer in &connected {
            for descriptor in &peer.manifest.tools {
                if seen_tools.insert(descriptor.capability_id.as_str()) {
                    tools.push(descriptor.clone());
                }
            }
        }
        tools.sort_by(|a, b| a.capability_id.cmp(&b.capability_id));

        let mut router_extensions = Map::new();
        router_extensions.insert(
            "peers".to_string(),
            Value::Array(
                connected
                    .iter()
                    .map(|peer| Value::String(peer.peer_id.clone()))
                    .collect(),
            ),
        );
        let mut extensions = HashMap::new();
        extensions.insert(
            HostCapabilityManifestExtensionsKey::try_from("router")
                .expect("'router' matches the extensions key pattern"),
            router_extensions,
        );

        let composed = HostCapabilityManifest {
            schema_version: NonZeroU64::new(1).expect("1 is non-zero"),
            host_id: HostCapabilityManifestHostId::try_from(self.host_id.as_str())
                .expect("router host_id is non-empty by construction"),
            capabilities,
            roles,
            namespaces,
            authority: None,
            extensions,
            tools,
        };
        spoke_ok(composed)
    }

    /// Per-peer array of all connected peers' cached manifests (the `host`
    /// field from their authenticated hello, cached at registration),
    /// ordered lexicographically by `peer_id` (UTF-8 byte order). Zero
    /// connected peers is valid and returns `[]`.
    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        let connected = self.connected_peers();
        spoke_ok(connected.into_iter().map(|peer| peer.manifest).collect())
    }
}

/// Construct a multi-peer capability router (contract §8). No dial options —
/// the router starts with zero peers; consumers dial each peer's
/// `RemoteAdapter` themselves and register the established adapter.
pub fn connect_multi_peer_router(options: MultiPeerRouterOptions) -> MultiPeerRouter {
    MultiPeerRouter::new(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoke_schemas::host_capability_manifest::HostCapabilityManifestExtensionsKey;
    use tokio::task::JoinSet;

    // ── Delegate-call dummy values (routing tests never inspect payloads) ──

    fn dummy_entry() -> KnowledgeEntry {
        serde_json::from_value(json!({
            "schema_version": 1,
            "entry_id": "e1",
            "entry_type": "character",
            "canonical_name": "E1",
            "status": "provisional",
            "body": { "summary": "dummy" },
            "extensions": {},
        }))
        .expect("valid KnowledgeEntry")
    }

    fn dummy_relation() -> Relation {
        serde_json::from_value(json!({
            "schema_version": 1,
            "relation_id": "r1",
            "relation_type": "located_in",
            "from_id": "e1",
            "to_id": "e2",
            "extensions": {},
        }))
        .expect("valid Relation")
    }

    fn dummy_finding() -> Finding {
        serde_json::from_value(json!({
            "schema_version": 1,
            "finding_id": "f1",
            "severity": "warning",
            "status": "open",
            "title": "F1",
            "extensions": {},
        }))
        .expect("valid Finding")
    }

    fn dummy_rule() -> Rule {
        serde_json::from_value(json!({
            "schema_version": 1,
            "rule_id": "rule-1",
            "canonical_name": "Rule One",
            "kind": "rule",
            "extensions": {},
        }))
        .expect("valid Rule")
    }

    fn dummy_event() -> TimelineEvent {
        serde_json::from_value(json!({
            "schema_version": 1,
            "timeline_event_id": "ev1",
            "canonical_name": "Event One",
            "extensions": {},
        }))
        .expect("valid TimelineEvent")
    }

    fn dummy_scope() -> Scope {
        serde_json::from_value(json!({ "scope_id": "s1" })).expect("valid Scope")
    }

    // ── Manifest builders (schema-shaped; defaults to a baseline data-store
    //    peer — mirror of the TS `manifest` helper) ─────────────────────────

    fn manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
        manifest_with(host_id, capabilities, &["data-store"], &["toy_world"])
    }

    fn manifest_with(
        host_id: &str,
        capabilities: &[&str],
        roles: &[&str],
        namespaces: &[&str],
    ) -> HostCapabilityManifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": host_id,
            "roles": roles,
            "capabilities": capabilities,
            "namespaces": namespaces,
            "extensions": {},
        }))
        .expect("valid HostCapabilityManifest")
    }

    fn manifest_with_authority(host_id: &str, scope_key: &str) -> HostCapabilityManifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": host_id,
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline"],
            "namespaces": ["toy_world"],
            "authority": { "scope_key": scope_key },
            "extensions": {},
        }))
        .expect("valid HostCapabilityManifest")
    }

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

    // ── Tool descriptors (frozen §2: op === capability_id) ─────────────────

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

    // ── Test double for the per-peer adapter surface (`RoutedRemoteAdapter`)
    //    — a plain `state` / `remote_peer_id` / `remote_manifest` plus the ten
    //    async port methods that record the delegated method name and return
    //    `down_reject` when a mid-op failure is being simulated (§7.2).
    //    Parity with the TS `FakePeer`. ─────────────────────────────────────

    struct FakePeer {
        peer_id: String,
        manifest: Option<HostCapabilityManifest>,
        /// Peer session state — `Mutex` so tests can flip `Established` →
        /// `Closed` mid-test (dynamic peer-down, W-002) while the adapter
        /// surface keeps `&self` reads.
        state: Mutex<RemoteAdapterState>,
        /// Delegated port method names, in call order.
        calls: Mutex<Vec<String>>,
        /// When set, every delegate method returns this reject (peer-down sim).
        down_reject: Mutex<Option<SpokeReject>>,
    }

    impl FakePeer {
        fn new(
            peer_id: &str,
            manifest: HostCapabilityManifest,
            state: RemoteAdapterState,
        ) -> Arc<Self> {
            Arc::new(Self {
                peer_id: peer_id.to_string(),
                manifest: Some(manifest),
                state: Mutex::new(state),
                calls: Mutex::new(Vec::new()),
                down_reject: Mutex::new(None),
            })
        }

        fn without_manifest(peer_id: &str) -> Arc<Self> {
            Arc::new(Self {
                peer_id: peer_id.to_string(),
                manifest: None,
                state: Mutex::new(RemoteAdapterState::Established),
                calls: Mutex::new(Vec::new()),
                down_reject: Mutex::new(None),
            })
        }

        fn set_state(&self, state: RemoteAdapterState) {
            *self.state.lock().expect("state lock") = state;
        }

        fn set_down_reject(&self, reject: SpokeReject) {
            *self.down_reject.lock().expect("down reject lock") = Some(reject);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls lock").clone()
        }

        fn record_and_maybe_fail<T>(&self, method: &str) -> Option<SpokeResult<T>> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(method.to_string());
            self.down_reject
                .lock()
                .expect("down reject lock")
                .clone()
                .map(SpokeResult::Reject)
        }
    }

    #[async_trait]
    impl RoutedRemoteAdapter for FakePeer {
        fn state(&self) -> RemoteAdapterState {
            *self.state.lock().expect("state lock")
        }

        fn remote_peer_id(&self) -> Option<String> {
            Some(self.peer_id.clone())
        }

        fn remote_manifest(&self) -> Option<HostCapabilityManifest> {
            self.manifest.clone()
        }

        async fn get_knowledge_entry(&self, _entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.record_and_maybe_fail("getKnowledgeEntry")
                .unwrap_or_else(|| spoke_ok(dummy_entry()))
        }

        async fn put_knowledge_entry(
            &self,
            _entry: KnowledgeEntry,
            _expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            self.record_and_maybe_fail("putKnowledgeEntry")
                .unwrap_or_else(|| spoke_ok(dummy_entry()))
        }

        async fn get_relation(&self, _relation_id: &str) -> SpokeResult<Relation> {
            self.record_and_maybe_fail("getRelation")
                .unwrap_or_else(|| spoke_ok(dummy_relation()))
        }

        async fn put_relation(
            &self,
            _relation: Relation,
            _expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            self.record_and_maybe_fail("putRelation")
                .unwrap_or_else(|| spoke_ok(dummy_relation()))
        }

        async fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.record_and_maybe_fail("listKnowledgeEntries")
                .unwrap_or_else(|| spoke_ok(vec![dummy_entry()]))
        }

        async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.record_and_maybe_fail("listTimelineEvents")
                .unwrap_or_else(|| spoke_ok(vec![dummy_event()]))
        }

        async fn put_findings(&self, _findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.record_and_maybe_fail("putFindings")
                .unwrap_or_else(|| spoke_ok(vec![dummy_finding()]))
        }

        async fn list_rules(&self, _rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.record_and_maybe_fail("listRules")
                .unwrap_or_else(|| spoke_ok(vec![dummy_rule()]))
        }

        async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
            self.record_and_maybe_fail("getHostCapabilityManifest")
                .unwrap_or_else(|| {
                    spoke_ok(self.manifest.clone().expect("fake manifest is available"))
                })
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<HostCapabilityManifest>> {
            self.record_and_maybe_fail("listPeerHostCapabilityManifests")
                .unwrap_or_else(|| spoke_ok(Vec::new()))
        }

        /// Forward tool-invoke face (§6) — records the delegation.
        async fn invoke_tool(
            &self,
            _capability_id: &str,
            _arguments: Value,
        ) -> SpokeResult<Value> {
            self.record_and_maybe_fail("invokeTool")
                .unwrap_or_else(|| spoke_ok(json!({ "served_by": self.peer_id })))
        }
    }

    // ── Registry (§7.4) ────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_peer_stores_established_adapter_and_is_idempotent() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer = FakePeer::new(
            "peer-a",
            manifest("host-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );

        assert_eq!(
            router.register_peer(peer.clone()).expect("register"),
            "peer-a"
        );
        assert_eq!(router.list_peers(), vec!["peer-a".to_string()]);
        // Idempotent on peer_id: re-registering the same adapter returns the same id.
        assert_eq!(router.register_peer(peer).expect("re-register"), "peer-a");
        assert_eq!(router.list_peers(), vec!["peer-a".to_string()]);
        // A second adapter with the same peer_id replaces the stored one.
        let replacement = FakePeer::new(
            "peer-a",
            manifest("host-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        assert_eq!(
            router.register_peer(replacement).expect("replace"),
            "peer-a"
        );
        assert_eq!(router.list_peers(), vec!["peer-a".to_string()]);
    }

    #[tokio::test]
    async fn register_peer_errors_when_the_adapter_has_no_established_session() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        // No verified remote peer id (empty) — the handshaking adapter has no
        // established session.
        let handshaking = FakePeer::new(
            "",
            manifest("host-a", &["spoke-baseline"]),
            RemoteAdapterState::Handshaking,
        );
        assert!(matches!(
            router.register_peer(handshaking),
            Err(MultiPeerRouterError::NoPeerId)
        ));
        // Established state but no cached manifest — still unregisterable.
        let no_manifest = FakePeer::without_manifest("peer-a");
        assert!(matches!(
            router.register_peer(no_manifest),
            Err(MultiPeerRouterError::NoManifest)
        ));
        assert!(router.list_peers().is_empty());
    }

    #[tokio::test]
    async fn unregister_peer_is_a_noop_for_unknown_and_removes_registered() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        router
            .register_peer(FakePeer::new(
                "peer-a",
                manifest("host-a", &["spoke-baseline"]),
                RemoteAdapterState::Established,
            ))
            .expect("register");

        router.unregister_peer("never-registered");
        assert_eq!(router.list_peers(), vec!["peer-a".to_string()]);

        router.unregister_peer("peer-a");
        assert!(router.list_peers().is_empty());
    }

    // ── Selection (§3) — router surface ────────────────────────────────────

    #[tokio::test]
    async fn selects_the_single_peer_with_the_required_capability() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let baseline = FakePeer::new(
            "peer-baseline",
            manifest("h-baseline", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        let computable = FakePeer::new(
            "peer-computable",
            manifest("h-computable", &["l2-computable"]),
            RemoteAdapterState::Established,
        );
        router
            .register_peer(computable.clone())
            .expect("register computable");
        router
            .register_peer(baseline.clone())
            .expect("register baseline");

        let result = router.list_knowledge_entries(&dummy_scope()).await;

        assert!(result.is_ok());
        assert_eq!(baseline.calls(), vec!["listKnowledgeEntries".to_string()]);
        assert!(computable.calls().is_empty());
    }

    #[tokio::test]
    async fn rejects_with_the_locked_no_capable_peer_reject_when_no_peer_has_the_required_capability(
    ) {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let computable = FakePeer::new(
            "peer-computable",
            manifest("h-computable", &["l2-computable"]),
            RemoteAdapterState::Established,
        );
        router.register_peer(computable.clone()).expect("register");

        let result = router.list_knowledge_entries(&dummy_scope()).await;

        match result {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("wire_code"))
                        .and_then(|code| code.as_str()),
                    Some("no_capable_peer")
                );
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(|kind| kind.as_str()),
                    Some("no_capable_peer")
                );
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("op"))
                        .and_then(|op| op.as_str()),
                    Some("port.scope.list_knowledge_entries")
                );
            }
            SpokeResult::Ok(_) => panic!("no capable peer must reject"),
        }
        // Terminal: no peer delegate ran (no wrong-peer fallback).
        assert!(computable.calls().is_empty());
    }

    #[tokio::test]
    async fn breaks_ties_deterministically_on_the_lowest_peer_id() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer_b = FakePeer::new(
            "peer-bbb",
            manifest("h-b", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        let peer_a = FakePeer::new(
            "peer-aaa",
            manifest("h-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        router.register_peer(peer_b.clone()).expect("register b");
        router.register_peer(peer_a.clone()).expect("register a");

        let result = router.get_knowledge_entry("e1").await;

        assert!(result.is_ok());
        assert_eq!(peer_a.calls(), vec!["getKnowledgeEntry".to_string()]);
        assert!(peer_b.calls().is_empty());
    }

    #[tokio::test]
    async fn excludes_closed_and_handshaking_peers_from_the_candidate_set() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let closed = FakePeer::new(
            "peer-closed",
            manifest("h-closed", &["spoke-baseline"]),
            RemoteAdapterState::Closed,
        );
        let handshaking = FakePeer::new(
            "peer-handshaking",
            manifest("h-handshaking", &["spoke-baseline"]),
            RemoteAdapterState::Handshaking,
        );
        let established = FakePeer::new(
            "peer-established",
            manifest("h-established", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        router
            .register_peer(closed.clone())
            .expect("register closed");
        router
            .register_peer(handshaking.clone())
            .expect("register handshaking");
        router
            .register_peer(established.clone())
            .expect("register established");

        let result = router.list_knowledge_entries(&dummy_scope()).await;

        assert!(result.is_ok());
        assert_eq!(
            established.calls(),
            vec!["listKnowledgeEntries".to_string()]
        );
        assert!(closed.calls().is_empty());
        assert!(handshaking.calls().is_empty());
    }

    // ── Pure selection — unknown-op capability gate (QC2 S-1) ─────────────

    #[tokio::test]
    async fn rejects_unknown_ops_with_the_locked_reject_instead_of_skipping_the_capability_gate()
    {
        // S-1: an op outside the locked §2 mapping table must NOT fall
        // through to an ungated selection — even a capable peer is rejected
        // with the terminal §5 no_capable_peer shape.
        let peer = SelectablePeer {
            peer_id: "peer-a".to_string(),
            manifest: manifest("h-a", &["spoke-baseline"]),
        };

        let selection = select_peer_for_op(&[peer], "product.op.unknown", &json!({}));

        match selection {
            SpokeResult::Ok(_) => panic!("unknown op must not select a peer"),
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(|kind| kind.as_str()),
                    Some("no_capable_peer")
                );
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("op"))
                        .and_then(|op| op.as_str()),
                    Some("product.op.unknown")
                );
            }
        }
    }

    // ── Dynamic peer-down (§7.4) — reactive exclusion on the next call ────

    #[tokio::test]
    async fn reroutes_to_the_surviving_peer_when_the_selected_peer_leaves_established_between_calls()
    {
        // W-002: two Established peers; the first call routes to the
        // tie-break winner (peer-a); the winner's session drops to Closed
        // WITHOUT unregister; the next call excludes it from the candidate
        // set and routes to peer-b — reactive exclusion, no proactive
        // eviction (the registry still lists peer-a).
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer_a = FakePeer::new(
            "peer-a",
            manifest("h-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        let peer_b = FakePeer::new(
            "peer-b",
            manifest("h-b", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        router.register_peer(peer_b.clone()).expect("register b");
        router.register_peer(peer_a.clone()).expect("register a");

        let first = router.list_knowledge_entries(&dummy_scope()).await;
        assert!(first.is_ok());
        assert_eq!(peer_a.calls(), vec!["listKnowledgeEntries".to_string()]);
        assert!(peer_b.calls().is_empty());

        // The selected peer's session closes mid-session (no unregister).
        peer_a.set_state(RemoteAdapterState::Closed);
        assert_eq!(
            router.list_peers(),
            vec!["peer-b".to_string(), "peer-a".to_string()],
            "Closed peer stays registered — the router never proactively evicts (§7.4)"
        );

        let second = router.list_knowledge_entries(&dummy_scope()).await;
        assert!(second.is_ok());
        assert_eq!(
            peer_b.calls(),
            vec!["listKnowledgeEntries".to_string()],
            "the next call routes to the surviving peer"
        );
        assert_eq!(
            peer_a.calls(),
            vec!["listKnowledgeEntries".to_string()],
            "the Closed peer receives no further delegates"
        );
    }

    #[tokio::test]
    async fn rejects_no_capable_peer_when_every_peer_leaves_established_between_calls() {
        // W-002 terminal: when ALL Established peers drop out between calls,
        // the next selection rejects with the locked §5 reject — no delegate
        // runs, no wrong-peer fallback.
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer_a = FakePeer::new(
            "peer-a",
            manifest("h-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        let peer_b = FakePeer::new(
            "peer-b",
            manifest("h-b", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        router.register_peer(peer_b.clone()).expect("register b");
        router.register_peer(peer_a.clone()).expect("register a");

        // First call succeeds on the tie-break winner.
        assert!(router.list_knowledge_entries(&dummy_scope()).await.is_ok());
        assert_eq!(peer_a.calls(), vec!["listKnowledgeEntries".to_string()]);
        assert!(peer_b.calls().is_empty());

        peer_a.set_state(RemoteAdapterState::Closed);
        peer_b.set_state(RemoteAdapterState::Closed);

        let second = router.list_knowledge_entries(&dummy_scope()).await;
        match second {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(|kind| kind.as_str()),
                    Some("no_capable_peer")
                );
            }
            SpokeResult::Ok(_) => panic!("no Established peer must reject"),
        }
        // No delegate ran on the second call.
        assert_eq!(peer_a.calls(), vec!["listKnowledgeEntries".to_string()]);
        assert!(peer_b.calls().is_empty());
    }

    // ── Concurrent registry stress (W-001) ────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_register_select_and_unregister_complete_without_deadlock_or_corruption()
    {
        // W-001: N tasks register peers (each id twice — the idempotent
        // replace path) while M tasks run selections and K tasks unregister
        // pre-seeded peers, all on a multi-threaded runtime. The registry
        // mutexes (`peers` → `registration_order`) and the two-lock selection
        // path are exercised under contention. Bounded deadline: a deadlock
        // would hang past it and fail the test.
        let router = Arc::new(connect_multi_peer_router(MultiPeerRouterOptions::default()));

        // Pre-seed four peers so early selections always have a candidate.
        let seeds: Vec<Arc<FakePeer>> = (0..4)
            .map(|i| {
                FakePeer::new(
                    &format!("seed-{i:02}"),
                    manifest(&format!("h-seed-{i:02}"), &["spoke-baseline"]),
                    RemoteAdapterState::Established,
                )
            })
            .collect();
        for seed in &seeds {
            router
                .register_peer(seed.clone())
                .expect("pre-seed registers");
        }

        const PEER_COUNT: usize = 12;
        let peers: Vec<Arc<FakePeer>> = (0..PEER_COUNT)
            .map(|i| {
                FakePeer::new(
                    &format!("peer-{i:02}"),
                    manifest(&format!("h-{i:02}"), &["spoke-baseline"]),
                    RemoteAdapterState::Established,
                )
            })
            .collect();

        let mut registers = JoinSet::new();
        for (i, peer) in peers.iter().enumerate() {
            let router = router.clone();
            let peer = peer.clone();
            registers.spawn(async move {
                // Register twice: the second exercises the idempotent replace
                // path while other tasks hold the registry lock.
                let first = router.register_peer(peer.clone());
                let second = router.register_peer(peer);
                (i, first, second)
            });
        }

        let mut selects = JoinSet::new();
        for i in 0..PEER_COUNT {
            let router = router.clone();
            selects.spawn(async move {
                let result = router.list_knowledge_entries(&dummy_scope()).await;
                (i, result)
            });
        }

        let mut unregisters = JoinSet::new();
        for seed in seeds.iter() {
            let router = router.clone();
            let peer_id = seed.peer_id.clone();
            unregisters.spawn(async move {
                router.unregister_peer(&peer_id);
            });
        }

        // All three task sets run concurrently; await everything under one
        // bounded deadline — a deadlock or a hung task fails the timeout.
        let (registers_outcome, selects_outcome, unregisters_outcome) = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            async {
                let mut registers_done: Vec<(
                    usize,
                    Result<String, MultiPeerRouterError>,
                    Result<String, MultiPeerRouterError>,
                )> = Vec::new();
                while let Some(joined) = registers.join_next().await {
                    registers_done.push(joined.expect("register task must not panic"));
                }
                let mut selects_done: Vec<(usize, SpokeResult<Vec<KnowledgeEntry>>)> = Vec::new();
                while let Some(joined) = selects.join_next().await {
                    selects_done.push(joined.expect("select task must not panic"));
                }
                let mut unregisters_done = 0usize;
                while let Some(joined) = unregisters.join_next().await {
                    joined.expect("unregister task must not panic");
                    unregisters_done += 1;
                }
                (registers_done, selects_done, unregisters_done)
            },
        )
        .await
        .expect("concurrent register/select/unregister must complete within the deadline (no deadlock)");

        // Every register completed with its own peer id (idempotent).
        for (i, first, second) in &registers_outcome {
            let expected = format!("peer-{i:02}");
            assert_eq!(first.as_ref().expect("first register"), &expected);
            assert_eq!(second.as_ref().expect("second register"), &expected);
        }

        // No corruption: exactly the concurrently registered ids remain (the
        // pre-seeded peers were unregistered), no duplicates, no seeds.
        let mut listed = router.list_peers();
        listed.sort();
        let mut expected: Vec<String> = (0..PEER_COUNT)
            .map(|i| format!("peer-{i:02}"))
            .collect();
        expected.sort();
        assert_eq!(
            listed, expected,
            "registry holds exactly the registered ids, no duplicates, no seeds"
        );
        assert_eq!(unregisters_outcome, 4);

        // Every selection completed; each outcome is a successful delegate or
        // the locked no_capable_peer reject (a selection racing a seed
        // unregister may hit the defensive "no longer registered" path).
        for (_, result) in &selects_outcome {
            match result {
                SpokeResult::Ok(_) => {}
                SpokeResult::Reject(reject) => {
                    assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
                    assert_eq!(
                        reject
                            .details
                            .as_ref()
                            .and_then(|details| details.get("kind"))
                            .and_then(|kind| kind.as_str()),
                        Some("no_capable_peer")
                    );
                }
            }
        }
    }

    // ── Pure selection — namespace filter (§2/§3) ──────────────────────────

    #[tokio::test]
    async fn namespace_filter_excludes_peers_that_do_not_advertise_the_request_namespace() {
        let peer_alpha = SelectablePeer {
            peer_id: "peer-alpha".to_string(),
            manifest: manifest_with("h-alpha", &["spoke-baseline"], &["data-store"], &["alpha"]),
        };
        let peer_beta = SelectablePeer {
            peer_id: "peer-beta".to_string(),
            manifest: manifest_with("h-beta", &["spoke-baseline"], &["data-store"], &["beta"]),
        };

        let selection = select_peer_for_op(
            &[peer_alpha, peer_beta],
            "port.scope.list_knowledge_entries",
            &json!({ "scope": { "scope_id": "s1", "namespace": "beta" } }),
        );

        match selection {
            SpokeResult::Ok(selected) => assert_eq!(selected.peer_id, "peer-beta"),
            SpokeResult::Reject(reject) => panic!("must select peer-beta: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn namespace_filter_is_skipped_when_the_request_carries_no_namespace() {
        let peer_alpha = SelectablePeer {
            peer_id: "peer-alpha".to_string(),
            manifest: manifest_with("h-alpha", &["spoke-baseline"], &["data-store"], &["alpha"]),
        };
        let peer_beta = SelectablePeer {
            peer_id: "peer-beta".to_string(),
            manifest: manifest_with("h-beta", &["spoke-baseline"], &["data-store"], &["beta"]),
        };

        let selection = select_peer_for_op(
            &[peer_alpha, peer_beta],
            "port.scope.list_knowledge_entries",
            &json!({ "scope": { "scope_id": "s1" } }),
        );

        match selection {
            SpokeResult::Ok(selected) => assert_eq!(selected.peer_id, "peer-alpha"), // tie-break
            SpokeResult::Reject(reject) => panic!("must select peer-alpha: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn namespace_filter_never_wildcard_expands_an_asterisk_request() {
        // Contract §2: no wildcard namespace in v1 — a literal `"*"` is the
        // literal string, never a match-all. The wire schema already rejects
        // a `"*"` namespace inside a manifest (pattern ^[a-z][a-z0-9_-]*$,
        // enforced on deserialize and every constructor — the tuple field is
        // private), so a peer can never declare the wildcard; the lock holds
        // structurally in Rust. The selection comparison is exact string
        // equality, and a REQUEST namespace of `"*"` must not match a peer
        // whose namespaces omit it.
        let peer_alpha = SelectablePeer {
            peer_id: "peer-alpha".to_string(),
            manifest: manifest_with("h-alpha", &["spoke-baseline"], &["data-store"], &["alpha"]),
        };

        let selection = select_peer_for_op(
            &[peer_alpha],
            "port.scope.list_knowledge_entries",
            &json!({ "scope": { "scope_id": "s1", "namespace": "*" } }),
        );

        match selection {
            SpokeResult::Ok(_) => panic!("literal '*' request namespace must not match 'alpha'"),
            SpokeResult::Reject(reject) => assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(|kind| kind.as_str()),
                Some("no_capable_peer")
            ),
        }
    }

    // ── Pure selection — role preference (§3 step 5) ───────────────────────

    #[tokio::test]
    async fn role_preference_prefers_the_peer_whose_roles_include_the_preferred_role() {
        let plain = SelectablePeer {
            peer_id: "peer-a".to_string(),
            manifest: manifest("h-a", &["spoke-baseline"]),
        };
        let checker = SelectablePeer {
            peer_id: "peer-z".to_string(),
            manifest: manifest_with(
                "h-z",
                &["spoke-baseline"],
                &["data-store", "checker"],
                &["toy_world"],
            ),
        };

        let selection = select_peer_for_op(&[plain, checker], "check", &json!({}));

        match selection {
            SpokeResult::Ok(selected) => {
                // Role partition beats tie-break: peer-z is chosen despite the
                // higher peer_id.
                assert_eq!(selected.peer_id, "peer-z");
            }
            SpokeResult::Reject(reject) => panic!("must select peer-z: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn role_preference_falls_back_to_the_role_unmatched_partition_when_none_has_the_preferred_role(
    ) {
        let plain = SelectablePeer {
            peer_id: "peer-a".to_string(),
            manifest: manifest("h-a", &["spoke-baseline"]),
        };
        let checker = SelectablePeer {
            peer_id: "peer-z".to_string(),
            manifest: manifest_with(
                "h-z",
                &["spoke-baseline"],
                &["data-store", "checker"],
                &["toy_world"],
            ),
        };

        // Neither peer has "assembler" → tie-break decides (soft preference).
        let selection = select_peer_for_op(&[plain, checker], "assemble", &json!({}));

        match selection {
            SpokeResult::Ok(selected) => assert_eq!(selected.peer_id, "peer-a"),
            SpokeResult::Reject(reject) => panic!("must select peer-a: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn no_role_preference_applies_to_port_baseline_ops() {
        let plain = SelectablePeer {
            peer_id: "peer-a".to_string(),
            manifest: manifest("h-a", &["spoke-baseline"]),
        };
        let checker = SelectablePeer {
            peer_id: "peer-z".to_string(),
            manifest: manifest_with(
                "h-z",
                &["spoke-baseline"],
                &["data-store", "checker"],
                &["toy_world"],
            ),
        };

        let selection = select_peer_for_op(&[plain, checker], "port.knowledge.put", &json!({}));

        match selection {
            SpokeResult::Ok(selected) => {
                // Tie-break wins; the checker role is ignored for port.* ops.
                assert_eq!(selected.peer_id, "peer-a");
            }
            SpokeResult::Reject(reject) => panic!("must select peer-a: {reject:?}"),
        }
    }

    // ── Pure selection — authority filter (§3 step 4) ──────────────────────

    #[tokio::test]
    async fn authority_filter_excludes_peers_whose_declared_scope_key_mismatches_the_request() {
        let matching = SelectablePeer {
            peer_id: "peer-match".to_string(),
            manifest: manifest_with_authority("h-match", "scope-K"),
        };
        let mismatched = SelectablePeer {
            peer_id: "peer-mismatch".to_string(),
            manifest: manifest_with_authority("h-mismatch", "scope-Z"),
        };

        let selection = select_peer_for_op(
            &[mismatched, matching],
            "port.scope.list_knowledge_entries",
            &json!({ "scope_key": "scope-K" }),
        );

        match selection {
            SpokeResult::Ok(selected) => assert_eq!(selected.peer_id, "peer-match"),
            SpokeResult::Reject(reject) => panic!("must select peer-match: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn authority_filter_is_skipped_when_only_one_side_declares_a_scope_key() {
        let mismatched = SelectablePeer {
            peer_id: "peer-mismatch".to_string(),
            manifest: manifest_with_authority("h-mismatch", "scope-Z"),
        };
        let undeclared = SelectablePeer {
            peer_id: "peer-a".to_string(),
            manifest: manifest("h-a", &["spoke-baseline"]),
        };

        let selection = select_peer_for_op(
            &[mismatched, undeclared],
            "port.scope.list_knowledge_entries",
            &json!({ "scope_key": "scope-K" }),
        );

        match selection {
            SpokeResult::Ok(selected) => {
                // peer-a declares nothing → filter skipped for it;
                // peer-mismatch excluded.
                assert_eq!(selected.peer_id, "peer-a");
            }
            SpokeResult::Reject(reject) => panic!("must select peer-a: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn authority_filter_is_skipped_when_the_request_carries_no_scope_key() {
        let matching = SelectablePeer {
            peer_id: "peer-match".to_string(),
            manifest: manifest_with_authority("h-match", "scope-K"),
        };
        let mismatched = SelectablePeer {
            peer_id: "peer-mismatch".to_string(),
            manifest: manifest_with_authority("h-mismatch", "scope-Z"),
        };

        let selection = select_peer_for_op(
            &[mismatched, matching],
            "port.scope.list_knowledge_entries",
            &json!({ "scope": { "scope_id": "s1" } }),
        );

        match selection {
            SpokeResult::Ok(selected) => assert_eq!(selected.peer_id, "peer-match"), // tie-break
            SpokeResult::Reject(reject) => panic!("must select peer-match: {reject:?}"),
        }
    }

    // ── Pure selection — tie-break byte order (§4) ─────────────────────────

    #[tokio::test]
    async fn tie_break_orders_peer_ids_by_utf8_byte_order_not_code_units() {
        // U+E000 encodes as UTF-8 EE 80 80; U+10000 encodes as F0 90 80 80.
        // UTF-16 code units: 0xE000 > 0xD800 (surrogate), so code-unit order
        // would pick U+10000 first; UTF-8 byte order (Rust `String` ordering)
        // picks U+E000 first.
        let bmp = SelectablePeer {
            peer_id: "\u{E000}".to_string(),
            manifest: manifest("h-bmp", &["spoke-baseline"]),
        };
        let astral = SelectablePeer {
            peer_id: "\u{10000}".to_string(),
            manifest: manifest("h-astral", &["spoke-baseline"]),
        };

        let selection = select_peer_for_op(&[astral, bmp], "port.knowledge.get", &json!({}));

        match selection {
            SpokeResult::Ok(selected) => assert_eq!(selected.peer_id, "\u{E000}"),
            SpokeResult::Reject(reject) => panic!("must select U+E000: {reject:?}"),
        }
    }

    // ── Failure policy (§7.2) ──────────────────────────────────────────────

    #[tokio::test]
    async fn returns_the_selected_peers_underlying_reject_as_is_without_alternate_retry() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer_a = FakePeer::new(
            "peer-a",
            manifest("h-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        let peer_b = FakePeer::new(
            "peer-b",
            manifest("h-b", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        router.register_peer(peer_b.clone()).expect("register b");
        router.register_peer(peer_a.clone()).expect("register a");
        // peer-a is selected (tie-break) and its session dies mid-op.
        let down_reject = SpokeReject {
            code: SpokeRejectCode::InternalError,
            message: "transport loss".to_string(),
            details: Some(Map::from_iter([(
                "kind".to_string(),
                Value::String("transport".to_string()),
            )])),
        };
        peer_a.set_down_reject(down_reject.clone());

        let result = router.list_knowledge_entries(&dummy_scope()).await;

        match result {
            SpokeResult::Reject(reject) => assert_eq!(reject, down_reject),
            SpokeResult::Ok(_) => panic!("selected-peer failure must reject"),
        }
        // No retry on the alternate capable peer.
        assert!(peer_b.calls().is_empty());
    }

    #[tokio::test]
    async fn does_not_remap_an_envelope_auth_failure_kind_to_no_capable_peer() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer_a = FakePeer::new(
            "peer-a",
            manifest("h-a", &["spoke-baseline"]),
            RemoteAdapterState::Established,
        );
        router.register_peer(peer_a.clone()).expect("register");
        peer_a.set_down_reject(SpokeReject {
            code: SpokeRejectCode::InternalError,
            message: "unauthenticated envelope".to_string(),
            details: Some(Map::from_iter([(
                "kind".to_string(),
                Value::String("envelope_auth_missing".to_string()),
            )])),
        });

        let result = router.get_knowledge_entry("e1").await;

        match result {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InternalError);
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(|kind| kind.as_str()),
                    Some("envelope_auth_missing")
                );
                assert!(reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .is_none());
            }
            SpokeResult::Ok(_) => panic!("envelope-auth failure must reject"),
        }
    }

    // ── HostManifest aggregation (§6) ──────────────────────────────────────

    #[tokio::test]
    async fn treats_an_empty_host_id_as_unset_defaulting_to_multi_peer_router() {
        // §8 constructor options parity with TS `options.hostId || default`:
        // an empty configured host id is treated as unset.
        let router = connect_multi_peer_router(MultiPeerRouterOptions {
            host_id: Some(String::new()),
        });

        let result = router.get_host_capability_manifest().await;

        match result {
            SpokeResult::Ok(composed) => {
                assert_eq!(composed.host_id.as_str(), "multi-peer-router");
            }
            SpokeResult::Reject(reject) => panic!("composed view must succeed: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn composes_the_union_of_connected_peers_with_the_routers_own_host_id() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions {
            host_id: Some("router-own".to_string()),
        });
        router
            .register_peer(FakePeer::new(
                "peer-a",
                manifest_with("h-a", &["spoke-baseline"], &["data-store"], &["alpha"]),
                RemoteAdapterState::Established,
            ))
            .expect("register a");
        router
            .register_peer(FakePeer::new(
                "peer-b",
                manifest_with(
                    "h-b",
                    &["spoke-baseline", "l2-computable"],
                    &["data-store", "checker"],
                    &["alpha", "beta"],
                ),
                RemoteAdapterState::Established,
            ))
            .expect("register b");

        let result = router.get_host_capability_manifest().await;

        match result {
            SpokeResult::Ok(composed) => {
                assert_eq!(composed.host_id.as_str(), "router-own");
                assert_eq!(composed.schema_version.get(), 1);
                let mut capabilities = composed.capabilities.clone();
                capabilities.sort();
                assert_eq!(capabilities, vec!["l2-computable", "spoke-baseline"]);
                let mut roles = composed.roles.clone();
                roles.sort();
                assert_eq!(roles, vec!["checker", "data-store"]);
                let mut namespaces: Vec<&str> =
                    composed.namespaces.iter().map(|ns| ns.as_str()).collect();
                namespaces.sort();
                assert_eq!(namespaces, vec!["alpha", "beta"]);
                // §6: authority.scope_key omitted; extensions.router.peers sorted.
                assert!(composed.authority.is_none());
                let router_ext = composed
                    .extensions
                    .get(&HostCapabilityManifestExtensionsKey::try_from("router").expect("key"))
                    .expect("router extensions");
                let peers = router_ext
                    .get("peers")
                    .and_then(Value::as_array)
                    .expect("peers array");
                let peer_ids: Vec<&str> = peers
                    .iter()
                    .map(|value| value.as_str().expect("peer id string"))
                    .collect();
                assert_eq!(peer_ids, vec!["peer-a", "peer-b"]);
            }
            SpokeResult::Reject(reject) => panic!("composed view must succeed: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn lists_per_peer_manifests_sorted_by_peer_id() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        router
            .register_peer(FakePeer::new(
                "peer-b",
                manifest_with("h-b", &["spoke-baseline"], &["data-store"], &["beta"]),
                RemoteAdapterState::Established,
            ))
            .expect("register b");
        router
            .register_peer(FakePeer::new(
                "peer-a",
                manifest_with("h-a", &["spoke-baseline"], &["data-store"], &["alpha"]),
                RemoteAdapterState::Established,
            ))
            .expect("register a");

        let result = router.list_peer_host_capability_manifests().await;

        match result {
            SpokeResult::Ok(manifests) => {
                let host_ids: Vec<&str> = manifests
                    .iter()
                    .map(|manifest| manifest.host_id.as_str())
                    .collect();
                assert_eq!(host_ids, vec!["h-a", "h-b"]);
            }
            SpokeResult::Reject(reject) => panic!("per-peer list must succeed: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn zero_peers_returns_empty_list_and_empty_unions() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions {
            host_id: Some("router-alone".to_string()),
        });

        let peers_result = router.list_peer_host_capability_manifests().await;
        match peers_result {
            SpokeResult::Ok(manifests) => assert!(manifests.is_empty()),
            SpokeResult::Reject(reject) => panic!("empty per-peer list must succeed: {reject:?}"),
        }

        let composed_result = router.get_host_capability_manifest().await;
        match composed_result {
            SpokeResult::Ok(composed) => {
                assert_eq!(composed.host_id.as_str(), "router-alone");
                assert!(composed.capabilities.is_empty());
                assert!(composed.roles.is_empty());
                assert!(composed.namespaces.is_empty());
                assert!(composed.authority.is_none());
                let router_ext = composed
                    .extensions
                    .get(&HostCapabilityManifestExtensionsKey::try_from("router").expect("key"))
                    .expect("router extensions");
                let peers = router_ext
                    .get("peers")
                    .and_then(Value::as_array)
                    .expect("peers array");
                assert!(peers.is_empty());
            }
            SpokeResult::Reject(reject) => panic!("empty composed view must succeed: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn excludes_non_established_registered_peers_from_the_composed_view() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions {
            host_id: Some("router-only-live".to_string()),
        });
        router
            .register_peer(FakePeer::new(
                "peer-closed",
                manifest_with(
                    "h-closed",
                    &["archive-scan"],
                    &["ghost"],
                    &["zeta"],
                ),
                RemoteAdapterState::Closed,
            ))
            .expect("register closed");
        router
            .register_peer(FakePeer::new(
                "peer-live",
                manifest_with(
                    "h-live",
                    &["spoke-baseline", "l2-computable"],
                    &["data-store", "checker"],
                    &["alpha", "beta"],
                ),
                RemoteAdapterState::Established,
            ))
            .expect("register live");

        // Per-peer array: only the Established peer's cached manifest.
        let per_peer = router.list_peer_host_capability_manifests().await;
        match per_peer {
            SpokeResult::Ok(manifests) => {
                let host_ids: Vec<&str> = manifests
                    .iter()
                    .map(|manifest| manifest.host_id.as_str())
                    .collect();
                assert_eq!(host_ids, vec!["h-live"]);
            }
            SpokeResult::Reject(reject) => panic!("per-peer list must succeed: {reject:?}"),
        }

        // Composed view (§6 "connected peers"): the Closed peer's unique
        // unions are absent — only the Established peer contributes, and
        // extensions.router.peers lists only it.
        let composed = router.get_host_capability_manifest().await;
        match composed {
            SpokeResult::Ok(composed) => {
                assert_eq!(composed.host_id.as_str(), "router-only-live");
                let mut capabilities = composed.capabilities.clone();
                capabilities.sort();
                assert_eq!(capabilities, vec!["l2-computable", "spoke-baseline"]);
                let mut roles = composed.roles.clone();
                roles.sort();
                assert_eq!(roles, vec!["checker", "data-store"]);
                let mut namespaces: Vec<&str> =
                    composed.namespaces.iter().map(|ns| ns.as_str()).collect();
                namespaces.sort();
                assert_eq!(namespaces, vec!["alpha", "beta"]);
                let router_ext = composed
                    .extensions
                    .get(&HostCapabilityManifestExtensionsKey::try_from("router").expect("key"))
                    .expect("router extensions");
                let peers = router_ext
                    .get("peers")
                    .and_then(Value::as_array)
                    .expect("peers array");
                let peer_ids: Vec<&str> = peers
                    .iter()
                    .map(|value| value.as_str().expect("peer id string"))
                    .collect();
                assert_eq!(peer_ids, vec!["peer-live"]);
            }
            SpokeResult::Reject(reject) => panic!("composed view must succeed: {reject:?}"),
        }
    }

    #[tokio::test]
    async fn composed_view_unions_tools_deduped_by_capability_id_in_lexicographic_order() {
        // Frozen §6: `tools[]` unions across connected peers, dedup by
        // `capability_id` (tools.echo.echo is shared), lexicographic order
        // for stability — NOT first-seen registration order.
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        router
            .register_peer(FakePeer::new(
                "peer-a",
                manifest_with_tools(
                    "h-a",
                    &["spoke-baseline", "tools.math.add", "tools.echo.echo"],
                    &["math", "echo"],
                    json!([add_descriptor(), echo_descriptor()]),
                ),
                RemoteAdapterState::Established,
            ))
            .expect("register a");
        router
            .register_peer(FakePeer::new(
                "peer-b",
                manifest_with_tools(
                    "h-b",
                    &["spoke-baseline", "tools.echo.echo", "tools.echo.boom"],
                    &["echo"],
                    json!([echo_descriptor(), boom_descriptor()]),
                ),
                RemoteAdapterState::Established,
            ))
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
    }

    // ── Tool routing (frozen §6) ───────────────────────────────────────────

    #[tokio::test]
    async fn invoke_tool_routes_to_the_peer_whose_manifest_offers_the_exact_tool_capability() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let add_peer = FakePeer::new(
            "peer-add",
            manifest_with_tools(
                "h-add",
                &["spoke-baseline", "tools.math.add"],
                &["math"],
                json!([add_descriptor()]),
            ),
            RemoteAdapterState::Established,
        );
        let echo_peer = FakePeer::new(
            "peer-echo",
            manifest_with_tools(
                "h-echo",
                &["spoke-baseline", "tools.echo.echo"],
                &["echo"],
                json!([echo_descriptor()]),
            ),
            RemoteAdapterState::Established,
        );
        router
            .register_peer(add_peer.clone())
            .expect("register add");
        router
            .register_peer(echo_peer.clone())
            .expect("register echo");

        // tools.math.add is advertised only by the add peer.
        let add = router
            .invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 }))
            .await;
        assert!(add.is_ok(), "add must route to the add peer: {add:?}");
        assert_eq!(add_peer.calls(), vec!["invokeTool"]);
        assert!(echo_peer.calls().is_empty());

        // tools.echo.echo is advertised only by the echo peer.
        let echo = router
            .invoke_tool("tools.echo.echo", json!({ "v": 1 }))
            .await;
        assert!(echo.is_ok(), "echo must route to the echo peer: {echo:?}");
        assert_eq!(echo_peer.calls(), vec!["invokeTool"]);
        assert_eq!(add_peer.calls(), vec!["invokeTool"]);
    }

    #[tokio::test]
    async fn invoke_tool_rejects_with_no_capable_peer_when_no_peer_offers_the_tool() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let add_peer = FakePeer::new(
            "peer-add",
            manifest_with_tools(
                "h-add",
                &["spoke-baseline", "tools.math.add"],
                &["math"],
                json!([add_descriptor()]),
            ),
            RemoteAdapterState::Established,
        );
        router
            .register_peer(add_peer.clone())
            .expect("register add");

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
        // Terminal: no delegate ran (no wrong-peer fallback).
        assert!(add_peer.calls().is_empty());
    }

    #[tokio::test]
    async fn invoke_tool_breaks_ties_on_the_lowest_peer_id_when_both_peers_offer_the_tool() {
        let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
        let peer_b = FakePeer::new(
            "peer-bbb",
            manifest_with_tools(
                "h-b",
                &["spoke-baseline", "tools.math.add"],
                &["math"],
                json!([add_descriptor()]),
            ),
            RemoteAdapterState::Established,
        );
        let peer_a = FakePeer::new(
            "peer-aaa",
            manifest_with_tools(
                "h-a",
                &["spoke-baseline", "tools.math.add"],
                &["math"],
                json!([add_descriptor()]),
            ),
            RemoteAdapterState::Established,
        );
        router.register_peer(peer_b.clone()).expect("register b");
        router.register_peer(peer_a.clone()).expect("register a");

        let result = router
            .invoke_tool("tools.math.add", json!({ "a": 1, "b": 2 }))
            .await;
        assert!(result.is_ok(), "tie-break invoke must route: {result:?}");
        // §4: lowest peer_id (UTF-8 byte order) wins.
        assert_eq!(peer_a.calls(), vec!["invokeTool"]);
        assert!(peer_b.calls().is_empty());
    }
}
