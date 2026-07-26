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

/// Parity export checklist — mirrors TS `adapter/ports.test.ts` "adapter port exports"
/// and locks the plan OQ-4 TS ↔ Rust symbol table (plus `CheckRunInput` / reject code).
#[cfg(test)]
mod parity_export_checklist {
    use crate::SpokeRejectCode;

    /// Normative TS ↔ Rust adapter symbols that must remain flat-exported from `lib.rs`.
    const TS_RUST_ADAPTER_PARITY: &[(&str, &str)] = &[
        ("KnowledgeEntryPort", "KnowledgeEntryPort"),
        ("RelationPort", "RelationPort"),
        ("ScopeQueryPort", "ScopeQueryPort"),
        ("FindingPort", "FindingPort"),
        ("RuleQueryPort", "RuleQueryPort"),
        ("ComputablePort", "ComputablePort"),
        ("ForkTimelineQueryPort", "ForkTimelineQueryPort"),
        ("BaselinePorts", "BaselinePorts"),
        ("ComputablePorts", "ComputablePorts"),
        ("ForkPorts", "ForkPorts"),
        ("FullPorts", "FullPorts"),
        ("CheckRunInput", "CheckRunInput"),
        ("orchestrateUpsert", "orchestrate_upsert"),
        ("orchestratePromote", "orchestrate_promote"),
        ("orchestrateRelate", "orchestrate_relate"),
        ("orchestrateCheck", "orchestrate_check"),
        ("orchestrateAssemble", "orchestrate_assemble"),
        ("orchestrateProject", "orchestrate_project"),
        ("orchestrateCompute", "orchestrate_compute"),
        ("orchestrateForkCheck", "orchestrate_fork_check"),
        ("orchestrateForkAssemble", "orchestrate_fork_assemble"),
        ("CAPABILITY_PORT_MISSING", "CAPABILITY_PORT_MISSING"),
    ];

    // Crate-root path resolution (parallel to TS `adapter/ports.test.ts` export smoke).
    // Missing `lib.rs` flat re-exports fail compile here.
    use crate::{
        orchestrate_assemble, orchestrate_check, orchestrate_compute, orchestrate_fork_assemble,
        orchestrate_fork_check, orchestrate_project, orchestrate_promote, orchestrate_relate,
        orchestrate_upsert, BaselinePorts, CheckRunInput, ComputablePort, ComputablePorts,
        FindingPort, ForkPorts, ForkTimelineQueryPort, FullPorts, KnowledgeEntryPort, RelationPort,
        RuleQueryPort, ScopeQueryPort,
    };

    /// Minimal probe type so generic orchestrator paths monomorphize in the checklist.
    struct ExportProbePorts;

    impl KnowledgeEntryPort for ExportProbePorts {
        fn get_knowledge_entry(
            &self,
            _entry_id: &str,
        ) -> crate::SpokeResult<spoke_schemas::KnowledgeEntry> {
            unreachable!("parity checklist probe")
        }
        fn put_knowledge_entry(
            &self,
            entry: spoke_schemas::KnowledgeEntry,
        ) -> crate::SpokeResult<spoke_schemas::KnowledgeEntry> {
            crate::spoke_ok(entry)
        }
    }
    impl RelationPort for ExportProbePorts {
        fn put_relation(
            &self,
            relation: spoke_schemas::Relation,
        ) -> crate::SpokeResult<spoke_schemas::Relation> {
            crate::spoke_ok(relation)
        }
    }
    impl ScopeQueryPort for ExportProbePorts {
        fn list_knowledge_entries(
            &self,
            _scope: &spoke_schemas::Scope,
        ) -> crate::SpokeResult<Vec<spoke_schemas::KnowledgeEntry>> {
            crate::spoke_ok(Vec::new())
        }
        fn list_timeline_events(
            &self,
            _scope: &spoke_schemas::Scope,
        ) -> crate::SpokeResult<Vec<spoke_schemas::TimelineEvent>> {
            crate::spoke_ok(Vec::new())
        }
    }
    impl FindingPort for ExportProbePorts {
        fn put_findings(
            &self,
            findings: Vec<spoke_schemas::Finding>,
        ) -> crate::SpokeResult<Vec<spoke_schemas::Finding>> {
            crate::spoke_ok(findings)
        }
    }
    impl RuleQueryPort for ExportProbePorts {
        fn list_rules(
            &self,
            _rule_refs: &[String],
        ) -> crate::SpokeResult<Vec<spoke_schemas::Rule>> {
            crate::spoke_ok(Vec::new())
        }
    }
    impl ForkTimelineQueryPort for ExportProbePorts {
        fn list_fork_timeline_events(
            &self,
            _scope: &spoke_schemas::Scope,
        ) -> crate::SpokeResult<Vec<spoke_schemas::TimelineEvent>> {
            crate::spoke_ok(Vec::new())
        }
    }
    impl ComputablePort for ExportProbePorts {
        fn project(
            &self,
            request: spoke_schemas::ProjectRequest,
        ) -> crate::SpokeResult<spoke_schemas::ProjectResponse> {
            serde_json::from_value(serde_json::json!({
                "session_id": request.session_id,
                "entry_id": request.entry_id,
                "computable": request.state,
            }))
            .map_or_else(
                |error| {
                    crate::spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        format!("parity probe project response: {error}"),
                        None,
                    )
                },
                crate::spoke_ok,
            )
        }
        fn compute(
            &self,
            request: spoke_schemas::ComputeRequest,
        ) -> crate::SpokeResult<spoke_schemas::ComputeResponse> {
            serde_json::from_value(serde_json::json!({
                "session_id": request.session_id,
                "entry_id": request.entry_id,
                "computable": request.computable,
            }))
            .map_or_else(
                |error| {
                    crate::spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        format!("parity probe compute response: {error}"),
                        None,
                    )
                },
                crate::spoke_ok,
            )
        }
    }

    fn wire<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("parity checklist fixture")
    }

    /// Compile-time / runtime checklist: every parity twin is reachable via crate root re-exports.
    /// Dropping a `lib.rs` re-export fails to compile; table drift fails the length/string asserts.
    #[test]
    fn public_exports_cover_ts_adapter_parity_table() {
        // Port family + composition traits (object-safe method surfaces).
        let _: Option<&dyn KnowledgeEntryPort> = None;
        let _: Option<&dyn RelationPort> = None;
        let _: Option<&dyn ScopeQueryPort> = None;
        let _: Option<&dyn FindingPort> = None;
        let _: Option<&dyn RuleQueryPort> = None;
        let _: Option<&dyn ComputablePort> = None;
        let _: Option<&dyn ForkTimelineQueryPort> = None;
        let _: Option<&dyn BaselinePorts> = None;
        let _: Option<&dyn ComputablePorts> = None;
        let _: Option<&dyn ForkPorts> = None;
        let _: Option<&dyn FullPorts> = None;
        let _: Option<CheckRunInput> = None;

        // `impl Trait` orchestrators cannot be stored as bare fn items; type-check call paths
        // inside a dead branch so missing crate-root exports fail compile without executing I/O.
        if false {
            let ports = ExportProbePorts;
            let _ = orchestrate_upsert(
                &ports,
                wire(serde_json::json!({ "knowledge_entries": [] })),
            );
            let _ = orchestrate_promote(
                &ports,
                wire(serde_json::json!({
                    "candidate": {
                        "schema_version": 1,
                        "entry_id": "kb_parity",
                        "entry_type": "character",
                        "canonical_name": "Parity",
                        "status": "provisional",
                        "body": {},
                        "extensions": {}
                    }
                })),
            );
            let _ = orchestrate_relate(
                &ports,
                wire(serde_json::json!({
                    "relation": {
                        "schema_version": 1,
                        "relation_id": "rel_parity",
                        "relation_type": "related_to",
                        "from_id": "a",
                        "to_id": "b",
                        "extensions": {}
                    }
                })),
            );
            let _ = orchestrate_check(
                &ports,
                wire(serde_json::json!({ "scope": { "scope_id": "parity" } })),
                |input: CheckRunInput| {
                    let _ = input;
                    crate::spoke_ok(Vec::new())
                },
            );
            let _ = orchestrate_assemble(
                &ports,
                wire(serde_json::json!({ "scope": { "scope_id": "parity" } })),
            );
            let _ = orchestrate_project(
                &ports,
                wire(serde_json::json!({
                    "session_id": "sess",
                    "entry_id": "kb",
                    "state": {}
                })),
            );
            let _ = orchestrate_compute(
                &ports,
                wire(serde_json::json!({
                    "session_id": "sess",
                    "entry_id": "kb",
                    "computable": {}
                })),
            );
            let _ = orchestrate_fork_check(
                &ports,
                wire(serde_json::json!({
                    "scope": { "scope_id": "parity", "fork_id": "fork_a" }
                })),
                |input: CheckRunInput| {
                    let _ = input;
                    crate::spoke_ok(Vec::new())
                },
            );
            let _ = orchestrate_fork_assemble(
                &ports,
                wire(serde_json::json!({
                    "scope": { "scope_id": "parity", "fork_id": "fork_a" }
                })),
            );
        }

        assert_eq!(
            SpokeRejectCode::CapabilityPortMissing.as_str(),
            "CAPABILITY_PORT_MISSING"
        );
        assert_eq!(
            SpokeRejectCode::try_from_str("CAPABILITY_PORT_MISSING"),
            Some(SpokeRejectCode::CapabilityPortMissing)
        );

        assert_eq!(TS_RUST_ADAPTER_PARITY.len(), 22);
        for (ts, rust) in TS_RUST_ADAPTER_PARITY {
            assert!(!ts.is_empty(), "TS symbol must be non-empty");
            assert!(!rust.is_empty(), "Rust symbol must be non-empty");
        }

        // Named twins from the plan OQ-4 table (identity or snake_case).
        assert_eq!(
            TS_RUST_ADAPTER_PARITY[0],
            ("KnowledgeEntryPort", "KnowledgeEntryPort")
        );
        assert_eq!(
            TS_RUST_ADAPTER_PARITY[12],
            ("orchestrateUpsert", "orchestrate_upsert")
        );
        assert_eq!(
            TS_RUST_ADAPTER_PARITY[20],
            ("orchestrateForkAssemble", "orchestrate_fork_assemble")
        );
        assert_eq!(
            TS_RUST_ADAPTER_PARITY[21],
            ("CAPABILITY_PORT_MISSING", "CAPABILITY_PORT_MISSING")
        );

        assert_eq!(stringify!(orchestrate_upsert), "orchestrate_upsert");
        assert_eq!(stringify!(CheckRunInput), "CheckRunInput");
        assert_eq!(stringify!(KnowledgeEntryPort), "KnowledgeEntryPort");
    }
}
