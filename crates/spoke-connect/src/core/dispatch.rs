//! Op dispatch gate: required capability ⊆ negotiated capabilities.
//!
//! Normative rule (`.agents/specs/connect/spoke-connect.md` §Op dispatch gate,
//! MUST): before executing or forwarding a `ConnectInvokeRequest`, a host
//! that performs op dispatch MUST ensure the capability required by `op` is
//! present in the session's `negotiated_capabilities`. If absent, the host
//! MUST NOT run the op handler and MUST answer with an error branch
//! (e.g. `op_unsupported` / `capability_missing`) — no side effects.
//!
//! The gate is evaluated against `negotiated_capabilities`, not the remote
//! manifest alone and not unsigned hello `extensions`.

use serde_json::Value;

/// Baseline capability: required by the baseline core ops.
pub const CAPABILITY_SPOKE_BASELINE: &str = "spoke-baseline";
/// Optional capability: required by the three remote knowledge ops.
/// It must be negotiated explicitly: the responder serves those ops only
/// through its knowledge face, so no other flag implies this one.
pub const CAPABILITY_SPOKE_KNOWLEDGE: &str = "spoke-knowledge";
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
/// Optional capability: the `spoke-cognition` family (one flag, no
/// subflags). Not an op requirement — no core-table row names it. A request
/// that *declares* a cognition requirement (see
/// [`cognition_requirement_present`]) additionally needs this flag in the
/// negotiated set; the payload-dependent conjunct lives at the dispatch
/// boundary next to `ke-ownership`. The legacy `l5-mind` flag is never read
/// as this family.
pub const CAPABILITY_SPOKE_COGNITION: &str = "spoke-cognition";
/// Optional capability: required by the history family core ops
/// (`history.revisions.get` / `history.revisions.prune`, spoke-history
/// contract §Connect operations and capability gate). Neither
/// `spoke-baseline` nor `spoke-knowledge` satisfies it — dispatch requires
/// exact `spoke-history` membership.
pub const CAPABILITY_SPOKE_HISTORY: &str = "spoke-history";
/// Optional capability: the `spoke-judgment` data/conformance family. A host
/// that validates or produces `Judgment` advertises this in
/// `HostCapabilityManifest.capabilities` (spoke-judgment contract §Manifest).
/// Manifest-only: it is never an op requirement — this table has no
/// `spoke-judgment` row, and `tools.<ns>.<tool_id>` ops carrying `Judgment`
/// payload are gated on their exact op string, not on this flag.
pub const CAPABILITY_SPOKE_JUDGMENT: &str = "spoke-judgment";
/// Optional capability: required by `knowledge.adopt` (spoke-knowledge-
/// adoption contract §Connect operation and capability gate). The op also
/// requires the mandatory co-capability `spoke-knowledge` — a host
/// declaring `spoke-knowledge-adoption` also declares `spoke-knowledge`.
pub const CAPABILITY_SPOKE_KNOWLEDGE_ADOPTION: &str = "spoke-knowledge-adoption";
/// Optional capability: required by a `port.knowledge.get` / `put` request
/// that declares a `scope` (A2-D1). It is a conjunct, never a replacement
/// for the op's `spoke-knowledge` requirement, and marks a peer that honours
/// the member instead of ignoring it. A session without it must answer
/// `op_unsupported` for such a request (no silent scope drop).
pub const CAPABILITY_SPOKE_KNOWLEDGE_SCOPED: &str = "spoke-knowledge-scoped";

/// Ops whose registered request carries a `Scope` and therefore the
/// `Scope.cognition` selection slot (`common.schema.json` → `Scope`):
/// `assemble` / `check`, the three `port.*` Scope ops, `port.knowledge.get` /
/// `put` (optional scope, A2), and
/// `history.revisions.get` / `history.revisions.prune`.
const SCOPE_COGNITION_OPS: [&str; 9] = [
    "check",
    "assemble",
    "port.scope.list_knowledge_entries",
    "port.scope.list_timeline_events",
    "port.fork.list_timeline_events",
    "port.knowledge.get",
    "port.knowledge.put",
    "history.revisions.get",
    "history.revisions.prune",
];

/// Memory ops whose registered request declares the `cognition_filter` slot
/// (`memory-recall-request`, `memory-records-list-request`,
/// `memory-reflections-{generate,list,state}-request`).
const COGNITION_FILTER_OPS: [&str; 5] = [
    "memory.recall",
    "memory.records.list",
    "memory.reflections.generate",
    "memory.reflections.list",
    "memory.reflections.state",
];

/// The request field path that declares a cognition requirement **for this
/// op**, or `None` when the request carries none. Both slots come from the
/// registered request schemas: only the actual standard carrier of the
/// dispatched operation can demand cognition, so collapsed/inspected data on
/// any other op — a tool's `arguments`, unrelated product metadata, an opaque
/// `extensions` member — stays ordinary payload data and never becomes a
/// synthetic requirement.
///
/// Declared-location predicate only: no trimming, normalization, registry
/// lookup or recursive search. A present JSON `null` counts as present
/// (malformed, never "absent").
#[must_use]
pub fn cognition_requirement_field(op: &str, payload: &Value) -> Option<&'static str> {
    if SCOPE_COGNITION_OPS.contains(&op)
        && payload
            .get("scope")
            .and_then(Value::as_object)
            .is_some_and(|scope| scope.contains_key("cognition"))
    {
        return Some("scope.cognition");
    }
    if COGNITION_FILTER_OPS.contains(&op) && payload.get("cognition_filter").is_some() {
        return Some("cognition_filter");
    }
    None
}

/// Whether `op`'s payload declares a `spoke-cognition` requirement. The
/// dispatch boundary then requires [`CAPABILITY_SPOKE_COGNITION`] in the
/// negotiated set; ordinary payloads without the op's carrier members are
/// untouched.
#[must_use]
pub fn cognition_requirement_present(op: &str, payload: &Value) -> bool {
    cognition_requirement_field(op, payload).is_some()
}

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
        // Memory family (SPOKE memory protocol §4.1/§13): 34 rows, each
        // mapping its op to the flag that gates it. Sub-flag rows stay
        // literal — negotiating a sub-flag never authorizes core ops.
        "memory.describe"
        | "memory.spaces.list"
        | "memory.spaces.create"
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
        // History family (spoke-history contract §Connect operations): two
        // rows, exact `spoke-history` membership — no other flag implies it.
        "history.revisions.get" | "history.revisions.prune" => {
            Some(CAPABILITY_SPOKE_HISTORY)
        }
        // Adoption family (spoke-knowledge-adoption contract §Connect
        // operation): one row — the mandatory co-capability
        // `spoke-knowledge` is asserted by `dispatch_allowed`, never by
        // this single-row return.
        "knowledge.adopt" => Some(CAPABILITY_SPOKE_KNOWLEDGE_ADOPTION),
        "memory.exports.create"
        | "memory.exports.get"
        | "memory.imports.stage"
        | "memory.imports.activate"
        | "memory.imports.discard" => Some(CAPABILITY_SPOKE_MEMORY_TRANSFER),
        _ => None,
    }
}

/// Whether `negotiated` satisfies `required`: plain membership. No capability
/// implies another — in particular `spoke-baseline` never satisfies
/// `spoke-knowledge`, because the knowledge ops are served only through the
/// knowledge face.
#[must_use]
pub fn capability_satisfies(negotiated: &[String], required: &str) -> bool {
    negotiated.iter().any(|c| c == required)
}

/// Whether `op` falls under a core-reserved prefix with no registered
/// row. `knowledge.*` is a reserved-product prefix owned by this core
/// decision (spoke-knowledge-adoption contract §Connect operation):
/// `knowledge.adopt` is its single registered operation — an op that
/// starts with `knowledge.` and has no core-table row is an unknown
/// reserved name, refused `op_unsupported` by every dispatch surface
/// BEFORE the product custom-map (`ConnectConfig.op_capability_requirements`
/// and equivalents) is consulted. Custom mappings apply only outside
/// reserved families.
#[must_use]
pub fn reserved_prefix_op(op: &str) -> bool {
    required_capability(op).is_none() && op.starts_with("knowledge.")
}

/// Whether `op` may be dispatched in a session with `negotiated_capabilities`.
///
/// Fails closed: an unknown `op` has no core-table requirement and is not
/// authorized by this gate (hosts answer `op_unsupported`).
#[must_use]
pub fn dispatch_allowed(op: &str, negotiated_capabilities: &[String]) -> bool {
    let Some(required) = required_capability(op) else {
        return false;
    };
    if !capability_satisfies(negotiated_capabilities, required) {
        return false;
    }
    // Compound gate (spoke-knowledge-adoption contract): `knowledge.adopt`
    // additionally requires the mandatory co-capability `spoke-knowledge`
    // — neither flag alone satisfies the gate.
    if required == CAPABILITY_SPOKE_KNOWLEDGE_ADOPTION
        && !capability_satisfies(negotiated_capabilities, CAPABILITY_SPOKE_KNOWLEDGE)
    {
        return false;
    }
    true
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
    use serde_json::json;

    fn caps(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// `knowledge.*` is a core-reserved prefix: the one registered op is
    /// not reserved-unknown, every unregistered name is — regardless of
    /// any product custom-map row — and names outside the prefix are
    /// never classified reserved (F-011: the distinction lives in core,
    /// not in each consumer).
    #[test]
    fn knowledge_prefix_reservation_is_core_owned() {
        assert!(!reserved_prefix_op("knowledge.adopt"));
        assert!(reserved_prefix_op("knowledge.archive"));
        assert!(reserved_prefix_op("knowledge."));
        assert!(!reserved_prefix_op("memory.archive.purge"));
        assert!(!reserved_prefix_op("knowledgeless"));
        assert!(!reserved_prefix_op("port.knowledge.get"));
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
    fn capability_satisfies_is_plain_membership() {
        assert!(capability_satisfies(
            &caps(&["spoke-knowledge"]),
            CAPABILITY_SPOKE_KNOWLEDGE
        ));
        assert!(capability_satisfies(
            &caps(&["spoke-baseline", "spoke-knowledge"]),
            CAPABILITY_SPOKE_KNOWLEDGE
        ));
        assert!(
            !capability_satisfies(&caps(&["spoke-baseline"]), CAPABILITY_SPOKE_KNOWLEDGE),
            "spoke-baseline must not satisfy spoke-knowledge: the knowledge ops \
             are served only through the knowledge face"
        );
        assert!(!capability_satisfies(
            &caps(&["l2-computable"]),
            CAPABILITY_SPOKE_KNOWLEDGE
        ));
        assert!(!capability_satisfies(&caps(&[]), CAPABILITY_SPOKE_KNOWLEDGE));
        assert!(!capability_satisfies(
            &caps(&["spoke-knowledge"]),
            CAPABILITY_SPOKE_BASELINE
        ));
        // No implication: neither family authorizes the other's core ops.
        for op in ["upsert", "promote", "relate", "check", "assemble"] {
            assert!(
                !dispatch_allowed(op, &caps(&["spoke-knowledge"])),
                "spoke-knowledge alone must not authorize {op}"
            );
        }
        // Fails closed on an op outside the core table (`port.*`), as before.
        assert!(!dispatch_allowed("port.knowledge.get", &caps(&["spoke-knowledge"])));
        assert!(!dispatch_allowed("product.op.unknown", &caps(&["spoke-baseline"])));
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
    fn knowledge_get_and_put_carry_the_scope_cognition_slot() {
        // The marker is a released wire name; renaming it is a negotiation
        // break.
        assert_eq!(CAPABILITY_SPOKE_KNOWLEDGE_SCOPED, "spoke-knowledge-scoped");
        for op in ["port.knowledge.get", "port.knowledge.put"] {
            assert_eq!(
                cognition_requirement_field(op, &json!({ "scope": { "cognition": {} } })),
                Some("scope.cognition"),
                "{op}: a declared cognition selection must demand the family"
            );
            assert!(
                !cognition_requirement_present(op, &json!({ "scope": { "scope_id": "s" } })),
                "{op}: a scope without the selection declares no requirement"
            );
        }
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

    const MEMORY_CORE_OPS: [&str; 26] = [
        "memory.describe",
        "memory.spaces.list",
        "memory.spaces.create",
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
    fn memory_family_table_maps_every_protocol_op_to_its_gating_flag() {
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
        // `memory.capture` has a row, so the responder boundary answers
        // `capability_missing` (requirement known, capability not negotiated).
        // The wire code is not asserted here — this gate only knows the
        // requirement; `tests/memory_loopback.rs` covers the served deny.
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
    fn memory_token_grants_never_cross_authorize_family_rows() {
        // The token gate is a second, independent boundary next to the
        // negotiated set: a grant carrying only a sibling sub-flag never
        // authorizes a family row. Parity twin: TS
        // `tests/core/dispatch-parity.test.ts` memory block.
        for op in MEMORY_CORE_OPS {
            assert!(token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY])), "{op}");
            assert!(!token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION])), "{op}");
            assert!(!token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY_TRANSFER])), "{op}");
        }
        for op in MEMORY_REFLECTION_OPS {
            assert!(token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION])), "{op}");
            assert!(!token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY])), "{op}");
            assert!(!token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY_TRANSFER])), "{op}");
        }
        for op in MEMORY_TRANSFER_OPS {
            assert!(token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY_TRANSFER])), "{op}");
            assert!(!token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY])), "{op}");
            assert!(!token_authorizes_op(required_capability(op), &caps(&[CAPABILITY_SPOKE_MEMORY_REFLECTION])), "{op}");
        }
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

    #[test]
    fn cognition_requirement_field_names_the_op_declared_wire_location() {
        // Parity with the TS `cognitionRequirementField` golden vectors: the
        // slot comes from the registered request schema of the dispatched op.
        assert_eq!(
            cognition_requirement_field(
                "check",
                &json!({"scope": {"scope_id": "s", "cognition": {"view": "current"}}})
            ),
            Some("scope.cognition")
        );
        assert_eq!(
            cognition_requirement_field(
                "port.scope.list_knowledge_entries",
                &json!({"scope": {"scope_id": "s", "cognition": {"view": "current"}}})
            ),
            Some("scope.cognition")
        );
        assert_eq!(
            cognition_requirement_field(
                "memory.records.list",
                &json!({"cognition_filter": {"state": {"eq": "known"}}})
            ),
            Some("cognition_filter")
        );
        // Present JSON null is a requirement (malformed), never "absent".
        assert_eq!(
            cognition_requirement_field("memory.recall", &json!({"cognition_filter": null})),
            Some("cognition_filter")
        );
        // The slots are disjoint per op, so an unrelated member can never
        // stand in front of the op's real requirement.
        assert_eq!(
            cognition_requirement_field(
                "memory.records.list",
                &json!({"cognition": {}, "cognition_filter": {"q_min": 0.5}})
            ),
            Some("cognition_filter")
        );
    }

    #[test]
    fn ordinary_payloads_declare_no_cognition_requirement() {
        assert!(!cognition_requirement_present("upsert", &json!({})));
        assert!(!cognition_requirement_present(
            "port.scope.list_knowledge_entries",
            &json!({"scope": {"scope_id": "s", "viewpoint": "kb_mira"}})
        ));
        assert!(!cognition_requirement_present("port.finding.put", &json!({"findings": []})));
        // A non-object `scope` cannot carry the declared member.
        assert!(!cognition_requirement_present(
            "check",
            &json!({"scope": "s"})
        ));
        assert!(!cognition_requirement_present("check", &json!({"scope": null})));
        // Only the ACTUAL carrier of the dispatched op demands cognition:
        // tool arguments, a Memory `cognition_filter` on a Scope op, an
        // opaque `cognition` on any other op are ordinary payload data.
        assert!(!cognition_requirement_present(
            "tools.math.add",
            &json!({"arguments": {}, "cognition": "tool metadata"})
        ));
        assert!(!cognition_requirement_present(
            "port.scope.list_knowledge_entries",
            &json!({"scope": {"scope_id": "s"}, "cognition_filter": {"state": {"eq": "known"}}})
        ));
        assert!(!cognition_requirement_present(
            "upsert",
            &json!({"cognition": {"view": "recorded_at"}})
        ));
        assert!(!cognition_requirement_present(
            "memory.records.list",
            &json!({"scope": {"scope_id": "s", "cognition": {"view": "current"}}})
        ));
        // A Memory content write's `content.cognition` is written data, not a
        // request requirement; its placement `scope` is unrelated payload too.
        assert!(!cognition_requirement_present(
            "memory.records.create",
            &json!({
                "content": {"cognition": {"records": []}},
                "scope": {"space_id": "s", "subject_id": "sub"},
            })
        ));
        // Present JSON null IS a requirement (malformed, never "absent").
        assert!(cognition_requirement_present(
            "check",
            &json!({"scope": {"scope_id": "s", "cognition": null}})
        ));
    }

    #[test]
    fn legacy_l5_mind_never_satisfies_the_cognition_family() {
        // The family is granted only by its own flag: `l5-mind` is an
        // unrelated legacy flag and the pure intersection never maps it.
        let legacy = caps(&[CAPABILITY_SPOKE_BASELINE, "l5-mind"]);
        assert!(!cognition_requirement_present("check", &json!({})));
        assert!(!legacy.iter().any(|cap| cap == CAPABILITY_SPOKE_COGNITION));
        // No op row names the family either: the requirement is payload-only.
        for op in [
            "upsert", "check", "assemble", "extract", "port.scope.list_knowledge_entries",
            "memory.records.list", "memory.recall",
        ] {
            assert_ne!(required_capability(op), Some(CAPABILITY_SPOKE_COGNITION), "{op}");
        }
    }
}
