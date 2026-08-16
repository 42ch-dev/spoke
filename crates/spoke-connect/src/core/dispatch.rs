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
}
