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
#[must_use]
pub fn required_capability(op: &str) -> Option<&'static str> {
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
}
