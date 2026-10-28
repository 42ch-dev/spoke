//! Capability-sliced adapter port contracts for injection orchestration.
//! Async `SpokeResult` surface — port methods return awaitable results; the
//! library stays I/O-free and only awaits injected ports.
//!
//! Port traits use `#[async_trait]` (Send futures) so the dyn probes
//! ([`ComputablePorts::as_computable`], [`ForkPorts::as_fork_timeline`])
//! stay object-safe. No runtime is required by this crate itself.

use async_trait::async_trait;
use crate::result::SpokeResult;
use serde_json::Value;
use spoke_schemas::{
    ComputeRequest, ComputeResponse, ExtractRequest, Finding, HostCapabilityManifest,
    KnowledgeEntry, MemoryCandidatesDismissRequest, MemoryCandidatesDismissResponse,
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
    MemorySubjectsGetResponse, Principal, ProjectRequest, ProjectResponse, Relation, Rule, Scope,
    TimelineEvent,
};

/// Knowledge entry persistence — get / put by entry id.
#[async_trait]
pub trait KnowledgeEntryPort {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry>;
    /// Persist a KnowledgeEntry with optimistic concurrency control.
    ///
    /// Adapters MUST treat `expected_base_revision` as the store’s required current
    /// revision before accepting the write (conditional put / OCC / CAS).
    /// `None` means the entry must be absent (create). A non-null value means the
    /// store’s current revision for `entry.entry_id` MUST equal
    /// `expected_base_revision`; otherwise reject with `STORED_REVISION_STALE` or
    /// `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
    /// in the adapter; the library stays I/O-free.
    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry>;
}

/// Relation persistence — get / put by relation id.
#[async_trait]
pub trait RelationPort {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation>;
    /// Persist a Relation with optimistic concurrency control.
    ///
    /// Adapters MUST treat `expected_base_revision` as the store’s required current
    /// revision before accepting the write (conditional put / OCC / CAS).
    /// `None` means the relation must be absent (create). A non-null value means the
    /// store’s current revision for `relation.relation_id` MUST equal
    /// `expected_base_revision`; otherwise reject with `STORED_REVISION_STALE` or
    /// `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
    /// in the adapter; the library stays I/O-free.
    ///
    /// Revision assignment is adapter-owned: on create
    /// (`expected_base_revision` `None`) the adapter MUST seed `revision = 1`;
    /// on an accepted update it MUST persist `revision = stored + 1`. The
    /// returned Relation carries the assigned revision — callers must not set
    /// it themselves.
    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation>;
}

/// Scope query for check / assemble — knowledge entries and timeline events.
#[async_trait]
pub trait ScopeQueryPort {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>>;
    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
}

/// Finding persistence.
#[async_trait]
pub trait FindingPort {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>>;
}

/// Rule query by reference list.
#[async_trait]
pub trait RuleQueryPort {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>>;
}

/// Host collaboration metadata — self manifest and product-known peer manifests.
///
/// Integrators call explicitly; orchestrators do not auto-fetch manifests.
#[async_trait]
pub trait HostManifestPort {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest>;
    async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>>;
}

/// Optional l2-computable session — project / compute.
#[async_trait]
pub trait ComputablePort {
    async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse>;
    async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse>;
}

/// Optional l5-fork timeline query.
///
/// Capability-specific refinement of [`ScopeQueryPort`]; one type MAY satisfy both.
#[async_trait]
pub trait ForkTimelineQueryPort {
    async fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
}

/// Ports required for spoke-baseline orchestration.
pub trait BaselinePorts:
    KnowledgeEntryPort
    + RelationPort
    + ScopeQueryPort
    + FindingPort
    + RuleQueryPort
    + HostManifestPort
{
}

impl<T> BaselinePorts for T where
    T: KnowledgeEntryPort
        + RelationPort
        + ScopeQueryPort
        + FindingPort
        + RuleQueryPort
        + HostManifestPort
{
}

/// Baseline plus optional computable capability.
///
/// Types that implement [`ComputablePort`] get a blanket availability probe.
/// Baseline-only types MAY implement this trait returning `None` for dynamically
/// assembled boundaries (parity with TS `requirePortMethod`).
pub trait ComputablePorts: BaselinePorts {
    fn as_computable(&self) -> Option<&dyn ComputablePort>;
}

impl<T> ComputablePorts for T
where
    T: BaselinePorts + ComputablePort,
{
    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        Some(self)
    }
}

/// Baseline plus optional fork timeline capability.
pub trait ForkPorts: BaselinePorts {
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort>;
}

impl<T> ForkPorts for T
where
    T: BaselinePorts + ForkTimelineQueryPort,
{
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        Some(self)
    }
}

/// Full composition of baseline, computable, and fork ports.
pub trait FullPorts: ComputablePorts + ForkPorts {}

impl<T> FullPorts for T where T: ComputablePorts + ForkPorts {}

/// Ergonomic marker for baseline adapter composition.
pub trait BaselineAdapter: BaselinePorts {}

impl<T: BaselinePorts> BaselineAdapter for T {}

/// Ergonomic marker for baseline plus computable adapter composition.
pub trait ComputableAdapter: ComputablePorts {}

impl<T: ComputablePorts> ComputableAdapter for T {}

/// Ergonomic marker for baseline plus fork adapter composition.
pub trait ForkAdapter: ForkPorts {}

impl<T: ForkPorts> ForkAdapter for T {}

/// Ergonomic marker for full adapter composition.
pub trait FullAdapter: FullPorts {}

impl<T: FullPorts> FullAdapter for T {}

/// Optional `ke-extraction` family — the injected product boundary for
/// referenced-content extraction.
///
/// Standalone like [`ToolInvokePort`](crate::ToolInvokePort): `ExtractionPort`
/// is NOT part of [`BaselinePorts`] / [`FullPorts`] or the adapter aliases. The
/// library contains no source I/O and no extraction engine; it only awaits the
/// injected port and callback, then validates and assembles.
#[async_trait]
pub trait ExtractionPort {
    /// Injected product source loader. The in-process value it returns may
    /// carry source content; it is not a wire object and never appears on
    /// `ExtractRequest` / `ExtractResponse`.
    async fn load_extraction_input(&self, request: &ExtractRequest) -> SpokeResult<Value>;
}

/// Extraction callback input — the request plus the loaded product input.
#[derive(Debug, Clone)]
pub struct ExtractRunInput {
    pub request: ExtractRequest,
    pub input: Value,
}

/// Product extraction output — candidate entries plus advisory run metadata.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub candidates: Vec<KnowledgeEntry>,
    /// Optional non-empty open string naming the product's extraction method.
    pub method: Option<String>,
    /// Optional advisory opaque coverage hint. `None` and
    /// [`serde_json::Value::Null`] both mean "no hint"; any other JSON value is
    /// retained verbatim.
    pub coverage_hint: Option<Value>,
}

/// Optional `spoke-memory` family — the injected product boundary for the
/// memory-system operations (§7 of the memory protocol).
///
/// Standalone like [`ExtractionPort`]: `MemoryPort` is NOT part of
/// [`BaselinePorts`] / [`FullPorts`] or the adapter aliases. One method per op
/// (33); every method takes the authenticated [`Principal`] as an explicit
/// admission-context parameter — it is never payload-asserted (MEM-R003). The
/// library contains no memory storage or enforcement; providers implement the
/// port and own storage, transactions, and every enforcement duty.
///
/// Common contract (§7.0): mutating ops require `idempotency_key` except
/// [`MemoryPort::ensure_subject`]; dedup namespace is `(space_id,
/// requester_principal_id, op, idempotency_key)`; list/recall ops paginate by
/// `cursor` + `limit`. Grant capabilities below are the minimum per op — an
/// `admin` grant with sufficient coverage also satisfies capability
/// requirements (§6.5).
#[async_trait]
pub trait MemoryPort {
    /// §7.1 `memory.describe` — provider self-description. **Grant:** none;
    /// authenticated principal only. Read-only.
    async fn describe(
        &self,
        principal: &Principal,
        request: MemoryDescribeRequest,
    ) -> SpokeResult<MemoryDescribeResponse>;

    /// §7.2 `memory.spaces.list` — visible spaces; also the drain-barrier read.
    /// **Grant:** any valid grant in the space, or provider-configured
    /// visibility. Read-only.
    async fn list_spaces(
        &self,
        principal: &Principal,
        request: MemorySpacesListRequest,
    ) -> SpokeResult<MemorySpacesListResponse>;

    /// §7.3 `memory.spaces.freeze` — freeze a space (§8.2 maintenance
    /// allowlist applies). **Grant:** `admin`. Idempotency key required.
    async fn freeze_space(
        &self,
        principal: &Principal,
        request: MemorySpacesFreezeRequest,
    ) -> SpokeResult<MemorySpacesFreezeResponse>;

    /// §7.4 `memory.spaces.unfreeze` — reverse of [`MemoryPort::freeze_space`];
    /// always allowed on a frozen space. **Grant:** `admin`.
    async fn unfreeze_space(
        &self,
        principal: &Principal,
        request: MemorySpacesUnfreezeRequest,
    ) -> SpokeResult<MemorySpacesUnfreezeResponse>;

    /// §7.5 `memory.subjects.ensure` — idempotent get-or-create; sole
    /// `idempotency_key` exemption (§7.0.5). **Grant:** `capture` (an `admin`
    /// grant with coverage also satisfies this, §6.5).
    async fn ensure_subject(
        &self,
        principal: &Principal,
        request: MemorySubjectsEnsureRequest,
    ) -> SpokeResult<MemorySubjectsEnsureResponse>;

    /// §7.6 `memory.subjects.get`. **Grant:** any capability on the subject.
    /// Read-only.
    async fn get_subject(
        &self,
        principal: &Principal,
        request: MemorySubjectsGetRequest,
    ) -> SpokeResult<MemorySubjectsGetResponse>;

    /// §7.7 `memory.subjects.fence` — subject/context lifecycle fencing.
    /// **Grant:** `admin`.
    async fn fence_subject(
        &self,
        principal: &Principal,
        request: MemorySubjectsFenceRequest,
    ) -> SpokeResult<MemorySubjectsFenceResponse>;

    /// §7.8 `memory.grants.list`. **Grant:** `admin` for full/filtered views;
    /// self-view limited to exactly the caller's own grants. Read-only.
    async fn list_grants(
        &self,
        principal: &Principal,
        request: MemoryGrantsListRequest,
    ) -> SpokeResult<MemoryGrantsListResponse>;

    /// §7.9 `memory.grants.grant`. **Grant:** `admin`; the issued grant must be
    /// non-amplifying relative to the granter's coverage (§6.5).
    async fn grant(
        &self,
        principal: &Principal,
        request: MemoryGrantsGrantRequest,
    ) -> SpokeResult<MemoryGrantsGrantResponse>;

    /// §7.10 `memory.grants.revoke` — allowed during freeze (§8.2 allowlist).
    /// **Grant:** `admin`.
    async fn revoke_grant(
        &self,
        principal: &Principal,
        request: MemoryGrantsRevokeRequest,
    ) -> SpokeResult<MemoryGrantsRevokeResponse>;

    /// §7.11 `memory.capture` — raw capture; produces zero or more pending
    /// candidates, never retained records. **Grant:** `capture` covering the
    /// declared `capture_scope`.
    async fn capture(
        &self,
        principal: &Principal,
        request: MemoryCaptureRequest,
    ) -> SpokeResult<MemoryCaptureResponse>;

    /// §7.12 `memory.candidates.list`. **Grant:** `review` covering
    /// `capture_scope` (context-restricted grants see only covered candidates).
    /// Read-only.
    async fn list_candidates(
        &self,
        principal: &Principal,
        request: MemoryCandidatesListRequest,
    ) -> SpokeResult<MemoryCandidatesListResponse>;

    /// §7.13 `memory.candidates.get`. **Grant:** `review` covering the
    /// candidate's `capture_scope`. Read-only.
    async fn get_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesGetRequest,
    ) -> SpokeResult<MemoryCandidatesGetResponse>;

    /// §7.14 `memory.candidates.dismiss` — consumption without a review
    /// decision. **Grant:** `review` covering the candidate's `capture_scope`.
    async fn dismiss_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesDismissRequest,
    ) -> SpokeResult<MemoryCandidatesDismissResponse>;

    /// §7.15 `memory.candidates.review` — the only candidate → record path.
    /// **Grant:** `review` + `records_write`, both covering `capture_scope`.
    async fn review_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesReviewRequest,
    ) -> SpokeResult<MemoryCandidatesReviewResponse>;

    /// §7.16 `memory.records.create` — direct authored write. **Grant:**
    /// `records_write` covering the declared placement.
    async fn create_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsCreateRequest,
    ) -> SpokeResult<MemoryRecordsCreateResponse>;

    /// §7.17 `memory.records.get`. **Grant:** `records_read` or `recall`
    /// covering placement; tombstoned reads require `admin`. Read-only.
    async fn get_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsGetRequest,
    ) -> SpokeResult<MemoryRecordsGetResponse>;

    /// §7.18 `memory.records.update` — placement changes are NOT updates (see
    /// [`MemoryPort::promote_record`]). **Grant:** `records_write` covering the
    /// current placement.
    async fn update_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsUpdateRequest,
    ) -> SpokeResult<MemoryRecordsUpdateResponse>;

    /// §7.19 `memory.records.delete` — tombstone retained, receipts unaffected.
    /// **Grant:** `records_write` covering the placement.
    async fn delete_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsDeleteRequest,
    ) -> SpokeResult<MemoryRecordsDeleteResponse>;

    /// §7.20 `memory.records.list`. **Grant:** `records_read` or `recall`
    /// covering the selection/filter; `include_tombstoned` requires `admin`.
    /// Read-only.
    async fn list_records(
        &self,
        principal: &Principal,
        request: MemoryRecordsListRequest,
    ) -> SpokeResult<MemoryRecordsListResponse>;

    /// §7.21 `memory.records.promote` — explicit placement change. **Grant:**
    /// `promote` + `records_write`, both covering source and target placements.
    async fn promote_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsPromoteRequest,
    ) -> SpokeResult<MemoryRecordsPromoteResponse>;

    /// §7.22 `memory.recall` — authorized, ranked, budgeted read. **Grant:**
    /// `recall` covering the selection/filter. Read-only.
    async fn recall(
        &self,
        principal: &Principal,
        request: MemoryRecallRequest,
    ) -> SpokeResult<MemoryRecallResponse>;

    /// §7.23 `memory.profile.get` — absent kinds are honest-empty (§4.2).
    /// **Grant:** any capability on the subject with shared access or
    /// unrestricted coverage. Read-only.
    async fn get_profile(
        &self,
        principal: &Principal,
        request: MemoryProfileGetRequest,
    ) -> SpokeResult<MemoryProfileGetResponse>;

    /// §7.24 `memory.profile.update`. **Grant:** `profile_write`.
    async fn update_profile(
        &self,
        principal: &Principal,
        request: MemoryProfileUpdateRequest,
    ) -> SpokeResult<MemoryProfileUpdateResponse>;

    /// §7.25 `memory.reflections.state` — capability `spoke-memory-reflection`.
    /// **Grant:** `reflect` covering the selection/filter. Read-only.
    async fn get_reflection_state(
        &self,
        principal: &Principal,
        request: MemoryReflectionsStateRequest,
    ) -> SpokeResult<MemoryReflectionsStateResponse>;

    /// §7.26 `memory.reflections.list` — capability `spoke-memory-reflection`.
    /// **Grant:** `reflect` covering the selection/filter. Read-only.
    async fn list_reflections(
        &self,
        principal: &Principal,
        request: MemoryReflectionsListRequest,
    ) -> SpokeResult<MemoryReflectionsListResponse>;

    /// §7.27 `memory.reflections.generate` — explicit cost-bearing generation;
    /// may complete asynchronously (§7.0.3). Capability
    /// `spoke-memory-reflection`. **Grant:** `reflect` covering the
    /// selection/filter.
    async fn generate_reflection(
        &self,
        principal: &Principal,
        request: MemoryReflectionsGenerateRequest,
    ) -> SpokeResult<MemoryReflectionsGenerateResponse>;

    /// §7.28 `memory.receipts.get` — sole uncertainty-resolution and pending
    /// polling channel. **Grant:** current authorization as the receipt's
    /// `requester_principal_id`, or `admin` in the space. Read-only.
    async fn get_receipt(
        &self,
        principal: &Principal,
        request: MemoryReceiptsGetRequest,
    ) -> SpokeResult<MemoryReceiptsGetResponse>;

    /// §7.29 `memory.exports.create` — capability `spoke-memory-transfer`;
    /// may complete asynchronously; never fails `SPACE_FROZEN` (§8.2
    /// allowlist). **Grant:** `transfer` covering the export scope;
    /// `include_tombstoned` requires `admin`.
    async fn create_export(
        &self,
        principal: &Principal,
        request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse>;

    /// §7.30 `memory.exports.get` — capability `spoke-memory-transfer`.
    /// **Grant:** `transfer` covering the export scope; `include_tombstoned`
    /// requires `admin`. Read-only.
    async fn get_export(
        &self,
        principal: &Principal,
        request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse>;

    /// §7.31 `memory.imports.stage` — per-chunk staging, strict ordinal order.
    /// Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin` on
    /// the target space.
    async fn stage_import(
        &self,
        principal: &Principal,
        request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse>;

    /// §7.32 `memory.imports.activate` — atomic visibility flip. Capability
    /// `spoke-memory-transfer`. **Grant:** `transfer` + `admin`.
    async fn activate_import(
        &self,
        principal: &Principal,
        request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse>;

    /// §7.33 `memory.imports.discard` — pre-activation rollback. Capability
    /// `spoke-memory-transfer`. **Grant:** `transfer` + `admin`.
    async fn discard_import(
        &self,
        principal: &Principal,
        request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse>;
}

/// Availability probe for the standalone `spoke-memory` family, mirroring
/// [`ComputablePorts`]: types that implement [`MemoryPort`] get the blanket
/// `Some(self)` probe, while dynamically assembled boundaries MAY implement
/// this trait with the default `None` (parity with TS `requirePortMethod`).
pub trait MemoryPorts {
    fn as_memory(&self) -> Option<&dyn MemoryPort> {
        None
    }
}

impl<T> MemoryPorts for T
where
    T: MemoryPort,
{
    fn as_memory(&self) -> Option<&dyn MemoryPort> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{spoke_ok, SpokeRejectCode};
    use serde_json::json;
    use std::collections::HashMap;

    fn make_manifest(
        host_id: &str,
        namespaces: &[&str],
        roles: &[&str],
    ) -> HostCapabilityManifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": host_id,
            "roles": roles,
            "capabilities": ["spoke-baseline"],
            "namespaces": namespaces,
            "extensions": {}
        }))
        .expect("valid HostCapabilityManifest")
    }

    fn normalize_peer_manifests(
        self_host_id: &str,
        peers: &[HostCapabilityManifest],
    ) -> Vec<HostCapabilityManifest> {
        let mut by_host_id: HashMap<String, HostCapabilityManifest> = HashMap::new();
        for peer in peers {
            if peer.host_id.as_str() == self_host_id {
                continue;
            }
            by_host_id.insert(peer.host_id.as_str().to_string(), peer.clone());
        }
        let mut normalized: Vec<HostCapabilityManifest> = by_host_id.into_values().collect();
        normalized.sort_by(|left, right| left.host_id.as_str().cmp(right.host_id.as_str()));
        normalized
    }

    struct HostManifestPortMock {
        self_manifest: HostCapabilityManifest,
        raw_peers: Vec<HostCapabilityManifest>,
    }

    impl HostManifestPortMock {
        fn new(
            self_manifest: HostCapabilityManifest,
            raw_peers: Vec<HostCapabilityManifest>,
        ) -> Self {
            Self {
                self_manifest,
                raw_peers,
            }
        }
    }

    #[async_trait]
    impl HostManifestPort for HostManifestPortMock {
        async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
            spoke_ok(self.self_manifest.clone())
        }

        async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
            spoke_ok(normalize_peer_manifests(
                self.self_manifest.host_id.as_str(),
                &self.raw_peers,
            ))
        }
    }

    struct BaselinePortStub {
        host_manifest: HostManifestPortMock,
    }

    impl BaselinePortStub {
        fn new(host_manifest: HostManifestPortMock) -> Self {
            Self { host_manifest }
        }
    }

    #[async_trait]
    impl KnowledgeEntryPort for BaselinePortStub {
        async fn get_knowledge_entry(&self, _entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            unreachable!("baseline port stub")
        }

        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            _expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            spoke_ok(entry)
        }
    }

    #[async_trait]
    impl RelationPort for BaselinePortStub {
        async fn get_relation(&self, _relation_id: &str) -> SpokeResult<Relation> {
            unreachable!("baseline port stub")
        }

        async fn put_relation(
            &self,
            relation: Relation,
            _expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            spoke_ok(relation)
        }
    }

    #[async_trait]
    impl ScopeQueryPort for BaselinePortStub {
        async fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            spoke_ok(Vec::new())
        }

        async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            spoke_ok(Vec::new())
        }
    }

    #[async_trait]
    impl FindingPort for BaselinePortStub {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            spoke_ok(findings)
        }
    }

    #[async_trait]
    impl RuleQueryPort for BaselinePortStub {
        async fn list_rules(&self, _rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            spoke_ok(Vec::new())
        }
    }

    #[async_trait]
    impl HostManifestPort for BaselinePortStub {
        async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
            self.host_manifest.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
            self.host_manifest
                .list_peer_host_capability_manifests()
                .await
        }
    }

    #[test]
    fn capability_port_missing_is_twentieth_spoke_reject_code() {
        assert_eq!(
            SpokeRejectCode::CapabilityPortMissing.as_str(),
            "CAPABILITY_PORT_MISSING"
        );
        assert_eq!(
            SpokeRejectCode::try_from_str("CAPABILITY_PORT_MISSING"),
            Some(SpokeRejectCode::CapabilityPortMissing)
        );
    }

    #[test]
    fn baseline_ports_accepts_all_six_baseline_families() {
        let ports = BaselinePortStub::new(HostManifestPortMock::new(
            make_manifest("self-host", &["self-ns"], &["data-store"]),
            Vec::new(),
        ));

        let _: &dyn BaselinePorts = &ports;
        let _: &dyn KnowledgeEntryPort = &ports;
        let _: &dyn RelationPort = &ports;
        let _: &dyn ScopeQueryPort = &ports;
        let _: &dyn FindingPort = &ports;
        let _: &dyn RuleQueryPort = &ports;
        let _: &dyn HostManifestPort = &ports;
    }

    #[test]
    fn host_manifest_port_returns_self_manifest() {
        let self_manifest = make_manifest(
            "adapter-self",
            &["alpha"],
            &["data-store", "checker"],
        );
        let ports = HostManifestPortMock::new(self_manifest.clone(), Vec::new());

        let result = pollster::block_on(ports.get_host_capability_manifest());

        match result {
            SpokeResult::Ok(value) => assert_eq!(value.host_id.as_str(), "adapter-self"),
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }

    #[test]
    fn host_manifest_port_accepts_empty_peer_list() {
        let ports = HostManifestPortMock::new(
            make_manifest("self-host", &["default"], &["data-store"]),
            Vec::new(),
        );

        let result = pollster::block_on(ports.list_peer_host_capability_manifests());

        match result {
            SpokeResult::Ok(value) => assert!(value.is_empty()),
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }

    #[test]
    fn host_manifest_port_returns_seeded_peers_with_disjoint_namespaces() {
        let self_manifest = make_manifest("self-host", &["self-ns"], &["data-store"]);
        let peer_a = make_manifest("peer-a", &["peer-a-ns"], &["checker"]);
        let peer_b = make_manifest("peer-b", &["peer-b-ns"], &["assembler"]);
        let ports = HostManifestPortMock::new(self_manifest, vec![peer_b.clone(), peer_a.clone()]);

        let result = pollster::block_on(ports.list_peer_host_capability_manifests());

        match result {
            SpokeResult::Ok(value) => {
                assert_eq!(value.len(), 2);
                assert_eq!(value[0].host_id.as_str(), "peer-a");
                assert_eq!(value[1].host_id.as_str(), "peer-b");
                let namespaces: Vec<&str> = value
                    .iter()
                    .flat_map(|manifest| manifest.namespaces.iter().map(|ns| ns.as_str()))
                    .collect();
                let unique: std::collections::HashSet<_> = namespaces.iter().copied().collect();
                assert_eq!(namespaces.len(), unique.len());
                assert!(!namespaces.contains(&"self-ns"));
            }
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }

    #[test]
    fn host_manifest_port_excludes_self_dedupes_and_sorts_peers() {
        let self_manifest = make_manifest("self-host", &["self-ns"], &["data-store"]);
        let peer_z = make_manifest("peer-z", &["z-ns"], &["data-store"]);
        let peer_a_dupe = make_manifest("peer-a", &["a-ns-dup"], &["checker"]);
        let peer_a = make_manifest("peer-a", &["a-ns"], &["assembler"]);
        let ports = HostManifestPortMock::new(
            self_manifest.clone(),
            vec![peer_z.clone(), self_manifest, peer_a_dupe, peer_a.clone()],
        );

        let result = pollster::block_on(ports.list_peer_host_capability_manifests());

        match result {
            SpokeResult::Ok(value) => {
                let host_ids: Vec<&str> = value.iter().map(|m| m.host_id.as_str()).collect();
                assert_eq!(host_ids, vec!["peer-a", "peer-z"]);
                assert_eq!(value[0].roles, peer_a.roles);
            }
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }
}
