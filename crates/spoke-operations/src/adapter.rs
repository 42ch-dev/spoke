//! Capability-sliced adapter ports and injection orchestration.

pub mod orchestrate;
pub mod ports;

pub use orchestrate::{
    orchestrate_assemble, orchestrate_check, orchestrate_compute, orchestrate_fork_assemble,
    orchestrate_fork_check, orchestrate_project, orchestrate_promote, orchestrate_relate,
    orchestrate_upsert, CheckRunInput,
};
pub use ports::{
    BaselinePorts, ComputablePort, ComputablePorts, FindingPort, ForkPorts, ForkTimelineQueryPort,
    FullPorts, KnowledgeEntryPort, RelationPort, RuleQueryPort, ScopeQueryPort,
};
