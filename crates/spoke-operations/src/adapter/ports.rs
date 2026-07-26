//! Capability-sliced adapter port contracts for injection orchestration.
//! Synchronous `SpokeResult` surface — adapters own async I/O behind this boundary.

use crate::result::SpokeResult;
use spoke_schemas::{
    ComputeRequest, ComputeResponse, Finding, KnowledgeEntry, ProjectRequest, ProjectResponse,
    Relation, Rule, Scope, TimelineEvent,
};

/// Knowledge entry persistence — get / put by entry id.
pub trait KnowledgeEntryPort {
    fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry>;
    /// Persist a KnowledgeEntry with optimistic concurrency control.
    ///
    /// Adapters MUST treat `expected_base_revision` as the store’s required current
    /// revision before accepting the write (conditional put / OCC / CAS).
    /// `None` means the entry must be absent (create). A non-null value means the
    /// store’s current revision for `entry.entry_id` MUST equal
    /// `expected_base_revision`; otherwise reject with `STORED_REVISION_STALE` or
    /// `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
    /// in the adapter; the library stays I/O-free.
    fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry>;
}

/// Relation persistence.
pub trait RelationPort {
    fn put_relation(&self, relation: Relation) -> SpokeResult<Relation>;
}

/// Scope query for check / assemble — knowledge entries and timeline events.
pub trait ScopeQueryPort {
    fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>>;
    fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
}

/// Finding persistence.
pub trait FindingPort {
    fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>>;
}

/// Rule query by reference list.
pub trait RuleQueryPort {
    fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>>;
}

/// Optional l2-computable session — project / compute.
pub trait ComputablePort {
    fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse>;
    fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse>;
}

/// Optional l5-fork timeline query.
///
/// Capability-specific refinement of [`ScopeQueryPort`]; one type MAY satisfy both.
pub trait ForkTimelineQueryPort {
    fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
}

/// Ports required for spoke-baseline orchestration.
pub trait BaselinePorts:
    KnowledgeEntryPort + RelationPort + ScopeQueryPort + FindingPort + RuleQueryPort
{
}

impl<T> BaselinePorts for T where
    T: KnowledgeEntryPort + RelationPort + ScopeQueryPort + FindingPort + RuleQueryPort
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
