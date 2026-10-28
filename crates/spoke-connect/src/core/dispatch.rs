//! Op dispatch gate: required capability ⊆ negotiated capabilities.
//!
//! Normative rule (`.mstar/specs/spoke-connect.md` §Op dispatch gate,
//! MUST): before executing or forwarding a `ConnectInvokeRequest`, a host
//! that performs op dispatch MUST ensure the capability required by `op` is
//! present in the session's `negotiated_capabilities`. If absent, the host
//! MUST NOT run the op handler and MUST answer with an error branch
//! (e.g. `op_unsupported` / `capability_missing`) — no side effects.
//!
//! The gate is evaluated against `negotiated_capabilities`, not the remote
//! manifest alone and not unsigned hello `extensions`.

/// Baseline capability: required by the baseline core ops.
pub const CAPABILITY_SPOKE_BASELINE: &str = "spoke-baseline";
/// Optional capability: required by the compute-family core ops.
pub const CAPABILITY_L2_COMPUTABLE: &str = "l2-computable";
/// Optional capability: required by the `extract` core op. The serving host
/// owns the extraction port, the loaded input value and the extractor; the
/// requester only needs the negotiated flag.
pub const CAPABILITY_KE_EXTRACTION: &str = "ke-extraction";
/// Optional capability: required by a Scope-bearing remote request whose
/// declared `scope.viewpoint` carries a reader context. It is a capability,
/// **not** an op — the payload-dependent requirement lives at the remote
/// dispatch boundary, so this table has no `ke-ownership` op row.
pub const CAPABILITY_KE_OWNERSHIP: &str = "ke-ownership";
/// Optional capability: required by the memory family core ops (SPOKE
/// memory protocol §4.1). A provider declaring either memory sub-flag MUST
/// also declare this one.
pub const CAPABILITY_SPOKE_MEMORY: &str = "spoke-memory";
/// Optional memory sub-flag: required by `memory.reflections.*`.
pub const CAPABILITY_SPOKE_MEMORY_REFLECTION: &str = "spoke-memory-reflection";
/// Optional memory sub-flag: required by `memory.exports.*` and
/// `memory.imports.*`.
pub const CAPABILITY_SPOKE_MEMORY_TRANSFER: &str = "spoke-memory-transfer";

/// The minimum capability required to dispatch `op`, per the protocol v1
/// core-op table. Product-defined `op` values return `None` — their required
/// capability is documented by the product and configured outside the core
/// table.
///
/// `tools.<ns>.<tool_id>` is a **core gate rule** (normative
/// `spoke-connect.md` §Op dispatch gate): self-describing tools require the
/// op string itself — no registry, no umbrella flag. The gate then
/// evaluates that exact string against `negotiated_capabilities` as for
/// every op.
///
/// The output lifetime is tied to `op` (`Option<&str>`); static rows coerce.
#[must_use]
pub fn required_capability(op: &str) -> Option<&str> {
    if op.starts_with("tools.") {
        return Some(op);
    }
    match op {
        "upsert" | "promote" | "relate" | "check" | "assemble" => Some(CAPABILITY_SPOKE_BASELINE),
        "project" | "compute" => Some(CAPABILITY_L2_COMPUTABLE),
        "extract" => Some(CAPABILITY_KE_EXTRACTION),
        // Memory family (SPOKE memory protocol §4.1/§13): 33 rows, each
        // mapping its op to the flag that gates it. Sub-flag rows stay
        // literal — negotiating a sub-flag never authorizes core ops.
        "memory.describe"
        | "memory.spaces.list"
        | "memory.spaces.freeze"
        | "memory.spaces.unfreeze"
        | "memory.subjects.ensure"
        | "memory.subjects.get"
        | "memory.subjects.fence"
        | "memory.grants.list"
        | "memory.grants.grant"
        | "memory.grants.revoke"
        | "memory.capture"
        | "memory.candidates.list"
        | "memory.candidates.get"
        | "memory.candidates.dismiss"
        | "memory.candidates.review"
        | "memory.records.create"
        | "memory.records.get"
        | "memory.records.update"
        | "memory.records.delete"
        | "memory.records.list"
        | "memory.records.promote"
        | "memory.recall"
        | "memory.profile.get"
        | "memory.profile.update"
        | "memory.receipts.get" => Some(CAPABILITY_SPOKE_MEMORY),
        "memory.reflections.state"
        | "memory.reflections.list"
        | "memory.reflections.generate" => Some(CAPABILITY_SPOKE_MEMORY_REFLECTION),
        "memory.exports.create"
        | "memory.exports.get"
        | "memory.imports.stage"
        | "memory.imports.activate"
        | "memory.imports.discard" => Some(CAPABILITY_SPOKE_MEMORY_TRANSFER),
        _ => None,
    }
}

/// Whether `op` may be dispatched in a session with `negotiated_capabilities`.
///
/// Fails closed: an unknown `op` has no core-table requirement and is not
/// authorized by this gate (hosts answer `op_unsupported`).
#[must_use]
pub fn dispatch_allowed(op: &str, negotiated_capabilities: &[String]) -> bool {
    match required_capability(op) {
        Some(required) => negotiated_capabilities.iter().any(|c| c == required),
        None => false,
    }
}

/// Whether a capability-token grant (validated `claims.capabilities`)
/// authorizes `op` — **membership** of `op`'s required capability in the
/// grant (normative §Capability matching: subset-of-grant, not exact-list
/// equality; extra capabilities on the token are ignored when unused).
///
/// `required` is the op's required capability from the core table or the
/// product-configured map. Fails closed: an unknown op (no requirement)
/// is not authorized by the token gate.
#[must_use]
pub fn token_authorizes_op(required: Option<&str>, token_capabilities: &[String]) -> bool {
    required.is_some_and(|r| token_capabilities.iter().any(|c| c == r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn baseline_ops_require_spoke_baseline() {
        for op in ["upsert", "promote", "relate", "check", "assemble"] {
            assert_eq!(required_capability(op), Some(CAPABILITY_SPOKE_BASELINE));
            assert!(dispatch_allowed(op, &caps(&["spoke-baseline"])));
            assert!(dispatch_allowed(
                op,
                &caps(&["spoke-connect", "spoke-baseline"])
            ));
            assert!(
                !dispatch_allowed(op, &caps(&["l2-computable"])),
                "l2-computable alone must not authorize baseline ops"
            );
            assert!(
                !dispatch_allowed(op, &caps(&[])),
                "empty negotiated set must not dispatch"
            );
        }
    }

    #[test]
    fn computable_ops_require_l2_computable() {
        for op in ["project", "compute"] {
            assert_eq!(required_capability(op), Some(CAPABILITY_L2_COMPUTABLE));
            assert!(dispatch_allowed(op, &caps(&["l2-computable"])));
            assert!(dispatch_allowed(
                op,
                &caps(&["spoke-baseline", "l2-computable"])
            ));
            assert!(
                !dispatch_allowed(op, &caps(&["spoke-baseline"])),
                "baseline alone must not authorize compute ops"
            );
        }
    }

    #[test]
    fn ke_remote_extract_requires_ke_extraction() {
        assert_eq!(
            required_capability("extract"),
            Some(CAPABILITY_KE_EXTRACTION)
        );
        assert!(dispatch_allowed("extract", &caps(&["ke-extraction"])));
        assert!(dispatch_allowed(
            "extract",
            &caps(&["spoke-baseline", "ke-extraction"])
        ));
        // Neither the baseline nor the ownership capability authorizes the
        // extract op on its own.
        assert!(!dispatch_allowed("extract", &caps(&["spoke-baseline"])));
        assert!(!dispatch_allowed("extract", &caps(&[CAPABILITY_KE_OWNERSHIP])));
        assert!(!dispatch_allowed("extract", &caps(&[])));
    }

    #[test]
    fn ke_remote_extract_token_grant_membership() {
        assert!(token_authorizes_op(
            required_capability("extract"),
            &caps(&["ke-extraction"])
        ));
        // Subset-of-grant: extra granted capabilities are ignored.
        assert!(token_authorizes_op(
            required_capability("extract"),
            &caps(&["ke-extraction", "ke-ownership", "unused-extra"])
        ));
        assert!(!token_authorizes_op(
            required_capability("extract"),
            &caps(&["spoke-baseline", "l2-computable"])
        ));
        assert!(!token_authorizes_op(required_capability("extract"), &caps(&[])));
    }

    #[test]
    fn unknown_ops_fail_closed() {
        assert_eq!(required_capability("custom-op"), None);
        assert!(!dispatch_allowed("custom-op", &caps(&["spoke-baseline"])));
        assert!(!dispatch_allowed("custom-op", &caps(&["spoke-connect"])));
        assert!(!dispatch_allowed("", &caps(&["spoke-baseline"])));
    }

    #[test]
    fn token_grant_authorizes_by_membership() {
        // Membership / subset-of-grant: the required capability present in
        // the grant authorizes the op; extra capabilities are ignored.
        assert!(token_authorizes_op(
            Some(CAPABILITY_SPOKE_BASELINE),
            &caps(&["spoke-baseline", "l2-computable", "unused-extra"])
        ));
        assert!(token_authorizes_op(
            Some(CAPABILITY_L2_COMPUTABLE),
            &caps(&["l2-computable"])
        ));
        // The grant is NOT an exact-list match: a subset suffices.
        assert!(token_authorizes_op(
            Some(CAPABILITY_SPOKE_BASELINE),
            &caps(&["spoke-baseline", "l2-computable"])
        ));
    }

    #[test]
    fn token_grant_missing_requirement_denies() {
        assert!(!token_authorizes_op(
            Some(CAPABILITY_L2_COMPUTABLE),
            &caps(&["spoke-baseline"])
        ));
        assert!(!token_authorizes_op(
            Some(CAPABILITY_SPOKE_BASELINE),
            &caps(&[])
        ));
        assert!(!token_authorizes_op(
            Some(CAPABILITY_SPOKE_BASELINE),
            &caps(&["l2-computable"])
        ));
    }

    #[test]
    fn token_grant_unknown_ops_fail_closed() {
        // An op with no required capability (core table or product map) is
        // never authorized by the token gate.
        assert!(!token_authorizes_op(None, &caps(&["spoke-baseline"])));
        assert!(!token_authorizes_op(None, &caps(&[])));
    }

    // `tools.<ns>.<tool_id>` prefix rule — parity golden vector with the TS
    // `dispatch-parity.test.ts` "tools.* prefix rule" block (frozen §3):
    // self-describing tools require the op string itself; the gate then
    // evaluates that exact string against `negotiated_capabilities` as for
    // every op.

    #[test]
    fn tools_prefix_rule_returns_the_op_string_itself() {
        let tool_op = "tools.math.add";
        assert_eq!(required_capability(tool_op), Some(tool_op));
        assert_eq!(
            required_capability("tools.any.namespaced.thing"),
            Some("tools.any.namespaced.thing")
        );
    }

    #[test]
    fn tools_prefix_rule_authorizes_only_when_the_exact_capability_is_negotiated() {
        let tool_op = "tools.math.add";
        // Authorized: the tool capability string itself is negotiated.
        assert!(dispatch_allowed(tool_op, &caps(&[tool_op])));
        // Not negotiated / wrong capability: denied (the self-describing
        // tool gate never consults an umbrella flag or the baseline
        // capability).
        assert!(!dispatch_allowed(tool_op, &caps(&[CAPABILITY_SPOKE_BASELINE])));
        assert!(!dispatch_allowed(tool_op, &caps(&[CAPABILITY_L2_COMPUTABLE])));
        assert!(!dispatch_allowed(tool_op, &caps(&[])));
    }

    #[test]
    fn tools_prefix_rule_token_grant_requires_the_exact_capability() {
        let tool_op = "tools.math.add";
        // Membership of the exact capability string in the grant.
        assert!(token_authorizes_op(
            required_capability(tool_op),
            &caps(&[tool_op])
        ));
        assert!(!token_authorizes_op(
            required_capability(tool_op),
            &caps(&[CAPABILITY_SPOKE_BASELINE])
        ));
        assert!(!token_authorizes_op(
            required_capability(tool_op),
            &caps(&[])
        ));
    }

    // Memory family (SPOKE memory protocol §4.1/§13): three flags gate
    // disjoint op groups; the gate evaluates negotiated capabilities only.
    // Parity twin: TS `tests/dispatch.test.ts` "memory family" block.

    const MEMORY_CORE_OPS: [&str; 25] = [
        "memory.describe",
        "memory.spaces.list",
        "memory.spaces.freeze",
        "memory.spaces.unfreeze",
        "memory.subjects.ensure",
        "memory.subjects.get",
        "memory.subjects.fence",
        "memory.grants.list",
        "memory.grants.grant",
        "memory.grants.revoke",
        "memory.capture",
        "memory.candidates.list",
        "memory.candidates.get",
        "memory.candidates.dismiss",
        "memory.candidates.review",
        "memory.records.create",
        "memory.records.get",
        "memory.records.update",
        "memory.records.delete",
        "memory.records.list",
        "memory.records.promote",
        "memory.recall",
        "memory.profile.get",
        "memory.profile.update",
        "memory.receipts.get",
    ];
    const MEMORY_REFLECTION_OPS: [&str; 3] = [
        "memory.reflections.state",
        "memory.reflections.list",
        "memory.reflections.generate",
    ];
    const MEMORY_TRANSFER_OPS: [&str; 5] = [
        "memory.exports.create",
        "memory.exports.get",
        "memory.imports.stage",
        "memory.imports.activate",
        "memory.imports.discard",
    ];

    #[test]
    fn memory_family_table_covers_all_33_protocol_ops() {
        assert_eq!(
            MEMORY_CORE_OPS.len() + MEMORY_REFLECTION_OPS.len() + MEMORY_TRANSFER_OPS.len(),
            33,
            "protocol §7 defines 33 memory ops"
        );
        for op in MEMORY_CORE_OPS {
            assert_eq!(required_capability(op), Some(CAPABILITY_SPOKE_MEMORY), "{op}");
            assert!(dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY])), "{op}");
        }
        for op in MEMORY_REFLECTION_OPS {
            assert_eq!(
                required_capability(op),
                Some(CAPABILITY_SPOKE_MEMORY_REFLECTION),
                "{op}"
            );
            assert!(
                dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION])),
                "{op}"
            );
        }
        for op in MEMORY_TRANSFER_OPS {
            assert_eq!(
                required_capability(op),
                Some(CAPABILITY_SPOKE_MEMORY_TRANSFER),
                "{op}"
            );
            assert!(
                dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY_TRANSFER])),
                "{op}"
            );
        }
    }

    #[test]
    fn unnegotiated_memory_op_denies_while_unknown_memory_op_stays_unsupported() {
        // `memory.capture` has a row, so the boundary answers
        // `capability_missing` (requirement known, capability not negotiated).
        assert_eq!(
            required_capability("memory.capture"),
            Some(CAPABILITY_SPOKE_MEMORY)
        );
        assert!(!dispatch_allowed("memory.capture", &caps(&[])));
        assert!(!dispatch_allowed(
            "memory.capture",
            &caps(&["spoke-baseline", "l2-computable", "ke-extraction"])
        ));
        // No row ⇒ no requirement ⇒ the host answers `op_unsupported`.
        assert_eq!(required_capability("memory.archive.purge"), None);
        assert!(!dispatch_allowed(
            "memory.archive.purge",
            &caps(&[
                CAPABILITY_SPOKE_MEMORY,
                CAPABILITY_SPOKE_MEMORY_REFLECTION,
                CAPABILITY_SPOKE_MEMORY_TRANSFER
            ])
        ));
    }

    #[test]
    fn memory_flags_never_cross_authorize_other_families() {
        for op in MEMORY_CORE_OPS {
            assert!(!dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION])), "{op}");
            assert!(!dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY_TRANSFER])), "{op}");
        }
        for op in MEMORY_REFLECTION_OPS {
            assert!(!dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY])), "{op}");
            assert!(!dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY_TRANSFER])), "{op}");
        }
        for op in MEMORY_TRANSFER_OPS {
            assert!(!dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY])), "{op}");
            assert!(!dispatch_allowed(op, &caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION])), "{op}");
        }
        // Existing families gain no memory authority either.
        assert!(!dispatch_allowed(
            "memory.recall",
            &caps(&[
                CAPABILITY_SPOKE_BASELINE,
                CAPABILITY_L2_COMPUTABLE,
                CAPABILITY_KE_EXTRACTION,
                CAPABILITY_KE_OWNERSHIP
            ])
        ));
    }

    #[test]
    fn memory_sub_flag_without_core_leaves_core_ops_unnegotiated() {
        // Protocol §4.1: a provider declaring a sub-flag MUST also declare
        // `spoke-memory`. The gate evaluates negotiated capabilities, so a
        // session that (invalidly) negotiated only the sub-flag admits the
        // sub-family but still fails core ops with `capability_missing`.
        let sub_only = caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION]);
        assert!(dispatch_allowed("memory.reflections.list", &sub_only));
        assert_eq!(
            required_capability("memory.capture"),
            Some(CAPABILITY_SPOKE_MEMORY)
        );
        assert!(!dispatch_allowed("memory.capture", &sub_only));
    }

    #[test]
    fn memory_fully_negotiated_session_dispatches_the_whole_family() {
        let all = caps(&[
            CAPABILITY_SPOKE_MEMORY,
            CAPABILITY_SPOKE_MEMORY_REFLECTION,
            CAPABILITY_SPOKE_MEMORY_TRANSFER,
        ]);
        for op in MEMORY_CORE_OPS {
            assert!(dispatch_allowed(op, &all), "{op}");
            assert!(token_authorizes_op(required_capability(op), &all), "{op}");
        }
        for op in MEMORY_REFLECTION_OPS.iter().chain(&MEMORY_TRANSFER_OPS) {
            assert!(dispatch_allowed(op, &all), "{op}");
            assert!(token_authorizes_op(required_capability(op), &all), "{op}");
        }
    }
}
